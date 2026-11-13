#!/usr/bin/env bash
# Builds NovaBoot + the Nova kernel into build/esp (Linux version of build.ps1).
#
#   tools/build.sh                 normal build
#   tools/build.sh --no-disks      the SATA/NVMe drivers never start (first real-PC boots)
#   tools/build.sh --x2apic        TEST: switch the local APICs to x2APIC mode
#   tools/build.sh --ap-page 70    TEST: CPU start-up code on page 0x70 (0x70000)
#   tools/build.sh --fault KIND    TEST: fault on purpose (see kernel/src/faults.rs)
set -euo pipefail
cd "$(dirname "$0")/.."

unset NOVA_FAULT NOVA_NO_DISKS NOVA_X2APIC NOVA_AP_PAGE
while [ $# -gt 0 ]; do
    case "$1" in
        --no-disks) export NOVA_NO_DISKS=1 ;;
        --x2apic) export NOVA_X2APIC=1 ;;
        --ap-page) export NOVA_AP_PAGE="$2"; shift ;;
        --fault) export NOVA_FAULT="$2"; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

# User programs: linked at 0x400000 with the normal code model, so they get their
# own RUSTFLAGS (.cargo/config.toml sets the kernel code model for this target).
RUSTFLAGS='-C relocation-model=static' cargo build --release -p nova-user \
    --target x86_64-unknown-none --target-dir target/user --config 'profile.release.debug=0'
mkdir -p kernel/user_bin
for f in target/user/x86_64-unknown-none/release/*; do
    name=$(basename "$f")
    if [ -f "$f" ] && [ -x "$f" ] && [[ "$name" != *.* ]]; then
        cp -f "$f" "kernel/user_bin/$name.elf"
    fi
done

cargo build --release -p nova-boot --target x86_64-unknown-uefi
cargo build --release -p nova-kernel --target x86_64-unknown-none

mkdir -p build/esp/EFI/BOOT build/esp/NOVA
cp -f target/x86_64-unknown-uefi/release/nova-boot.efi build/esp/EFI/BOOT/BOOTX64.EFI
cp -f target/x86_64-unknown-none/release/nova-kernel build/esp/NOVA/KERNEL.ELF
[ -n "${NOVA_NO_DISKS:-}" ] && echo "NO-DISKS kernel: the SATA/NVMe drivers will not start"
echo "Boot volume ready: build/esp"
