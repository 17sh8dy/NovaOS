#!/usr/bin/env bash
# Lays out a Secure-Boot USB stick for Nova in build/secureboot-esp:
#
#   EFI/BOOT/BOOTX64.EFI   shim, signed by Microsoft (from Ubuntu's shim-signed package)
#   EFI/BOOT/mmx64.efi     MokManager (signed by Canonical; shim trusts it)
#   EFI/BOOT/grubx64.efi   NovaBoot, signed with our MOK (shim loads this name)
#   NOVA/KERNEL.ELF        the kernel
#   nova-mok.der           our public certificate, to enrol from the stick
#
# Deliberately NOT included: fbx64.efi (shim's "fallback", which would create
# firmware boot entries). Nothing on this stick writes boot entries.
#
# First boot on a PC: shim can't verify NovaBoot yet and opens MokManager:
# "Enroll key from disk" -> the stick -> nova-mok.der -> Continue -> Yes ->
# Reboot. After that Nova boots with Secure Boot on. (On a Linux PC you can
# instead run: sudo mokutil --import nova-mok.der, then reboot and confirm.)
#
# Needs: build/esp (tools/build.sh), sbsigntool, shim-signed, the key from make-key.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
keys="${NOVA_KEY_DIR:-$HOME/.nova-keys}"
out=build/secureboot-esp

[ -f build/esp/EFI/BOOT/BOOTX64.EFI ] || { echo "build/esp is missing: run tools/build.sh first"; exit 1; }
[ -f "$keys/nova-mok.key" ] || { echo "no key: run tools/secureboot/make-key.sh first"; exit 1; }
command -v sbsign >/dev/null || { echo "sbsign missing: sudo apt install sbsigntool"; exit 1; }

# Ubuntu's signed shim (the newest one the package offers) and MokManager.
shim=""
for c in /usr/lib/shim/shimx64.efi.signed.latest /usr/lib/shim/shimx64.efi.signed /usr/lib/shim/shimx64.efi.signed.previous; do
    [ -f "$c" ] && { shim="$c"; break; }
done
mm=""
for c in /usr/lib/shim/mmx64.efi.signed /usr/lib/shim/mmx64.efi; do
    [ -f "$c" ] && { mm="$c"; break; }
done
[ -n "$shim" ] && [ -n "$mm" ] || { echo "shim not found: sudo apt install shim-signed"; exit 1; }

rm -rf "$out"
mkdir -p "$out/EFI/BOOT" "$out/NOVA"
cp "$shim" "$out/EFI/BOOT/BOOTX64.EFI"
cp "$mm" "$out/EFI/BOOT/mmx64.efi"
sbsign --key "$keys/nova-mok.key" --cert "$keys/nova-mok.crt" \
    --output "$out/EFI/BOOT/grubx64.efi" build/esp/EFI/BOOT/BOOTX64.EFI
sbverify --cert "$keys/nova-mok.crt" "$out/EFI/BOOT/grubx64.efi"
cp build/esp/NOVA/KERNEL.ELF "$out/NOVA/KERNEL.ELF"
cp "$keys/nova-mok.der" "$out/nova-mok.der"

echo
echo "Secure Boot stick layout ready in $out (shim: $shim)"
find "$out" -type f | sort
echo
echo "To write a USB stick (FAT32, ALL DATA ON THE STICK IS LOST), copy the"
echo "contents of $out to the root of the stick. Double-check the device name."
