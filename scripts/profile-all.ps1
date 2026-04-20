#Requires -Version 5.1
<#
  Convenience wrapper: runs the CPU profile pass and the memory profile pass
  back to back, then opens the dashboard in the default browser. Each pass
  launches the app; use the UI for a representative workload, then close the
  window to advance to the next pass.
#>

param(
    [string]$OutDir = "target/profile"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

& (Join-Path $PSScriptRoot "profile-cpu.ps1") -OutDir $OutDir
& (Join-Path $PSScriptRoot "profile-memory.ps1") -OutDir $OutDir

$dash = Join-Path $PSScriptRoot "profile.html"
Write-Host ""
Write-Host "==> Opening $dash" -ForegroundColor Cyan
Start-Process $dash
