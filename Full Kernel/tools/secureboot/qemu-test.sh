#!/usr/bin/env bash
# Secure Boot test in QEMU (run on Linux, e.g. the Dell): OVMF with Secure
# Boot ENFORCING and Microsoft's keys enrolled (like a real PC), booting the
# stick from make-stick.sh. Three checks:
#
#   1. without our key enrolled: shim must refuse NovaBoot
#   2. with our key enrolled (as MokManager would do): Nova boots, and
#      NovaBoot reports "Secure Boot: ON"
#   3. with NovaBoot tampered with (one byte changed): refused again
#
# Needs: qemu-system-x86, ovmf, python3-virt-firmware (for virt-fw-vars).
set -euo pipefail
cd "$(dirname "$0")/../.."
keys="${NOVA_KEY_DIR:-$HOME/.nova-keys}"
stick=build/secureboot-esp
work=build/sb-test
code=/usr/share/OVMF/OVMF_CODE_4M.secboot.fd
msvars=/usr/share/OVMF/OVMF_VARS_4M.ms.fd

for f in "$code" "$msvars"; do [ -f "$f" ] || { echo "missing $f: sudo apt install ovmf"; exit 1; }; done
command -v virt-fw-vars >/dev/null || { echo "virt-fw-vars missing: sudo apt install python3-virt-firmware"; exit 1; }
[ -f "$stick/EFI/BOOT/grubx64.efi" ] || { echo "run tools/secureboot/make-stick.sh first"; exit 1; }
accel=tcg
[ -w /dev/kvm ] && accel=kvm
# Shim's vendor GUID for MokList entries (any GUID works as the owner of a cert).
owner=605dab50-e046-4300-abb6-3dd810dd8b23

rm -rf "$work"
mkdir -p "$work"

boot() { # $1 = name, $2 = vars file, $3 = esp dir; prints the serial log path
    local log="$work/$1.log"
    timeout 90 qemu-system-x86_64 -machine q35,smm=on -accel "$accel" -m 512M -smp 2 \
        -global driver=cfi.pflash01,property=secure,value=on \
        -drive if=pflash,format=raw,unit=0,readonly=on,file="$code" \
        -drive if=pflash,format=raw,unit=1,file="$2" \
        -drive format=raw,file=fat:rw:"$3" \
        -nic none -display none -serial file:"$log" -no-reboot &
    local pid=$!
    for _ in $(seq 1 85); do
        sleep 1
        if grep -qa "NOVA KERNEL READY\|cannot start Nova\|Verification failed\|Security Violation\|MokManager\|Shim UEFI key management" "$log" 2>/dev/null; then
            sleep 2
            break
        fi
    done
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "$log"
}

fail=0
check() { # $1 = description, $2 = log, $3 = must-match regex, $4 = must-not-match regex
    if grep -qaE "$3" "$2" && ! grep -qaE "$4" "$2"; then
        echo "PASS: $1"
    else
        echo "FAIL: $1 (see $2)"
        fail=1
    fi
}

# 1. Microsoft keys only: shim runs (it is Microsoft-signed) but must not run NovaBoot.
cp "$msvars" "$work/vars-nomok.fd"
log=$(boot nomok "$work/vars-nomok.fd" "$stick")
check "without our key, shim refuses NovaBoot" "$log" "Verification failed|Security Violation|MokManager|Shim UEFI key management|Failed to load" "NovaBoot [0-9]"

# 2. Our certificate in MokList (what MokManager's "Enroll key from disk" does).
if virt-fw-vars --help 2>&1 | grep -q -- "--add-mok"; then
    virt-fw-vars --input "$msvars" --output "$work/vars-mok.fd" --add-mok "$owner" "$keys/nova-mok.crt" >/dev/null
    how="MokList (as MokManager would)"
else
    # Older virt-firmware: put it in db instead (shim also trusts db).
    virt-fw-vars --input "$msvars" --output "$work/vars-mok.fd" --add-db "$owner" "$keys/nova-mok.crt" >/dev/null
    how="db (this virt-fw-vars has no --add-mok)"
fi
log=$(boot mok "$work/vars-mok.fd" "$stick")
check "with our key in $how, Nova boots with Secure Boot ON" "$log" "Secure Boot: ON" "cannot start Nova|Verification failed"
check "... all the way to the kernel" "$log" "NOVA KERNEL READY" "NOVA KERNEL PANIC"

# 3. A NovaBoot that was changed after signing must be refused.
cp -r "$stick" "$work/tampered-esp"
python3 - "$work/tampered-esp/EFI/BOOT/grubx64.efi" <<'PY'
import sys
p = sys.argv[1]
d = bytearray(open(p, "rb").read())
d[0x1100] ^= 0xFF   # a byte inside .text
open(p, "wb").write(d)
PY
cp "$work/vars-mok.fd" "$work/vars-tamper.fd"
log=$(boot tampered "$work/vars-tamper.fd" "$work/tampered-esp")
check "a tampered NovaBoot is refused" "$log" "Verification failed|Security Violation|MokManager|Shim UEFI key management|Failed to load" "NovaBoot [0-9]"

exit $fail
