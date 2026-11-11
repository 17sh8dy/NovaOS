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

    # Stage 2: for each deliberate fault, build a test kernel and check the
    # crash report names the right exception (and the right stack, for #DF).
    $faults = @(
        @{ Kind = 'div';   Expect = @('NOVA KERNEL EXCEPTION', '#DE divide error') },
        @{ Kind = 'ud';    Expect = @('NOVA KERNEL EXCEPTION', '#UD invalid opcode') },
        @{ Kind = 'gp';    Expect = @('NOVA KERNEL EXCEPTION', '#GP general protection fault') },
        @{ Kind = 'pf';    Expect = @('NOVA KERNEL EXCEPTION', '#PF page fault', '0xffffc00000000000', 'page not present, read') },
        @{ Kind = 'df';    Expect = @('NOVA KERNEL EXCEPTION', '#DF double fault', 'handler ran on IST 1') },
        @{ Kind = 'stack'; Expect = @('NOVA KERNEL EXCEPTION', '#DF double fault', 'handler ran on IST 1') },
        @{ Kind = 'nmi';   Expect = @('NOVA KERNEL EXCEPTION', 'NMI non-maskable interrupt', 'handler ran on IST 2') },
        @{ Kind = 'panic'; Expect = @('NOVA KERNEL PANIC', 'deliberate panic') },
        @{ Kind = 'null';  Expect = @('#PF page fault', '0x0000000000000000', 'page not present, read') },
        @{ Kind = 'lowmem'; Expect = @('#PF page fault', '0x0000000000100000', 'page not present, read') },
        @{ Kind = 'ro';    Expect = @('#PF page fault', 'protection violation, write') },
        @{ Kind = 'nx';    Expect = @('#PF page fault', 'instruction fetch') },
        @{ Kind = 'stray'; Expect = @('irq: stray interrupt on vector 0x81', 'NOVA KERNEL READY') },
        @{ Kind = 'tdiv';  Expect = @('NOVA KERNEL EXCEPTION', '#DE divide error', 'on CPU', ', thread ') },
        @{ Kind = 'tstack'; Expect = @('NOVA KERNEL EXCEPTION', '#DF double fault', 'handler ran on IST 1', ', thread ') }
    )
    try {
        foreach ($f in $faults) {
            Step "fault: $($f.Kind)" {
                & (Join-Path $PSScriptRoot 'build.ps1') -Fault $f.Kind
                [string[]]$a = @('-I', 'tools\boot_test.py', '--shot', "fault_$($f.Kind)")
                foreach ($e in $f.Expect) { $a += @('--expect', $e) }
                python @a
            }
        }
    }
    finally {
        & (Join-Path $PSScriptRoot 'build.ps1')   # back to the normal kernel
    }
    Step 'normal boot + mouse (move, left click, right click)' { python -I tools\boot_test.py --mouse }

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
