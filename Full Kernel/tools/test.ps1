<#
.SYNOPSIS
  The full check: unit tests, clippy, build, and real boots in QEMU.

.DESCRIPTION
  1. cargo test       - ELF parser, page-table builder, boot protocol (host)
  2. cargo clippy     - libraries, bootloader (UEFI target), kernel (bare metal)
  3. tools\build.ps1  - builds build\esp
  4. boot tests       - 256 MiB, 6 GiB (RAM above 4 GiB), and a boot with
                        the kernel file missing (must fail cleanly)
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
$failed = @()
function Step($name, [scriptblock]$body) {
    Write-Host "`n=== $name" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -ne 0) { $script:failed += $name; Write-Host "FAILED: $name" -ForegroundColor Red }
}
try {
    Step 'unit tests' { cargo test }
    Step 'clippy (libraries)' { cargo clippy --all-targets -- -D warnings }
    Step 'clippy (bootloader)' { cargo clippy --release -p nova-boot --target x86_64-unknown-uefi -- -D warnings }
    Step 'clippy (kernel)' { cargo clippy --release -p nova-kernel --target x86_64-unknown-none -- -D warnings }
    Step 'build' { & (Join-Path $PSScriptRoot 'build.ps1'); $global:LASTEXITCODE = 0 }
    Step 'boot 256M' { python -I tools\boot_test.py --memory 256M }
    Step 'boot 6G' { python -I tools\boot_test.py --memory 6G }

    $kernel = 'build\esp\NOVA\KERNEL.ELF'
    Move-Item $kernel "$kernel.hold" -Force
    try {
        Step 'missing kernel fails cleanly' { python -I tools\boot_test.py --expect 'Returning to the firmware' }
    }
    finally {
        Move-Item "$kernel.hold" $kernel -Force
    }
}
finally {
    Pop-Location
}

if ($failed.Count) {
    Write-Host "`n$($failed.Count) step(s) failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "`nAll checks passed." -ForegroundColor Green
