<#
.SYNOPSIS
  Builds NovaBoot + the Nova kernel and lays out a UEFI boot volume in build\esp.

.DESCRIPTION
  build\esp\EFI\BOOT\BOOTX64.EFI  <- NovaBoot (the default UEFI boot path)
  build\esp\NOVA\KERNEL.ELF       <- the kernel NovaBoot loads

  The esp folder is exactly what a FAT32 USB stick needs to contain to boot
  on a real UEFI PC (with Secure Boot off).

.PARAMETER DebugBuild
  Build without optimisations.

.PARAMETER Fault
  Build a TEST kernel that triggers a deliberate fault after the boot screen:
  div, ud, gp, pf, df, stack, nmi or panic (see kernel\src\faults.rs). Use it to
  see the crash screen; run tools\build.ps1 again without it for a normal kernel.

.PARAMETER NoDisks
  A kernel that never starts the SATA/NVMe drivers (no disk controller is
  touched at all). Meant for the first boot on a real PC.

.PARAMETER X2Apic
  TEST: the kernel switches the local APICs to x2APIC mode itself (what some
  firmware hands over), so QEMU exercises that code path.

.PARAMETER ApPage
  TEST: put the other CPUs' start-up code on this page below 1 MiB (hex page
  number, e.g. 70 = 0x70000) instead of 0x8000, to prove it can live anywhere.
#>
param([switch]$DebugBuild, [string]$Fault = '', [switch]$NoDisks, [switch]$X2Apic, [string]$ApPage = '')

$buildFlags = @{ NOVA_FAULT = $Fault; NOVA_NO_DISKS = $(if ($NoDisks) { '1' } else { '' }); NOVA_X2APIC = $(if ($X2Apic) { '1' } else { '' }); NOVA_AP_PAGE = $ApPage }

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$buildProfile = if ($DebugBuild) { 'debug' } else { 'release' }
[string[]]$cargoFlags = if ($DebugBuild) { @() } else { @('--release') }

Push-Location $root
try {
    & (Join-Path $PSScriptRoot 'build_user.ps1')   # the kernel embeds the user programs
    cargo build @cargoFlags -p nova-boot --target x86_64-unknown-uefi
    if ($LASTEXITCODE -ne 0) { throw 'NovaBoot (bootloader) build failed' }
    foreach ($name in $buildFlags.Keys) {
        if ($buildFlags[$name]) { Set-Item "Env:$name" $buildFlags[$name] } else { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
    }
    if ($Fault) { Write-Host "TEST KERNEL: it will fault on purpose ($Fault)" -ForegroundColor Yellow }
    if ($NoDisks) { Write-Host 'NO-DISKS kernel: the SATA/NVMe drivers will not start' -ForegroundColor Yellow }
    if ($X2Apic) { Write-Host 'TEST KERNEL: switches to x2APIC mode' -ForegroundColor Yellow }
    if ($ApPage) { Write-Host "TEST KERNEL: CPU start-up code on page 0x$ApPage" -ForegroundColor Yellow }
    cargo build @cargoFlags -p nova-kernel --target x86_64-unknown-none
    if ($LASTEXITCODE -ne 0) { throw 'Kernel build failed' }

    $esp = Join-Path $root 'build\esp'
    New-Item -ItemType Directory -Force -Path (Join-Path $esp 'EFI\BOOT'), (Join-Path $esp 'NOVA') | Out-Null
    Copy-Item -Force "target\x86_64-unknown-uefi\$buildProfile\nova-boot.efi" (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI')
    Copy-Item -Force "target\x86_64-unknown-none\$buildProfile\nova-kernel" (Join-Path $esp 'NOVA\KERNEL.ELF')

    Write-Host "Boot volume ready: $esp ($buildProfile)"
}
finally {
    foreach ($name in $buildFlags.Keys) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
    Pop-Location
}
