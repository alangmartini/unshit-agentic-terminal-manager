#Requires -Version 5.1
<#
  Thin wrapper. The real implementation lives in xtask/ and is invoked via
  `cargo xtask profile cpu` or the `cargo profile-cpu` alias.

  Output: target/profile/cpu.json.gz
  Load at scripts/profile.html (CPU card) or https://profiler.firefox.com.
#>

param(
    [string]$OutDir = "target/profile",
    [int]$Rate = 1000
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

cargo xtask profile cpu --out-dir $OutDir --rate $Rate
if ($LASTEXITCODE -ne 0) { throw "cargo xtask profile cpu failed" }
