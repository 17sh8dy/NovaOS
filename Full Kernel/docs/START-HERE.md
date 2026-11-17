# START HERE (for a new session with no memory)

Nova Full Kernel = a real x86_64 OS: own UEFI bootloader (**NovaBoot**) + Rust kernel, **zero third-party
crates**. Separate from the Tauri desktop-app in the repo root (`src-tauri/`, never claim kernel powers for
that one). Read `README.md`, `docs/ROADMAP.md`, `docs/BOOT-PROTOCOL.md` and `docs/REAL-HARDWARE.md` for
detail; this page is the orientation + current state.

Machine-specific details (the signing machine's address, drive letters, which PC) are NOT in git (public
repo). They are in `LOCAL-NOTES.md` next to this folder's `README.md` (git-ignored). Read it too.

## Where we left off (2026-11-16, later): FIRST REAL BOOT WORKED

* **Nova booted on the ASUS PC with Secure Boot ON (shim + MOK, pefix.py fix worked).** The desktop loads, but
  the keyboard and mouse do nothing. Next: find out what the on-screen boot log says about `xhci:` / `usb:`
  lines (controller started? ports? "hubs not supported"? "not a keyboard or mouse"?), then fix the USB path
  in `kernel/src/xhci.rs`. Ask the user for a photo of the log/`usb` shell output.
* Photo taken: desktop + `nsh` prompt, "32 CPU(s)", pointer parked at its start position (no mouse report ever
  arrived). The xhci/usb log lines were not visible: `shell::terminal()` cleared the console. Fixed: it now
  replays the `xhci*`/`usb*` log lines above the greeting (diagnostic; built `-NoDisks -Safe 30`, builds OK, not
  yet tried in QEMU or on the stick). Devices: Razer Viper V3 Pro (cable; dongle mode also possible) and a
  SteelSeries Apex Pro TKL Wireless, both composite HID with no hub, both in rear ports.
* **Next:** sign the new build on the signing machine (it was unreachable at the end of the session), write the
  stick (confirm first), user boots it and photographs the Terminal window as soon as it appears.

## Earlier today (2026-11-16)

* **First real boot (ASUS and the Dell, same result): shim showed "Verification failed: (0x3) Unsupported"
  before MokManager.** Cause, reproduced in QEMU on the Dell (`tools/secureboot/qemu-test.sh`, serial log says
  "Section 2 is inside image headers. Malformed section header."): NovaBoot's `.data` had flags
  "initialised data" but no bytes in the file; shim's strict PE loader refuses that (firmware loaders accept it).
  Fix: `tools/pefix.py` marks such a section BSS; `build.ps1`/`build.sh` run it after copying BOOTX64.EFI
  (before signing). With the fix, qemu-test.sh passes all 4 checks (Nova reaches READY with Secure Boot ON).
* The stick (drive E:) was rewritten with the fixed NovaBoot (only `EFI\BOOT\grubx64.efi` changed; all 5 files
  hash-verified against the Dell-built `build/secureboot-esp`). **Next: the user retries F8 -> stick ->
  MokManager enrolment on the ASUS PC and reports what each screen shows.** Not yet committed/pushed
  (`tools/pefix.py`, `build.ps1`, `build.sh`, this file): ask before committing or pushing.
* Note: qemu-test.sh checks 1 and 3 used to pass spuriously (the old error also contained "Verification
  failed"); they now fail with Security Violation, as intended. Dell now has qemu/ovmf/virt-firmware installed.
* If Windows asked for the BitLocker recovery key after the MOK enrolment, that contradicts the PCR 7
  reasoning in `docs/REAL-HARDWARE.md`: record it there. The recovery screen has no time limit.
* Update this section after each real-hardware attempt (what happened, what to try next).

## State (2026-11-15)

* Stages 1-10 done (boot, CPU tables, memory, interrupts/SMP, user space, AHCI/NVMe, NovaFS, xHCI USB,
  desktop + user-space shell `nsh`). Everything is tested in QEMU by `tools\test.ps1` (all green).
* **Safety net** (`tools\build.ps1 -Safe 30`, `crates/safety`, `kernel/src/safety.rs`): a crash, a boot that
  never prints READY, or a frozen desktop shows the crash screen with a countdown, then resets the PC. A
  PS/2 key press stops the countdown. Off by default. Details in `docs/REAL-HARDWARE.md`.
* **Never booted on real hardware yet.** A Secure-Boot stick (shim + MOK) has been built and written; the
  first real boot is the next step. See "First real boot" below.

## Build and test (Windows, PowerShell)

* `tools\build.ps1` builds into `build\esp`. Switches: `-NoDisks` (SATA/NVMe drivers never start; use for
  real hardware), `-Safe 30` or `-Safe '30,90,45'` (QUOTE it: unquoted `3,8,6` becomes an array), `-Fault KIND`
  (deliberate fault for tests), `-X2Apic`, `-ApPage 70`.
* `tools\run.ps1` runs it in a QEMU window. `tools\test.ps1` = the whole suite (about 25 minutes).
* **Always finish by running `tools\build.ps1` with no switches** so `build\esp` is a normal kernel again.
* QEMU: use WHPX (`tools/qemu.py` already does). TCG crashes on this PC. If the user has a QEMU window
  open, do NOT kill it (it locks `build\esp`); ask them to close it.

## First real boot (what is planned)

1. Build `tools\build.ps1 -NoDisks -Safe 30`.
2. Sign NovaBoot on a Linux machine (`tools/secureboot/make-key.sh`, `make-stick.sh`; Windows has no
   `sbsign`). The MOK private key lives only on that Linux machine, never in git, chat or on the stick.
3. Copy `build\secureboot-esp` to a FAT USB stick (copy, don't format an unknown stick without asking).
4. Boot it from the firmware's one-time boot menu (F8 on ASUS). First time: shim opens MokManager ->
   Enroll key from disk -> `nova-mok.der` -> Continue -> Yes -> Reboot. Then boot the stick again.
5. Keyboard and mouse must be in rear motherboard USB ports (no USB hub support yet).

## Rules (the user's, and hard-won)

* **Secure Boot stays ON on the main PC.** Never suggest turning it off or changing firmware settings.
  The main PC's C: drive is BitLocker-encrypted (TPM, PCR 7+11). The recovery key exists as a printout the
  user verified against the live key. Never ask for or print the 48 digits.
* Nova never writes to a disk unless the user types `format N erase`; real-hardware builds use `-NoDisks`.
  NovaBoot never writes firmware variables or boot entries. (Shim's MOK enrolment does write shim's own
  MokList variable; tell the user.)
* Confirm before: writing/formatting any USB drive (look at it first), pushing, deploying.
* Push only branch `full-kernel` (kernel commits). Never merge it into `main` or push `main`.
* The main PC has an unresolved CPU Machine Check reset problem (see the user's memory notes); a sudden
  reset during a test may be that, not Nova.

## Traps

* Bash/Python heredocs mangle backslashes in Windows paths (`\b` becomes a backspace). Write patch
  scripts to a file with the Write tool, or use the Edit tool.
* `tools/boot_test.py --expect` used to pass spuriously when the kernel reached READY first; fixed
  2026-11-15. QEMU runs with `-no-reboot`, so a guest reset makes QEMU exit (`--expect-exit` checks it).
* A boot occasionally takes ~65 s before READY in QEMU (host hiccup, not investigated).
* Files named `con.rs`, `aux`, `nul`, `prn`, `com1` can't be added by Git for Windows.
* Git warns about LF/CRLF; scripted edits should keep each file's existing line endings.

## Next ideas

First real boot (above); USB hubs; USB storage; MSI; window resize; signals/redirection; FADT-based reset
(the safety net uses port 0xCF9, then the keyboard controller, then a triple fault; only QEMU-tested).
