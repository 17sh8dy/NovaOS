<#
.SYNOPSIS
  Builds the user-space programs (user\src\bin) and copies them to
  kernel\user_bin, where the kernel embeds them.

.DESCRIPTION
  User programs are linked at 0x400000 with the normal (small) code model, so
  they are built with their own RUSTFLAGS instead of the kernel's
  (.cargo\config.toml uses the kernel code model for x86_64-unknown-none).
  Run this before building or checking the kernel; build.ps1 and test.ps1 do.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $env:RUSTFLAGS = '-C relocation-model=static'
    cargo build --release -p nova-user --target x86_64-unknown-none --target-dir target\user --config 'profile.release.debug=0'
    if ($LASTEXITCODE -ne 0) { throw 'user programs failed to build' }
    $out = Join-Path $root 'kernel\user_bin'
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $bins = Get-ChildItem 'target\user\x86_64-unknown-none\release' -File | Where-Object { $_.Extension -eq '' -and $_.Name -notmatch '^(build|deps)' }
    foreach ($b in $bins) { Copy-Item -Force $b.FullName (Join-Path $out ($b.Name + '.elf')) }
    Write-Host "User programs: $(($bins | ForEach-Object Name) -join ', ')"
}
finally {
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    Pop-Location
}
