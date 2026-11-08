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


def main() -> int:
    import argparse
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--memory", default="256M", help="guest RAM, e.g. 256M, 2G, 6G")
    ap.add_argument("--expect", metavar="TEXT",
                    help="negative test: pass if TEXT appears in the log instead of a full boot")
    opts = ap.parse_args()
    qemu = q.find_qemu()
    serial = q.BUILD / "serial.log"
    serial.unlink(missing_ok=True)
    port = free_port()
    args = q.base_args(qemu, opts.memory) + [
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
            if opts.expect and opts.expect in log:
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

        if proc.poll() is None:
            time.sleep(0.5)  # let the last frame land
            ppm = q.BUILD / "boot_test.ppm"
            try:
                qmp.cmd("screendump", filename=str(ppm))
                time.sleep(0.3)
                (q.BUILD / "boot_test.png").write_bytes(ppm_to_png(ppm.read_bytes()))
                ppm.unlink()
                print(f"Screenshot: {q.BUILD / 'boot_test.png'}")
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
