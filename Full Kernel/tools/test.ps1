<#
.SYNOPSIS
  The full check: unit tests, clippy, build, and real boots in QEMU.

.DESCRIPTION
  1. cargo test       - ELF parser, page-table builder, boot protocol (host)
  2. cargo clippy     - libraries, bootloader (UEFI target), kernel (bare metal)
  3. tools\build.ps1  - builds build\esp
  4. boot tests       - 256 MiB, 6 GiB (RAM above 4 GiB), deliberate faults,
                        SATA + NVMe storage (with a reboot), PS/2 and USB input,
                        and a boot with
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
    Step 'user programs' { & (Join-Path $PSScriptRoot 'build_user.ps1'); $global:LASTEXITCODE = 0 }
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
    Step 'storage 1: format, install, write files' { python -I tools\boot_test.py --fresh-disk --disk-phase 1 }
    Step 'storage 2: reboot, files persist' { python -I tools\boot_test.py --disk-phase 2 }
    Step 'NVMe 1: format, install, write files' { python -I tools\boot_test.py --fresh-disk --disk-phase 1 --disk-kind nvme }
    Step 'NVMe 2: reboot, files persist' { python -I tools\boot_test.py --disk-phase 2 --disk-kind nvme }
    Step 'NVMe with 4 KiB sectors 1' { python -I tools\boot_test.py --fresh-disk --disk-phase 1 --disk-kind nvme --nvme-block 4096 }
    Step 'NVMe with 4 KiB sectors 2' { python -I tools\boot_test.py --disk-phase 2 --disk-kind nvme --nvme-block 4096 }
    Step 'normal boot + mouse + keyboard/shell + desktop' { python -I tools\boot_test.py --fresh-disk --mouse --shell --desktop --user --shot desktop }
    Step 'USB keyboard + mouse, no PS/2 (like a laptop), desktop' { python -I tools\boot_test.py --fresh-disk --no-ps2 --usb --mouse --shell --desktop --user --shot usb }
    Step 'USB at 6G (DMA above 4 GiB)' { python -I tools\boot_test.py --memory 6G --usb --mouse --shell }

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
