<#
.SYNOPSIS
  Builds Nova and boots it in a QEMU window. Serial output (bootloader +
  kernel log) appears in this terminal. Close the QEMU window to stop.

.PARAMETER NoBuild
  Boot whatever is already in build\esp.

.PARAMETER Memory
  Guest RAM, e.g. 256M or 2G (default 256M).

.PARAMETER Cpus
  Virtual CPUs (default 4).
#>
param([switch]$NoBuild, [string]$Memory = '256M', [int]$Cpus = 4)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $NoBuild) { & (Join-Path $PSScriptRoot 'build.ps1') }

# The argument list comes from tools\qemu.py so run and test boot the same machine.
$py = @"
import sys, json
sys.path.insert(0, r'$PSScriptRoot')
import qemu as q
print(json.dumps(q.base_args(q.find_qemu(), '$Memory', $Cpus)))
"@
$qemuArgs = python -c $py | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare QEMU' }

Push-Location (Join-Path $root 'build')
try {
    $exe = $qemuArgs[0]
    $rest = @($qemuArgs | Select-Object -Skip 1) + @('-serial', 'stdio', '-name', 'Nova OS')
    & $exe @rest
}
finally {
    Pop-Location
}
