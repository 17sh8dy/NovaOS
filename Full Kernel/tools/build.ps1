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
#>
param([switch]$DebugBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$buildProfile = if ($DebugBuild) { 'debug' } else { 'release' }
[string[]]$cargoFlags = if ($DebugBuild) { @() } else { @('--release') }

Push-Location $root
try {
    cargo build @cargoFlags -p nova-boot --target x86_64-unknown-uefi
    if ($LASTEXITCODE -ne 0) { throw 'NovaBoot (bootloader) build failed' }
    cargo build @cargoFlags -p nova-kernel --target x86_64-unknown-none
    if ($LASTEXITCODE -ne 0) { throw 'Kernel build failed' }

    $esp = Join-Path $root 'build\esp'
    New-Item -ItemType Directory -Force -Path (Join-Path $esp 'EFI\BOOT'), (Join-Path $esp 'NOVA') | Out-Null
    Copy-Item -Force "target\x86_64-unknown-uefi\$buildProfile\nova-boot.efi" (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI')
    Copy-Item -Force "target\x86_64-unknown-none\$buildProfile\nova-kernel" (Join-Path $esp 'NOVA\KERNEL.ELF')

    Write-Host "Boot volume ready: $esp ($buildProfile)"
}
finally {
    Pop-Location
}
