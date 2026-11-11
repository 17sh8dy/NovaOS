"""Headless boot test: boots Nova in QEMU and checks it reaches the kernel.

  python tools/boot_test.py            # after tools/build.ps1

Passes when the serial log shows the bootloader handing off and the kernel
printing its ready marker. Also saves a screenshot of the framebuffer to
build/boot_test.png and the full serial log to build/serial.log.
"""

import json
import re
import socket
import struct
import subprocess
import sys
import time
import zlib

sys.path.insert(0, str(__import__("pathlib").Path(__file__).resolve().parent))
import qemu as q  # noqa: E402

TIMEOUT_S = 120
MUST_SEE = [
    "NovaBoot",                       # bootloader started (firmware console)
    "Exiting UEFI boot services",     # bootloader got to the handoff
    "NovaBoot: firmware exited",      # ExitBootServices succeeded
    "Nova kernel",                    # kernel's first line
    "memory map:",                    # kernel read the BootInfo
    "breakpoint self-test passed",    # stage 2: own IDT works
    "sched: self-test passed",        # stage 7: threads, preemption, SMP
    "timer: tick test passed",        # stage 5: APIC timer + interrupts work
    "pmm: self-test passed",          # stage 3: frame allocator works on real memory
    "NOVA KERNEL READY",              # kernel finished
]
FAILURES = ["NOVA KERNEL PANIC", "NovaBoot panic", "cannot start Nova"]
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def read_log(path) -> str:
    try:
        raw = path.read_bytes().decode("utf-8", "replace")
    except FileNotFoundError:
        return ""
    return ANSI.sub("", raw).replace("\r", "")


class Qmp:
    def __init__(self, port: int, proc: subprocess.Popen):
        deadline = time.time() + 15
        while True:
            try:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=10)
                break
            except OSError:
                if proc.poll() is not None:
                    raise SystemExit(f"FAIL: QEMU exited during startup (code {proc.returncode:#x})")
                if time.time() > deadline:
                    raise
                time.sleep(0.2)
        self.file = self.sock.makefile("rwb")
        self._read()  # greeting
        self.cmd("qmp_capabilities")

    def _read(self):
        while True:
            msg = json.loads(self.file.readline())
            if "event" not in msg:
                return msg

    def cmd(self, name, **args):
        self.file.write(json.dumps({"execute": name, "arguments": args}).encode() + b"\n")
        self.file.flush()
        reply = self._read()
        if "error" in reply:
            raise RuntimeError(f"QMP {name}: {reply['error']}")
        return reply.get("return")


def ppm_to_png(ppm: bytes) -> bytes:
    """Converts a binary PPM (P6) screendump to PNG with only the stdlib."""
    parts = []
    i = 0
    while len(parts) < 4:
        while ppm[i:i + 1].isspace():
            i += 1
        if ppm[i:i + 1] == b"#":
            while ppm[i:i + 1] != b"\n":
                i += 1
            continue
        j = i
        while not ppm[j:j + 1].isspace():
            j += 1
        parts.append(ppm[i:j])
        i = j
    i += 1
    if parts[0] != b"P6":
        raise ValueError("not a P6 PPM")
    w, h = int(parts[1]), int(parts[2])
    pixels = ppm[i:i + w * h * 3]
    rows = b"".join(b"\x00" + pixels[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b""))


def mouse_test(qmp, serial, start=(640, 400), move=(100, 50)) -> str | None:
    """Drives the guest's PS/2 mouse through QEMU's input layer and checks the
    kernel saw it. Returns None on success or a description of what failed."""

    def wait_for(text, seconds=10):
        end = time.time() + seconds
        while time.time() < end:
            if text in read_log(serial):
                return True
            time.sleep(0.1)
        return False

    x, y = start[0] + move[0], start[1] + move[1]
    qmp.cmd("input-send-event", events=[
        {"type": "rel", "data": {"axis": "x", "value": move[0]}},
        {"type": "rel", "data": {"axis": "y", "value": move[1]}},
    ])
    want = f"mouse: first movement, pointer at ({x}, {y})"
    if not wait_for(want):
        return f"expected {want!r} after moving the mouse"
    for btn in ("left", "right"):
        for down in (True, False):
            qmp.cmd("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": btn}}])
            want = f"mouse: {btn} button {'down' if down else 'up'} at ({x}, {y})"
            if not wait_for(want):
                return f"expected {want!r}"

    # The mouse above arrived by interrupt (the kernel does not poll), and the
    # CPU must be sleeping between interrupts: about one wake-up per 10 ms tick.
    if not wait_for("idle: up", 15):
        return "the kernel never reported its idle statistics"
    m = re.search(r"idle: up (\d+) s, (\d+) wakeups from hlt, (\d+) stray \+ (\d+) spurious", read_log(serial))
    secs, wakeups, stray = int(m.group(1)), int(m.group(2)), int(m.group(3))
    if not (secs * 60 <= wakeups <= secs * 160):
        return f"idle loop woke {wakeups} times in {secs} s (expected ~{secs * 100}): the CPU is not sleeping properly"
    if stray:
        return f"{stray} stray interrupts"
    print(f"Idle: {wakeups} hlt wake-ups in {secs} s (about one per timer tick), no stray interrupts")
    return None


QCODES = {" ": "spc", "\n": "ret", "-": "minus", ".": "dot", "/": "slash", "\b": "backspace"}


def press(qmp, qcode, shift=False):
    """One key press + release through QEMU's input layer (the guest sees real PS/2 scancodes)."""
    events = []
    if shift:
        events.append({"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}})
    events.append({"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": qcode}}})
    events.append({"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": qcode}}})
    if shift:
        events.append({"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}})
    qmp.cmd("input-send-event", events=events)
    time.sleep(0.04)


def type_text(qmp, text):
    for ch in text:
        if ch in QCODES:
            press(qmp, QCODES[ch])
        elif ch.isupper():
            press(qmp, ch.lower(), shift=True)
        else:
            press(qmp, ch)


def shell_test(qmp, serial, cpus) -> str | None:
    """Types into the guest's shell through the emulated keyboard and checks
    what the console printed (every console line is mirrored to the serial log)."""

    def wait_for(pattern, seconds=10):
        end = time.time() + seconds
        while time.time() < end:
            m = re.search(pattern, read_log(serial))
            if m:
                return m
            time.sleep(0.1)
        return None

    if not wait_for(r"shell: ready"):
        return "the shell never started"
    steps = [
        ("echo Hello Nova\n", r"con: Hello Nova"),
        ("echoo\b x\n", r"con: x\n"),                      # backspace edits the line
        ("uptime\n", r"con: up \d+\.\d\d s"),
        ("mem\n", r"con: heap: +\d+ KiB mapped"),
        ("cpus\n", rf"con: {cpus} CPU\(s\) online"),
        ("sleep 100\n", r"con: slept 100 ms"),
        ("nope\n", r"con: unknown command 'nope'"),
    ]
    for keys, expect in steps:
        type_text(qmp, keys)
        if not wait_for(expect):
            return f"after typing {keys!r} expected {expect!r}"
    # History: Up recalls the last command, Enter runs it again.
    before = len(re.findall(r"unknown command 'nope'", read_log(serial)))
    press(qmp, "up")
    press(qmp, "ret")
    end = time.time() + 10
    while time.time() < end and len(re.findall(r"unknown command 'nope'", read_log(serial))) == before:
        time.sleep(0.1)
    if len(re.findall(r"unknown command 'nope'", read_log(serial))) == before:
        return "Up + Enter did not re-run the previous command (history)"
    # Ctrl+C abandons a line.
    type_text(qmp, "abandoned")
    qmp.cmd("input-send-event", events=[
        {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "ctrl"}}},
        {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "c"}}},
        {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "c"}}},
        {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "ctrl"}}},
    ])
    time.sleep(0.3)
    type_text(qmp, "echo after cancel\n")
    if not wait_for(r"con: after cancel"):
        return "the shell did not recover after Ctrl+C"
    if "abandoned" in "".join(re.findall(r"con: (?:nova> )?.*", read_log(serial))).replace("^C", ""):
        return "a cancelled line was still run"
    return None


def user_test(qmp, serial) -> str | None:
    """Runs user-space programs from the shell: ring 3, system calls, the
    keyboard from user space, isolation, killing misbehaving processes, and a
    check that finished processes give all their memory back."""

    def wait_for(pattern, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            m = re.search(pattern, read_log(serial))
            if m:
                return m
            time.sleep(0.1)
        return None

    def run(command, expect, seconds=15):
        type_text(qmp, command + "\n")
        if not wait_for(expect, seconds):
            return f"after {command!r} expected {expect!r}"
        return None

    def free_frames():
        type_text(qmp, "mem\n")
        end = time.time() + 10
        while time.time() < end:
            ms = re.findall(r"\((\d+) frames free\)", read_log(serial))
            if ms and wait_for(r"con: heap:", 1):
                return int(ms[-1])
            time.sleep(0.1)
        return None

    if not wait_for(r"shell: ready"):
        return "the shell never started"
    before = free_frames()

    for command, expect in [
        ("progs", r"con:   hello +\d+ bytes"),
        ("run hello", r"con: Hello from user space! I am process \d+\."),
    ]:
        if (err := run(command, expect)):
            return err
    if not wait_for(r"\[process \d+ \(hello\) exited with code 0\]"):
        return "hello did not report a clean exit"

    # The keyboard, from user space.
    type_text(qmp, "run greet\n")
    if not wait_for(r"con: What is your name\?"):
        return "greet never asked for a name"
    type_text(qmp, "Nova\n")
    if not wait_for(r"con: Hello, Nova!"):
        return "greet did not read the typed name (read system call)"
    if not wait_for(r"\[process \d+ \(greet\) exited with code 0\]"):
        return "greet did not exit cleanly"

    # A crashing program is killed; the kernel and shell carry on.
    if (err := run("run crash", r"killed: #PF page fault at 0x[0-9a-f]+, address 0x0\]")):
        return err
    if (err := run("echo still alive", r"con: still alive")):
        return err

    # Hostile system calls are refused; a privileged instruction kills it.
    type_text(qmp, "run evil\n")
    for expect in [r"write from a kernel address returned -14", r"wrapping range returned -14",
                   r"unmapped address returned -14", r"unknown system call returned -38",
                   r"\(evil\) killed: #GP"]:
        if not wait_for(expect):
            return f"evil: expected {expect!r}"

    # Isolation + preemption: several memtests and counters at once.
    type_text(qmp, "spawn memtest memtest memtest counter counter\n")
    end = time.time() + 30
    while time.time() < end and len(re.findall(r"memtest ok", read_log(serial))) < 3:
        time.sleep(0.2)
    if len(re.findall(r"memtest ok", read_log(serial))) < 3:
        return "the three memtests did not all pass (private memory broken?)"
    if "overwritten by someone else" in read_log(serial):
        return "a process saw another process's memory"
    if not wait_for(r"counter 10[\s\S]*counter 10"):
        return "the two counters did not both finish"
    pids = re.findall(r"con: \[(\d+)\] counter (\d+)", read_log(serial))
    first_done = next((i for i, (p, n) in enumerate(pids) if n == "10"), None)
    if first_done is None or len({p for p, _ in pids[:first_done]}) < 2:
        return "the two counters did not interleave (no multitasking between processes)"
    if not wait_for(r"con: no user processes running", 1):
        # give them time to be reaped, then ask
        time.sleep(1.0)
    type_text(qmp, "ps\n")
    if not wait_for(r"con: no user processes running"):
        return "processes are still listed after they all finished"

    # Memory: ten more processes must not leak frames.
    mid = free_frames()
    for _ in range(10):
        type_text(qmp, "run hello\n")
        time.sleep(0.25)
    time.sleep(1.0)
    after = free_frames()
    if before is None or mid is None or after is None:
        return "could not read the free frame count"
    if mid - after > 40:
        return f"free frames went from {mid} to {after} over ten processes: a leak of ~{(mid - after) // 10} frames per process"
    print(f"Memory: {mid} frames free before ten more processes, {after} after (no leak)")
    return None


def main() -> int:
    import argparse
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--memory", default="256M", help="guest RAM, e.g. 256M, 2G, 6G")
    ap.add_argument("--expect", metavar="TEXT", action="append",
                    help="negative test: pass if ALL the given TEXTs appear in the log instead of a "
                         "full boot (repeatable)")
    ap.add_argument("--cpus", type=int, default=None, help="virtual CPUs (default 4, or NOVA_CPUS)")
    ap.add_argument("--shot", default="boot_test", help="screenshot name (build/<name>.png)")
    ap.add_argument("--user", action="store_true", help="after boot, run user-space programs from the shell")
    ap.add_argument("--shell", action="store_true", help="after boot, type commands into the shell through QEMU's keyboard")
    ap.add_argument("--mouse", action="store_true",
                    help="after boot, move the mouse and click through QEMU and check the kernel saw it")
    opts = ap.parse_args()
    qemu = q.find_qemu()
    serial = q.BUILD / "serial.log"
    serial.unlink(missing_ok=True)
    port = free_port()
    args = q.base_args(qemu, opts.memory, opts.cpus) + [
        "-display", "none",
        "-serial", "file:serial.log",
        "-qmp", f"tcp:127.0.0.1:{port},server=on,wait=off",
    ]
    print(f"Booting Nova in QEMU (headless, accel={q.accel()})...")
    proc = subprocess.Popen(args, cwd=q.BUILD, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    started = time.time()
    ok = False
    reason = "timed out"
    try:
        qmp = Qmp(port, proc)
        while time.time() - started < TIMEOUT_S:
            log = read_log(serial)
            if opts.expect and all(e in log for e in opts.expect):
                time.sleep(1.0)
                ok = True
                break
            failed = next((f for f in FAILURES if f in log), None)
            if failed:
                time.sleep(0.5)  # let the rest of the message arrive
                reason = f"saw failure: {failed!r}"
                break
            if all(m in log for m in MUST_SEE):
                ok = True
                break
            if proc.poll() is not None:
                reason = f"QEMU exited early with code {proc.returncode}"
                break
            time.sleep(0.25)
        elapsed = time.time() - started

        if ok and opts.mouse and not opts.expect:
            problem = mouse_test(qmp, serial)
            if problem:
                ok = False
                reason = "mouse test: " + problem
            else:
                print("Mouse: moved, left-clicked and right-clicked; the kernel saw all of it")

        if ok and opts.shell and not opts.expect:
            problem = shell_test(qmp, serial, opts.cpus or q.default_cpus())
            if problem:
                ok = False
                reason = "shell test: " + problem
            else:
                print("Shell: typed commands, edited with backspace, used history and Ctrl+C; every reply was correct")

        if ok and opts.user and not opts.expect:
            problem = user_test(qmp, serial)
            if problem:
                ok = False
                reason = "user-space test: " + problem
            else:
                print("User space: programs ran in ring 3, read the keyboard, were isolated, and bad ones were killed")

        if proc.poll() is None:
            time.sleep(0.5)  # let the last frame land
            ppm = q.BUILD / f"{opts.shot}.ppm"
            try:
                qmp.cmd("screendump", filename=str(ppm))
                time.sleep(0.3)
                (q.BUILD / f"{opts.shot}.png").write_bytes(ppm_to_png(ppm.read_bytes()))
                ppm.unlink()
                print(f"Screenshot: {q.BUILD / (opts.shot + '.png')}")
            except Exception as e:  # screenshot is a bonus, not the verdict
                print(f"(screenshot failed: {e})")
            try:
                qmp.cmd("quit")
            except Exception:
                pass
    finally:
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        out = proc.stdout.read().decode("utf-8", "replace").strip() if proc.stdout else ""
        if out:
            print("QEMU said:", out)

    log = read_log(serial)
    print("----- serial log -----")
    print(log.strip())
    print("----------------------")
    missing = [m for m in MUST_SEE if m not in log]
    if ok and opts.expect:
        print(f"PASS: saw expected {opts.expect!r} in {elapsed:.1f}s")
        return 0
    if ok:
        print(f"PASS: Nova booted to the kernel in {elapsed:.1f}s ({opts.memory} RAM)")
        return 0
    print(f"FAIL: {reason}; missing: {missing}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
