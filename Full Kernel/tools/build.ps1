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

.PARAMETER Safe
  The safety net, for a first run on a real PC: "-Safe 30" resets the PC 30 seconds
  after a crash (a PS/2 key press stops the countdown); a boot that has not reached
  READY after 90 s, or a desktop frozen for 30 s, counts as a crash. "-Safe 30,90,45"
  sets crash, boot and desktop seconds. Nothing is written anywhere; the PC is only reset.
  Off by default (the QEMU tests rely on a crash screen that stays up).

.PARAMETER X2Apic
  TEST: the kernel switches the local APICs to x2APIC mode itself (what some
  firmware hands over), so QEMU exercises that code path.

.PARAMETER Password
  Asks for a login password (typed twice, never shown) and bakes only its salted
  PBKDF2-SHA-256 record into the kernel: the desktop then starts locked, even on a
  -NoDisks USB stick. The password itself is stored nowhere. A password set later on
  the lock screen (saved in /system/password on a Nova disk) takes precedence.

.PARAMETER TestPassword
  TEST: like -Password but takes the password as an argument (for tools\test.ps1).
  Never use it for a real password: it ends up in the shell history.

.PARAMETER ApPage
  TEST: put the other CPUs' start-up code on this page below 1 MiB (hex page
  number, e.g. 70 = 0x70000) instead of 0x8000, to prove it can live anywhere.
#>
param([switch]$DebugBuild, [string]$Fault = '', [switch]$NoDisks, [string]$Safe = '', [switch]$X2Apic, [string]$ApPage = '',
      [switch]$Password, [string]$TestPassword = '')

# The record Nova stores instead of a password (same format as crates/auth):
# nova-pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>
function New-PasswordRecord([byte[]]$secret) {
    $iterations = 100000
    $salt = [byte[]]::new(16)
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($salt)
    $kdf = [System.Security.Cryptography.Rfc2898DeriveBytes]::new($secret, $salt, $iterations, [System.Security.Cryptography.HashAlgorithmName]::SHA256)
    $hash = $kdf.GetBytes(32)
    $kdf.Dispose()
    [Array]::Clear($secret, 0, $secret.Length)
    $hex = { param($b) -join ($b | ForEach-Object { $_.ToString('x2') }) }
    "nova-pbkdf2-sha256`$$iterations`$$(& $hex $salt)`$$(& $hex $hash)"
}

function Read-NewPassword {
    $plain = { param($s) [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($s)) }
    while ($true) {
        $a = & $plain (Read-Host -AsSecureString 'Nova login password')
        $b = & $plain (Read-Host -AsSecureString 'Type it again')
        if (-not $a) { Write-Host 'Empty; try again.' -ForegroundColor Yellow; continue }
        if ($a -ceq $b) { return $a }
        Write-Host 'They did not match; try again.' -ForegroundColor Yellow
    }
}

$passwordRecord = ''
if ($Password -or $TestPassword) {
    $pw = if ($TestPassword) { $TestPassword } else { Read-NewPassword }
    $passwordRecord = New-PasswordRecord ([Text.Encoding]::UTF8.GetBytes($pw))
    Remove-Variable pw
}

$buildFlags = @{ NOVA_FAULT = $Fault; NOVA_NO_DISKS = $(if ($NoDisks) { '1' } else { '' }); NOVA_SAFE = $Safe; NOVA_X2APIC = $(if ($X2Apic) { '1' } else { '' }); NOVA_AP_PAGE = $ApPage; NOVA_PASSWORD = $passwordRecord }

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
    if ($Safe) { Write-Host "SAFETY NET on ($Safe): a crash or a stuck boot/desktop resets the PC after the countdown" -ForegroundColor Yellow }
    if ($X2Apic) { Write-Host 'TEST KERNEL: switches to x2APIC mode' -ForegroundColor Yellow }
    if ($ApPage) { Write-Host "TEST KERNEL: CPU start-up code on page 0x$ApPage" -ForegroundColor Yellow }
    if ($passwordRecord) { Write-Host 'LOGIN: the desktop starts locked (password record built in)' -ForegroundColor Yellow }
    cargo build @cargoFlags -p nova-kernel --target x86_64-unknown-none
    if ($LASTEXITCODE -ne 0) { throw 'Kernel build failed' }

    $esp = Join-Path $root 'build\esp'
    New-Item -ItemType Directory -Force -Path (Join-Path $esp 'EFI\BOOT'), (Join-Path $esp 'NOVA') | Out-Null
    Copy-Item -Force "target\x86_64-unknown-uefi\$buildProfile\nova-boot.efi" (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI')
    python -I tools\pefix.py (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI')
    if ($LASTEXITCODE -ne 0) { throw 'pefix failed' }
    Copy-Item -Force "target\x86_64-unknown-none\$buildProfile\nova-kernel" (Join-Path $esp 'NOVA\KERNEL.ELF')

    Write-Host "Boot volume ready: $esp ($buildProfile)"
}
finally {
    foreach ($name in $buildFlags.Keys) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
    Pop-Location
}
