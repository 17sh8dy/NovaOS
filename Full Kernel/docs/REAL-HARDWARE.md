# Booting Nova on a real PC

Nova has only run in QEMU. This is the plan for its first real boots, written
for two machines: a **Dell running Ubuntu** (first) and the **main Windows PC**
(ASUS PRIME B550-PLUS, Ryzen 9 5950X, RX 7800 XT, BitLocker on C:). Rule one:
**no firmware setting is changed and no disk is written.**

## What Nova does and doesn't touch (checked in the code)

| Thing | What happens |
|---|---|
| Disks | At boot each disk's block 0 is *read* (looking for a NovaFS). Nothing is written unless you type `format N erase`. `sync`/`reboot`/`halt` write only to a disk that already holds a NovaFS. The `-NoDisks` build never even starts the SATA/NVMe drivers. |
| Firmware variables (NVRAM) | NovaBoot never writes one. It *reads* `SecureBoot` and `SetupMode` to report the Secure Boot state. |
| Boot entries / boot order | Untouched. Boot the stick from the one-time boot menu (F8 on ASUS, F12 on Dell). The stick has no `fbx64.efi` (shim's helper that creates boot entries). |
| BIOS chip, CMOS | Not written. The clock is only read. |
| Devices | Reset or reconfigured in RAM only (USB, disk controllers); a power cycle undoes everything. |

## Secure Boot without changing it: shim + MOK

The PC trusts Microsoft's keys. **shim** (Ubuntu's, signed by Microsoft) runs,
then runs `grubx64.efi` if it is signed by a key in its own list (the
**MOK**, Machine Owner Key). We sign NovaBoot with our own key and enrol that
key once per PC:

1. `tools/secureboot/make-key.sh` (on Linux): key pair in `~/.nova-keys`. The
   private key never leaves that folder.
2. `tools/build.sh` then `tools/secureboot/make-stick.sh`: signed NovaBoot +
   shim + MokManager + the kernel + `nova-mok.der`, in `build/secureboot-esp`.
3. First boot of the stick on a PC: shim can't verify NovaBoot yet and opens
   **MokManager** -> *Enroll key from disk* -> the stick -> `nova-mok.der` ->
   *Continue* -> *Yes* -> *Reboot*. (On Ubuntu: `sudo mokutil --import nova-mok.der`, reboot, confirm.)
4. From then on Nova boots with Secure Boot ON; NovaBoot prints `Secure Boot: ON`.

The enrolled key lives in shim's MokList variable, **not** in the firmware's
Secure Boot database (db/KEK/PK), so the Secure Boot policy Windows measures
for BitLocker (TPM PCR 7) should not change. Have the recovery key anyway.

**shim's PE loader is stricter than the firmware's.** NovaBoot must have no section that is flagged
"initialised data" yet has no bytes in the file (lld leaves that for all-zero statics in `.data`). shim then
shows "Verification failed: (0x3) Unsupported" before MokManager (the serial log says "Section N is inside
image headers"). `tools/pefix.py` (run by `build.ps1`/`build.sh` before signing) marks such a section as BSS.
If shim ever shows `Unsupported` again, run `qemu-test.sh` on the Dell and read `build/sb-test/*.log`: a
key-not-enrolled refusal says "Security Violation", anything else is a file-format problem.

`tools/secureboot/qemu-test.sh` checks all of this in QEMU with Secure Boot
enforcing and Microsoft's keys: refused without the key, boots with it, and
refuses a tampered NovaBoot.

**Microsoft-signed Nova (no MOK step):** possible later via the public shim
review (a registered organisation, an EV code-signing certificate, SBAT
upkeep). Not needed for our own machines. NovaBoot already carries the
`.sbat` section shim requires (`boot/src/sbat.rs`).

## The safety net (build with `-Safe`)

For a first boot on a real PC build the stick's kernel with the safety net:

    tools\build.ps1 -NoDisks -Safe 30

* **A crash** (exception or panic) shows the crash screen with a green box in the
  corner counting down from 30 s, then **resets the PC**. The firmware starts over
  with the boot order it already had: Windows, if the Nova stick is only ever
  booted from the one-time boot menu (F8 on ASUS) and was not made the first boot
  device. Photograph the screen during the count: that is the bug report.
* **A PS/2 key press stops the countdown** and leaves the screen up. USB keyboards
  cannot stop it (the crash path does not run the USB driver): that is why the
  count is 30 s, long enough for a photo.
* **A boot that has not printed READY after 90 s**, or a **desktop that has not
  run for 30 s**, is treated as a crash (same screen, same countdown). The check
  runs from the timer interrupt, so it also catches hangs inside drivers; it can
  not catch a hang before the timer starts (power button then) or one with
  interrupts off.
* `-Safe 30,90,45` sets crash, boot and desktop seconds. Nothing is ever written:
  the only thing the safety net does to the PC is reset it.
* The reset tries the chipset reset register (port 0xCF9), then the keyboard
  controller pulse, then a triple fault. Only the QEMU reset is tested; real AMD
  hardware is not. If the reset does nothing, hold the power button.
* If the firmware ever boots the stick first, a crash would loop: remove the stick.
  (Nova cannot count crashes across a reset without writing somewhere; it doesn't.)
* Off by default: the QEMU tests rely on a crash screen that stays up.

## Before touching the Windows PC

1. **Save the BitLocker recovery key** (admin PowerShell, while C: is unlocked):
   `manage-bde -protectors -get C: -Type RecoveryPassword`. Write the 48 digits
   down (paper + phone), not on C:. Note the key ID.
2. Check (no secrets in the output): `manage-bde -protectors -get C: -Type TPM`
   (expect "PCR Validation Profile: 7, 11") and `manage-bde -status` (other drives).
3. Check the firmware trusts the Microsoft CA that shim is signed with (admin):
   `$db=[Text.Encoding]::ASCII.GetString((Get-SecureBootUEFI db).bytes); $db -match 'Microsoft Corporation UEFI CA 2011'; $db -match 'Microsoft UEFI CA 2023'`
4. Back up important files. Photograph the BIOS Boot and Secure Boot pages.

## Test order

1. **QEMU** (Windows: `tools\test.ps1`; Dell: `qemu-test.sh`): green.
2. **Dell**, from a stick, `--no-disks` build first, then a normal build.
3. **Main PC**, from a stick (shim + MOK), Secure Boot left ON, `-NoDisks` build first.

Use `-NoDisks -Safe 30` for the first boots. During any test: F8/F12 boot menu only, never save BIOS changes, holding the
power button is always safe, remove the stick and the PC boots as before.
If a BitLocker recovery screen ever appears: type the saved key.

## What the screen shows

* From the kernel's first line, the **boot log** is drawn on screen; when the
  desktop starts it continues in the Terminal window until the shell starts.
  If a boot freezes, the last lines show where.
* A crash shows the **crash screen** with the registers and the **last log
  lines**. A phone photo of it is a complete bug report.

## Hardware notes (main PC)

* **Local APIC mode:** handled whether the firmware leaves xAPIC or x2APIC
  (tested both in QEMU: `build.ps1 -X2Apic`).
* **CPU start-up page:** chosen from the firmware's memory map instead of
  assuming 0x8000 is free (tested with it moved: `build.ps1 -ApPage 70`).
* **GPU framebuffer:** on this PC it is at 0xD0000000 (below 4 GiB). With
  Resizable BAR it can be far higher; NovaBoot now maps it wherever it is
  (up to 32 TiB).
* **Keyboard/mouse:** no USB hub support yet: use the rear USB ports on the
  motherboard (not front panel, monitor or dock), or a PS/2 keyboard.
* **32 threads:** supported (limit 64).
* This PC has an unresolved CPU Machine Check reset problem; a sudden reset
  may be that rather than Nova.
