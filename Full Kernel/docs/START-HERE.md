# START HERE (for a new session with no memory)

Nova Full Kernel = a real x86_64 OS: own UEFI bootloader (**NovaBoot**) + Rust kernel, **zero third-party
crates**. Separate from the Tauri desktop-app in the repo root (`src-tauri/`, never claim kernel powers for
that one). Read `README.md`, `docs/ROADMAP.md`, `docs/BOOT-PROTOCOL.md` and `docs/REAL-HARDWARE.md` for
detail; this page is the orientation + current state.

Machine-specific details (the signing machine's address, drive letters, which PC) are NOT in git (public
repo). They are in `LOCAL-NOTES.md` next to this folder's `README.md` (git-ignored). Read it too.

## 2026-11-18: USB mouse fix: CONFIRMED WORKING ON THE ASUS

* Photo of the diagnostic stick: both xHCI controllers start, the keyboard (Apex) works, the Razer Viper V3 Pro
  mouse did not move the pointer. Its HID report descriptor showed why: buttons, **2 vendor bytes**, wheel,
  then **16-bit X and Y**: not the 3-byte boot layout Nova assumed. Fix: `crates/usb/src/report.rs` parses the
  mouse's report descriptor (host-tested with the Razer's real bytes) and `kernel/src/xhci.rs` decodes with it
  (report protocol when the layout is understood, boot layout as fallback). The `usb` shell command shows
  per-endpoint counters and the last raw report. **Confirmed by the user on the ASUS (Secure Boot ON):
  keyboard and mouse both work, and Terminal, Settings, Notes and Calculator were all used.** Also fixed: the scheduler stack-reuse self-test flaked on
  many-CPU machines (a dead thread's stack returns to the pool a moment after `join` returns).
* Still open: `port 2: could not read the device descriptor` on the second xHCI (0b:00.3); USB hubs.

## 2026-11-17 (later): stage 12, apps + window sizing (QEMU only)

* Stages 11 (`8168f86`) and 12 (`c8406c7`) are committed and **pushed** to `full-kernel`. Stage 12: Calculator, Notes, Settings, maximise/resize,
  idle auto-lock, `/system` folder. Details: `docs/ROADMAP.md` "Stage 12 notes". The roadmap now has the
  plan for **Vexa on Nova** (needs networking first; Vexa itself is Chromium and can't run here).
* **USB on the ASUS is still not fixed:** the diagnostic build that shows the `xhci:`/`usb:` lines in the
  Terminal is on the stick. The user boots it and photographs the Terminal; fix `kernel/src/xhci.rs` from that.

## 2026-11-17: stage 11, login password + desktop polish (QEMU only)

* New: a password lock screen (at boot when a password is set, launcher Lock, Ctrl+Alt+L, `lock`, `passwd`),
  `crates/auth` (SHA-256/PBKDF2), hover highlights, Alt+Tab, launcher Restart. Details: `docs/ROADMAP.md`
  "Stage 11 notes". No password set = the desktop opens as before (all older tests unchanged).
* **For a locked USB stick:** `tools\build.ps1 -NoDisks -Safe 30 -Password` (asks for the password; only a
  salted hash goes into KERNEL.ELF). **But the USB keyboard does not work on the ASUS yet** (below): a locked
  stick there could not be unlocked. Fix USB input first, or only use `-Password` with a PS/2 keyboard.
* Seen once in QEMU: `scheduler self-test FAILED: dead threads' stacks are not being reused` (panic at boot);
  the rerun passed. Pre-existing and intermittent, not investigated yet.

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
* **Next:** the new build is signed and on stick E: (5 files hash-verified, qemu-test.sh 4/4 pass). User boots it,
  and photographs the Terminal window as soon as it appears (lists the xhci/usb lines).

## Earlier today (2026-11-16)

* **First real boot (ASUS and the Dell, same result): shim showed "Verification failed: (0x3) Unsupported"
  before MokManager.** Cause, reproduced in QEMU on the Dell (`tools/secureboot/qemu-test.sh`, serial log says
  "Section 2 is inside image headers. Malformed section header."): NovaBoot's `.data` had flags
  "initialised data" but no bytes in the file; shim's strict PE loader refuses that (firmware loaders accept it).
  Fix: `tools/pefix.py` marks such a section BSS; `build.ps1`/`build.sh` run it after copying BOOTX64.EFI
  (before signing). With the fix, qemu-test.sh passes all 4 checks (Nova reaches READY with Secure Boot ON).
* The stick (drive E:) was rewritten with the fixed NovaBoot (only `EFI\BOOT\grubx64.efi` changed; all 5 files
  hash-verified against the Dell-built `build/secureboot-esp`). **Next: the user retries F8 -> stick ->
  MokManager enrolment on the ASUS PC and reports what each screen shows.** (Since committed and pushed.)
* Note: qemu-test.sh checks 1 and 3 used to pass spuriously (the old error also contained "Verification
  failed"); they now fail with Security Violation, as intended. Dell now has qemu/ovmf/virt-firmware installed.
* If Windows asked for the BitLocker recovery key after the MOK enrolment, that contradicts the PCR 7
  reasoning in `docs/REAL-HARDWARE.md`: record it there. The recovery screen has no time limit.
* Update this section after each real-hardware attempt (what happened, what to try next).

## State (2026-11-17)

* Stages 1-12 done (boot, CPU tables, memory, interrupts/SMP, user space, AHCI/NVMe, NovaFS, xHCI USB,
  desktop + user-space shell `nsh`, login password, Calculator/Notes/Settings, window resizing, `/system`).
  Everything is tested in QEMU by `tools\test.ps1` (about 25 minutes).
* **Safety net** (`tools\build.ps1 -Safe 30`, `crates/safety`, `kernel/src/safety.rs`): a crash, a boot that
  never prints READY, or a frozen desktop shows the crash screen with a countdown, then resets the PC. A
  PS/2 key press stops the countdown. Off by default. Details in `docs/REAL-HARDWARE.md`.
* **Real hardware:** Nova boots on the ASUS PC with Secure Boot ON (shim + MOK) to its desktop, and USB
  keyboard and mouse work there (confirmed 2026-11-18). The locked-stick warning (`-Password`) no longer applies.

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

Fix USB input on the ASUS (above); networking, then Vexa on Nova (roadmap); Notes selection + clipboard;
USB hubs; USB storage; MSI; window resize; signals/redirection; FADT-based reset
(the safety net uses port 0xCF9, then the keyboard controller, then a triple fault; only QEMU-tested).
