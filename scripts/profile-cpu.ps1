#Requires -Version 5.1
<#
  CPU profile run: wraps the release binary with samply, a sampling profiler
  that produces Gecko/Firefox Profiler format. Install samply once with
  `cargo install samply` (this script will do it for you if missing).

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

if (-not (Get-Command samply -ErrorAction SilentlyContinue)) {
    Write-Host "==> samply not found. Installing via cargo..." -ForegroundColor Cyan
    cargo install samply
    if ($LASTEXITCODE -ne 0) { throw "cargo install samply failed" }
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Write-Host "==> cargo build --release" -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$bin = Join-Path $repoRoot "target/release/terminal-manager.exe"
if (-not (Test-Path $bin)) { throw "binary not found at $bin" }

$cpuFile = Join-Path $OutDir "cpu.json.gz"
if (Test-Path $cpuFile) { Remove-Item $cpuFile }

Write-Host ""
Write-Host "==> samply record --save-only --output $cpuFile --rate $Rate -- $bin" -ForegroundColor Cyan
Write-Host "    Exercise the UI, then close the window to stop recording."
Write-Host ""

samply record --save-only --output $cpuFile --rate $Rate -- $bin

Write-Host ""
if (Test-Path $cpuFile) {
    $size = (Get-Item $cpuFile).Length
    Write-Host ("==> CPU profile written: {0} ({1:N0} bytes)" -f $cpuFile, $size) -ForegroundColor Green
    Write-Host "    Open scripts/profile.html (CPU card) and drop the file on the viewer."
} else {
    Write-Warning "Expected $cpuFile but the file was not produced."
}
