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


SHIFTED = {"|": "backslash", "&": "7", "_": "minus", "!": "1", ":": "semicolon"}


def type_text(qmp, text):
    for ch in text:
        if ch in QCODES:
            press(qmp, QCODES[ch])
        elif ch in SHIFTED:
            press(qmp, SHIFTED[ch], shift=True)
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
    time.sleep(1.0)  # give them time to be reaped, then ask
    before = len(re.findall(r"con: nova> ps\n", read_log(serial)))
    type_text(qmp, "ps\n")
    if not wait_for(r"con:   pid \d+ +nsh"):
        return "ps did not list the shell (nsh is a user process)"
    time.sleep(0.5)
    after_ps = read_log(serial).split("con: nova> ps\n")[before + 1] if before + 1 < len(read_log(serial).split("con: nova> ps\n")) else ""
    listed = re.findall(r"con:   pid \d+ +(\S+)", after_ps.split("con: nova>")[0])
    if listed != ["nsh"]:
        return f"after they all finished, ps should list only the shell, got {listed}"

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


DISK_KINDS = {
    # kind: (log line that finds it, model + size + bus as `disks` prints it)
    "sata": (r"ahci: disk (\d+): QEMU HARDDISK \(64 MiB\)", r"QEMU HARDDISK, 64 MiB, SATA"),
    "nvme": (r"nvme: disk (\d+): QEMU NVMe Ctrl \(64 MiB\)", r"QEMU NVMe Ctrl, 64 MiB, NVMe"),
}


def disk_test(qmp, serial, phase, kind="sata") -> str | None:
    """Phase 1 formats the (blank) virtual disk, installs programs and writes files.
    Phase 2 boots again on the SAME disk and checks everything survived.
    `kind` picks the SATA or the NVMe disk."""

    def wait_for(pattern, seconds=20):
        end = time.time() + seconds
        while time.time() < end:
            m = re.search(pattern, read_log(serial))
            if m:
                return m
            time.sleep(0.1)
        return None

    def count(pattern):
        return len(re.findall(pattern, read_log(serial)))

    def run(command, expect, seconds=20):
        before = count(expect)
        type_text(qmp, command + "\n")
        end = time.time() + seconds
        while time.time() < end:
            if count(expect) > before:
                return None
            time.sleep(0.1)
        return f"after {command!r} expected {expect!r}"

    if not wait_for(r"shell: ready"):
        return "the shell never started"
    log = read_log(serial)
    found_re, listed_re = DISK_KINDS[kind]
    found = re.search(found_re, log)
    if not found:
        return f"the {kind.upper()} disk was not found at boot"
    target = found.group(1)

    if phase == 1:
        if "no NovaFS on any disk" not in log:
            return "a fresh disk should not contain a NovaFS"
        type_text(qmp, "disks\n")
        m = wait_for(rf"con: +disk ({target}): {listed_re}")
        if not m:
            return f"the 64 MiB {kind.upper()} disk was not listed as disk {target}"
        idx = m.group(1)
        steps = [
            (f"format {idx} erase", r"con: formatted disk \d+ with NovaFS"),
            ("mounts", r"con: +/ +novafs"),
            ("install", r"con: installed \d+ programs in /bin"),
            ("mkdir /docs", None),
            ("write /docs/note.txt persistent hello world", r"con: wrote \d+ bytes to /docs/note.txt"),
            ("cat /docs/note.txt", r"con: persistent hello world"),
            ("cd /docs", None),
            ("pwd", r"con: /docs"),
            ("ls", r"con: +\d+ +note.txt"),
            ("cd /", None),
            ("run /bin/hello", r"Hello from user space! I am process \d+\."),
            ("run fsdemo", r"con: fsdemo ok: wrote, read, seeked, listed and deleted \d+ bytes"),
            ("run fsbig", r"con: fsbig ok: 5 MiB written, verified byte for byte, deleted"),
            ("run cat /docs/note.txt", r"\(cat\) exited with code 0"),
            ("run ls /bin", r"con: +\d+ +hello"),
            ("fscheck", r"con: file system is clean"),
            ("rm /nonexistent", r"con: /nonexistent: no such file or directory"),
            ("sync", r"con: synced"),
        ]
    else:
        if f"vfs: mounted NovaFS from disk {target} at /" not in log:
            return f"the NovaFS on disk {target} was not mounted at boot"
        steps = [
            ("cat /docs/note.txt", r"con: persistent hello world"),
            ("ls /bin", r"con: +\d+ +hello"),
            ("run /bin/hello", r"Hello from user space! I am process \d+\."),
            ("run ls /docs", r"con: +\d+ +note.txt"),
            ("fscheck", r"con: file system is clean"),
            ("df", r"con: +/ +novafs +\d+ KiB free"),
            ("rm /docs/note.txt", None),
            ("ls /docs", None),
            ("fscheck", r"con: file system is clean"),
            ("sync", r"con: synced"),
        ]
    for command, expect in steps:
        if expect is None:
            type_text(qmp, command + "\n")
            time.sleep(0.4)
            continue
        err = run(command, expect)
        if err:
            return err
    if phase == 2 and count(r"con: +\d+ +note.txt") > 1 and "note.txt" in "".join(re.findall(r"nova> ls /docs\n(?:con: .*\n)*", read_log(serial))[-1:]):
        return "rm did not remove the file"
    return None


def usb_test(qmp, serial, typed: bool, moved: bool) -> str | None:
    """With a USB keyboard and mouse attached: they were found, the keys and
    pointer the other tests used really came through USB, and unplugging and
    plugging the keyboard back in works."""

    def wait_for(pattern, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            m = re.search(pattern, read_log(serial))
            if m:
                return m
            time.sleep(0.1)
        return None

    def count(pattern):
        return len(re.findall(pattern, read_log(serial)))

    for pattern, what in [
        (r"usb: port \d+: QEMU USB Keyboard \(0627:0001\), [^:]+: keyboard", "the USB keyboard was not set up"),
        (r"usb: port \d+: QEMU USB Mouse \(0627:0001\), [^:]+: mouse", "the USB mouse was not set up"),
        *([(r"usb: first key from QEMU USB Keyboard", "no key arrived through the USB keyboard")] if typed else []),
        *([(r"usb: first report from QEMU USB Mouse", "no movement arrived through the USB mouse")] if moved else []),
    ]:
        if not wait_for(pattern, 5):
            return what
    type_text(qmp, "usb\n")
    if not wait_for(r"con:   port \d+: QEMU USB Keyboard, .*keyboard"):
        return "the 'usb' command did not list the keyboard"

    # Unplug the keyboard, plug it back in, and type through it again.
    attached = r"usb: port \d+: QEMU USB Keyboard \("
    qmp.cmd("device_del", id="usbkbd")
    if not wait_for(r"usb: port \d+: QEMU USB Keyboard unplugged", 10):
        return "unplugging the keyboard was not noticed"
    qmp.cmd("device_add", driver="usb-kbd", bus="xhci.0", id="usbkbd")
    end = time.time() + 10
    while time.time() < end and count(attached) < 2:
        time.sleep(0.1)
    if count(attached) < 2:
        return "the keyboard was not set up again after plugging it back in"
    time.sleep(0.5)
    type_text(qmp, "echo replugged ok\n")
    if not wait_for(r"con: replugged ok"):
        return "typing did not work after plugging the keyboard back in"
    return None


def screenshot(qmp, name):
    """Saves the guest screen to build/<name>.png."""
    ppm = q.BUILD / f"{name}.ppm"
    qmp.cmd("screendump", filename=str(ppm))
    time.sleep(0.3)
    (q.BUILD / f"{name}.png").write_bytes(ppm_to_png(ppm.read_bytes()))
    ppm.unlink(missing_ok=True)


def desktop_test(qmp, serial) -> str | None:
    """Uses the desktop like a person would, through QEMU's mouse and keyboard:
    the launcher, the Files window (keyboard browsing, preview), dragging,
    closing, focus moving between windows (and with it the keyboard), the
    System window, minimising and restoring from the taskbar. Every click's
    position is checked against what the kernel says the pointer did."""

    def log():
        return read_log(serial)

    def wait_for(pattern, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            m = re.search(pattern, log())
            if m:
                return m
            time.sleep(0.1)
        return None

    def count(pattern):
        return len(re.findall(pattern, log()))

    def wait_new(pattern, before, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            ms = re.findall(pattern, log())
            if len(ms) > before:
                return ms[-1]
            time.sleep(0.1)
        return None

    # Where is the pointer now? The last button event or first movement says.
    pos = None
    for m in re.finditer(r"mouse: (?:first movement, pointer|left button (?:down|up)|right button (?:down|up)) at \((\d+), (\d+)\)", log()):
        pos = [int(m.group(1)), int(m.group(2))]
    if pos is None:
        m = re.search(r"desktop: (\d+)x(\d+),", log())
        pos = [int(m.group(1)) // 2, int(m.group(2)) // 2]

    def move_to(x, y):
        dx, dy = x - pos[0], y - pos[1]
        while dx or dy:
            sx = max(-80, min(80, dx))
            sy = max(-80, min(80, dy))
            qmp.cmd("input-send-event", events=[
                {"type": "rel", "data": {"axis": "x", "value": sx}},
                {"type": "rel", "data": {"axis": "y", "value": sy}},
            ])
            dx -= sx
            dy -= sy
            time.sleep(0.03)
        pos[0], pos[1] = x, y

    def button(down):
        before = count(rf"mouse: left button {'down' if down else 'up'} at \({pos[0]}, {pos[1]}\)")
        qmp.cmd("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": "left"}}])
        if wait_new(rf"mouse: left button {'down' if down else 'up'} at \({pos[0]}, {pos[1]}\)", before, 5) is None:
            raise AssertionError(f"the kernel did not see the button {'press' if down else 'release'} at {tuple(pos)} "
                                 f"(pointer out of step?)")

    def click(x, y):
        move_to(x, y)
        button(True)
        button(False)

    def geometry(name, what):
        m = wait_for(rf"wm: {name} {what} at \((-?\d+), (-?\d+)\) \d+x\d+: title \((\d+), (\d+)\), close \((\d+), (\d+)\), "
                     rf"minimize \((\d+), (\d+)\), taskbar \((\d+), (\d+)\)")
        if not m:
            return None
        v = [int(g) for g in m.groups()]
        return {"at": (v[0], v[1]), "title": (v[2], v[3]), "close": (v[4], v[5]), "min": (v[6], v[7]), "task": (v[8], v[9])}

    def menu_item(name):
        before = count(r"desktop: menu opened")
        click(*launcher)
        if wait_new(r"desktop: menu opened", before, 5) is None:
            return None
        m = re.findall(rf"desktop: menu opened: .*?{name} at \((\d+), (\d+)\)", log())
        return (int(m[-1][0]), int(m[-1][1])) if m else None

    try:
        m = wait_for(r"desktop: ready; launcher at \((\d+), (\d+)\)")
        if not m:
            return "the desktop never said it was ready"
        launcher = (int(m.group(1)), int(m.group(2)))
        if not wait_for(r"shell: ready"):
            return "the shell never started"
        term = geometry("Terminal", "open")

        # Something to look at in Files (the RAM file system is fine).
        for command, expect in [("install", r"con: installed \d+ programs"), ("mkdir /docs", None),
                                ("write /docs/hello.txt Hello from the Files window", r"con: wrote \d+ bytes")]:
            type_text(qmp, command + "\n")
            if expect and not wait_for(expect):
                return f"terminal: {command!r} did not answer {expect!r}"
            time.sleep(0.3)

        # Open Files from the launcher.
        item = menu_item("Files")
        if not item:
            return "clicking the launcher did not open the menu"
        click(*item)
        files = geometry("Files", "opened")
        if not files:
            return "choosing Files in the menu did not open the Files window"
        if not wait_for(r"files: / \(\d+ entries\)"):
            return "Files did not list /"

        # Keyboard browsing (keys now go to Files, not the shell).
        before = count(r"files: /bin \(")
        press(qmp, "ret")
        if wait_new(r"files: /bin \((\d+) entries\)", before, 5) is None:
            return "Enter on 'bin' did not open /bin"
        press(qmp, "backspace")
        press(qmp, "down")
        press(qmp, "ret")
        if not wait_for(r"files: /docs \(1 entries\)"):
            return "Backspace, Down, Enter did not open /docs"
        press(qmp, "down")
        if not wait_for(r"files: preview /docs/hello.txt \(\d+ bytes\)"):
            return "selecting hello.txt did not preview it"
        time.sleep(0.3)
        screenshot(qmp, "desktop_files")
        type_text(qmp, "echo leaked\n")
        time.sleep(1.0)
        if "con: leaked" in log():
            return "typing into the Files window reached the shell"

        # Drag Files by its title bar.
        tx, ty = files["title"]
        move_to(tx, ty)
        button(True)
        move_to(tx - 200, ty + 60)
        button(False)
        moved = geometry("Files", "moved")
        want = (files["at"][0] - 200, files["at"][1] + 60)
        if not moved or moved["at"] != want:
            return f"dragging the title bar should move Files to {want}, got {moved and moved['at']}"

        # Close it: focus (and the keyboard) fall back to the Terminal.
        click(*moved["close"])
        if not wait_for(r"wm: Files closed; focus Terminal"):
            return "the close button did not close Files (or focus did not return to the Terminal)"
        type_text(qmp, "echo back in the terminal\n")
        if not wait_for(r"con: back in the terminal"):
            return "after Files closed, typing did not reach the shell"

        # System: open; click the Terminal to focus it, then System's taskbar button.
        item = menu_item("System")
        click(*item)
        system = geometry("System", "opened")
        if not system or not wait_for(r"system: window shows \d+ lines"):
            return "the System window did not open"
        click(*term["title"])
        if not wait_for(r"wm: focus Terminal"):
            return "clicking the Terminal's title did not focus it"
        type_text(qmp, "echo focus follows clicks\n")
        if not wait_for(r"con: focus follows clicks"):
            return "after clicking the Terminal, typing did not reach the shell"
        if count(r"wm: Terminal moved") or count(r"wm: Files moved") != 1:
            return "a click on a title bar was reported as a move"
        click(*system["task"])
        if not wait_for(r"wm: focus System"):
            return "System's taskbar button did not bring it to the front"
        click(*system["task"])
        if not wait_for(r"wm: System minimized"):
            return "clicking the focused window's taskbar button did not minimise it"
        click(*system["task"])
        if not wait_for(r"wm: System restored"):
            return "clicking a minimised window's taskbar button did not restore it"
        time.sleep(1.2)  # a clock tick and a System refresh
        screenshot(qmp, "desktop_system")
        # Hand the keyboard back to the shell for whatever test runs next.
        before = count(r"wm: focus Terminal")
        click(*term["title"])
        if wait_new(r"wm: focus Terminal", before, 5) is None:
            return "could not give the Terminal the focus back"
        move_to(pos[0], pos[1] - 120)  # keep the pointer off the taskbar for the last screenshot
    except AssertionError as e:
        return str(e)
    return None


def nsh_test(qmp, serial) -> str | None:
    """The shell is a user program: built-ins, cd affecting the programs it
    starts, pipelines of two and three programs, background jobs, Ctrl+C
    ending a program that reads the keyboard, and exit restarting the shell."""

    def log():
        return read_log(serial)

    def wait_for(pattern, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            m = re.search(pattern, log())
            if m:
                return m
            time.sleep(0.1)
        return None

    def output_of(command, seconds=15):
        """Types `command`, then an end marker, and returns the console lines in between
        (the shell runs one command at a time, so that is exactly what `command` printed)."""
        output_of.n = getattr(output_of, "n", 0) + 1
        marker = f"con: nova> {command}\n"
        done = f"con: end-{output_of.n}\n"
        before = log().count(marker)
        type_text(qmp, command + "\n")
        type_text(qmp, f"echo end-{output_of.n}\n")
        end = time.time() + seconds
        while time.time() < end:
            text = log()
            parts = text.split(marker)
            if len(parts) > before + 1 and done in parts[before + 1]:
                block = parts[before + 1].split(f"con: nova> echo end-{output_of.n}\n")[0]
                return [l[5:] for l in block.splitlines() if l.startswith("con: ") and not l.startswith("con: (input)")]
            time.sleep(0.1)
        return None

    if not wait_for(r"shell: ready \(nsh, a user program, is process \d+\)"):
        return "the shell (nsh) never started as a user process"
    if not wait_for(r"con: Nova shell \(nsh, a user program\)"):
        return "nsh did not greet"

    out = output_of("install")
    if not out or not re.search(r"installed (\d+) programs", " ".join(out)):
        return f"install: {out}"
    programs = int(re.search(r"installed (\d+) programs", " ".join(out)).group(1))

    out = output_of("ls /bin | grep ns")
    if out is None or [l.split()[-1] for l in out] != ["nsh"]:
        return f"'ls /bin | grep ns' should print just nsh, got {out}"
    out = output_of("ls /bin | wc")
    if not out or not out[0].startswith(f"{programs} lines,"):
        return f"'ls /bin | wc' should count {programs} lines, got {out}"
    out = output_of("ls /bin | grep e | wc")
    with_e = sum(1 for n in ["cat", "counter", "crash", "evil", "fsbig", "fsdemo", "greet", "grep", "hello", "ls", "memtest", "nsh", "wc"] if "e" in n)
    if not out or not out[0].startswith(f"{with_e} lines,"):
        return f"'ls /bin | grep e | wc' should count {with_e} lines, got {out}"

    out = output_of("cd /bin")
    out = output_of("pwd")
    if out != ["/bin"]:
        return f"pwd after cd /bin: {out}"
    out = output_of("ls | grep nsh")
    if not out or "nsh" not in out[0]:
        return f"a program started after 'cd /bin' should list /bin, got {out}"
    out = output_of("cd /nope")
    if out != ["cd: /nope: no such file or directory"]:
        return f"cd to a missing folder: {out}"
    output_of("cd /")

    out = output_of("hello &")
    if not out or not re.match(r"\[process \d+ \(hello\) started in the background\]", out[0]):
        return f"a background job: {out}"
    if not wait_for(r"\[process \d+ \(hello\) exited with code 0\]"):
        return "the background job's end was not reported"

    # A program reading the keyboard ends at Ctrl+C (end of input) instead of hanging the Terminal.
    type_text(qmp, "grep x\n")
    time.sleep(0.8)
    qmp.cmd("input-send-event", events=[
        {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "ctrl"}}},
        {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "c"}}},
        {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "c"}}},
        {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "ctrl"}}},
    ])
    if not wait_for(r"con: \[grep exited with code 1\]"):
        return "Ctrl+C did not end a program that was reading the keyboard"

    before = len(re.findall(r"shell: ready \(nsh", log()))
    type_text(qmp, "exit\n")
    end = time.time() + 10
    while time.time() < end and len(re.findall(r"shell: ready \(nsh", log())) <= before:
        time.sleep(0.1)
    if len(re.findall(r"shell: ready \(nsh", log())) <= before:
        return "after 'exit' the Terminal did not start a new shell"
    time.sleep(0.5)
    out = output_of("echo still here")
    if out != ["still here"]:
        return f"the restarted shell: {out}"
    out = output_of("ps")
    if not out or not any(re.match(r"  pid \d+ +nsh", l) for l in out):
        return f"ps should list the shell: {out}"
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
    ap.add_argument("--disk-phase", type=int, choices=(1, 2), help="storage test: 1 = format + write files, 2 = boot again and check they persist")
    ap.add_argument("--disk-kind", choices=sorted(DISK_KINDS), default="sata", help="which disk the storage test uses")
    ap.add_argument("--no-ps2", action="store_true", help="a machine without a PS/2 controller (like most laptops): USB is the only input")
    ap.add_argument("--nvme-block", type=int, choices=(512, 4096), default=512, help="sector size of the virtual NVMe disk")
    ap.add_argument("--nsh", action="store_true", help="the user-space shell: pipelines, cd, background jobs, Ctrl+C, exit")
    ap.add_argument("--desktop", action="store_true", help="use the desktop: launcher, Files, dragging, focus, System, taskbar")
    ap.add_argument("--usb", action="store_true", help="attach a USB keyboard + mouse (QEMU input goes to them) and test USB")
    ap.add_argument("--fresh-disk", action="store_true", help="start from a blank virtual disk")
    ap.add_argument("--user", action="store_true", help="after boot, run user-space programs from the shell")
    ap.add_argument("--shell", action="store_true", help="after boot, type commands into the shell through QEMU's keyboard")
    ap.add_argument("--mouse", action="store_true",
                    help="after boot, move the mouse and click through QEMU and check the kernel saw it")
    opts = ap.parse_args()
    qemu = q.find_qemu()
    if opts.fresh_disk:
        q.ensure_disk(fresh=True)
    serial = q.BUILD / "serial.log"
    serial.unlink(missing_ok=True)
    port = free_port()
    args = q.base_args(qemu, opts.memory, opts.cpus, usb=opts.usb, nvme_block=opts.nvme_block, ps2=not opts.no_ps2) + [
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

        if ok and opts.disk_phase and not opts.expect:
            problem = disk_test(qmp, serial, opts.disk_phase, opts.disk_kind)
            kind = opts.disk_kind.upper()
            if problem:
                ok = False
                reason = f"{kind} disk test (phase {opts.disk_phase}): " + problem
            elif opts.disk_phase == 1:
                print(f"{kind} disk (phase 1): formatted, installed programs, wrote files, ran from disk, fsck clean")
            else:
                print(f"{kind} disk (phase 2): after a reboot the files, the installed programs and a clean fsck are all still there")

        if ok and opts.usb and not opts.expect:
            problem = usb_test(qmp, serial, typed=opts.shell, moved=opts.mouse)
            if problem:
                ok = False
                reason = "USB test: " + problem
            else:
                print("USB: keyboard and mouse found, typing and pointing came through USB, unplug + replug worked")

        if ok and opts.nsh and not opts.expect:
            problem = nsh_test(qmp, serial)
            if problem:
                ok = False
                reason = "nsh test: " + problem
            else:
                print("nsh: built-ins, cd, pipelines (2 and 3 programs), background jobs, Ctrl+C and exit/restart all worked")

        if ok and opts.desktop and not opts.expect:
            problem = desktop_test(qmp, serial)
            if problem:
                ok = False
                reason = "desktop test: " + problem
            else:
                print("Desktop: launcher, Files (browse + preview), drag, close, focus + keyboard routing, System, taskbar minimise/restore all worked")

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
