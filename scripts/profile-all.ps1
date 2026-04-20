#Requires -Version 5.1
<#
  Thin wrapper. The real implementation lives in xtask/ and is invoked via
  `cargo xtask profile all` or the `cargo profile-all` alias.
#>

param(
    [string]$OutDir = "target/profile"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

cargo xtask profile all --out-dir $OutDir
if ($LASTEXITCODE -ne 0) { throw "cargo xtask profile all failed" }
