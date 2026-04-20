#Requires -Version 5.1
<#
  Thin wrapper. The real implementation lives in xtask/ and is invoked via
  `cargo xtask profile memory` or the `cargo profile-memory` alias.

  Output: target/profile/dhat-heap.json
  Load the JSON at scripts/profile.html or https://nnethercote.github.io/dh_view/dh_view.html.
#>

param(
    [string]$OutDir = "target/profile"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

cargo xtask profile memory --out-dir $OutDir
if ($LASTEXITCODE -ne 0) { throw "cargo xtask profile memory failed" }
