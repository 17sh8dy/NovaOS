"""Shared QEMU setup for run.ps1 and boot_test.py.

Boots build/esp as a FAT drive using QEMU's bundled UEFI firmware (edk2/OVMF).
"""

import os
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build"
ESP = BUILD / "esp"


def find_qemu() -> Path:
    candidates = [
        os.environ.get("NOVA_QEMU"),
        shutil.which("qemu-system-x86_64"),
        r"C:\Program Files\qemu\qemu-system-x86_64.exe",
    ]
    for c in candidates:
        if c and Path(c).is_file():
            return Path(c)
    raise SystemExit(
        "qemu-system-x86_64 not found. Install it with:\n"
        "  winget install SoftwareFreedomConservancy.QEMU\n"
        "or set NOVA_QEMU to its path."
    )


def firmware(qemu: Path) -> tuple[Path, Path]:
    """Returns (code, vars). The vars file is a private writable copy."""
    share = qemu.parent / "share"
    code = share / "edk2-x86_64-code.fd"
    vars_template = share / "edk2-i386-vars.fd"  # same format for x86_64
    if not code.is_file() or not vars_template.is_file():
        raise SystemExit(f"UEFI firmware not found in {share}")
    fw = BUILD / "firmware"
    fw.mkdir(parents=True, exist_ok=True)
    local_code = fw / "code.fd"
    local_vars = fw / "vars.fd"
    if not local_code.exists() or local_code.stat().st_size != code.stat().st_size:
        shutil.copyfile(code, local_code)
    if not local_vars.exists():
        shutil.copyfile(vars_template, local_vars)
    return local_code, local_vars


def accel() -> str:
    """CPU backend. Default: WHPX (the Windows Hypervisor Platform).

    QEMU 11.1's software emulator (TCG) crashes on this machine with
    0xC0000409 the moment the guest CPU starts, even with no guest at all
    (`qemu-system-x86_64 -display none -nodefaults`), while WHPX works.
    Set NOVA_ACCEL=tcg to try the emulator anyway (e.g. on another PC).
    """
    return os.environ.get("NOVA_ACCEL", "whpx")


DISK = BUILD / "disk.img"
NVME = BUILD / "nvme.img"
DISK_MIB = 64


def ensure_disk(fresh: bool = False) -> Path:
    """The virtual disks Nova can format and keep files on (sparse 64 MiB files):
    disk.img is a SATA disk, nvme.img an NVMe SSD. `fresh` blanks both."""
    BUILD.mkdir(parents=True, exist_ok=True)
    for img in (DISK, NVME):
        if fresh or not img.exists():
            img.unlink(missing_ok=True)
            with open(img, "wb") as f:
                f.truncate(DISK_MIB << 20)
    return DISK


def usb_default() -> bool:
    """Plug a USB keyboard + mouse in (NOVA_USB=1)."""
    return os.environ.get("NOVA_USB", "0") not in ("", "0")


def default_cpus() -> int:
    """How many CPUs the virtual machine has (NOVA_CPUS overrides; default 4)."""
    return int(os.environ.get("NOVA_CPUS", "4"))


def base_args(qemu: Path, memory: str = "256M", cpus: int | None = None, usb: bool | None = None, nvme_block: int = 512, ps2: bool = True) -> list[str]:
    """QEMU arguments. Paths are relative to BUILD, which must be the cwd
    (QEMU's -drive syntax can't take a path with ',' and the vvfat driver is
    simplest with a relative directory)."""
    if not (ESP / "EFI" / "BOOT" / "BOOTX64.EFI").is_file():
        raise SystemExit("build/esp is missing; run tools/build.ps1 first")
    firmware(qemu)
    ensure_disk()
    if usb is None:
        usb = usb_default()
    args = [
        str(qemu),
        "-machine", "q35" if ps2 else "q35,i8042=off",
        "-accel", accel(),
        "-m", memory,
        "-smp", str(cpus or default_cpus()),
        "-drive", "if=pflash,format=raw,readonly=on,file=firmware/code.fd",
        "-drive", "if=pflash,format=raw,file=firmware/vars.fd",
        "-drive", "format=raw,file=fat:rw:esp",
        # A second SATA disk for Nova's own file system (the boot volume above is the first).
        "-drive", "if=none,id=nova0,format=raw,file=disk.img",
        "-device", "ide-hd,drive=nova0,bus=ide.1",
        # An NVMe SSD (what most real PCs have).
        "-drive", "if=none,id=nvme0,format=raw,file=nvme.img",
        "-device", f"nvme,serial=NOVA0001,drive=nvme0,logical_block_size={nvme_block},physical_block_size={nvme_block}",
        # A USB 3 (xHCI) controller, always there; keyboard + mouse on request.
        "-device", "qemu-xhci,id=xhci",
        "-nic", "none",
        "-no-reboot",
    ]
    if usb:
        # With these attached, QEMU sends its keyboard/mouse input to the USB
        # devices instead of PS/2.
        args += ["-device", "usb-kbd,bus=xhci.0,id=usbkbd", "-device", "usb-mouse,bus=xhci.0,id=usbmouse"]
    return args
