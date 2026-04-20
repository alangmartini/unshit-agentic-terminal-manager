#Requires -Version 5.1
<#
  Heap profile run: builds terminal-manager with the `profiling` cargo feature
  (dhat global allocator) and launches the app. When the app exits normally,
  dhat writes `target/profile/dhat-heap.json` with allocation backtraces.

  Load the JSON in the dashboard at scripts/profile.html, or directly at
  https://nnethercote.github.io/dh_view/dh_view.html.
#>

param(
    [string]$OutDir = "target/profile"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Write-Host "==> cargo build --release --features profiling" -ForegroundColor Cyan
cargo build --release --features profiling
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$bin = Join-Path $repoRoot "target/release/terminal-manager.exe"
if (-not (Test-Path $bin)) { throw "binary not found at $bin" }

$heapFile = Join-Path $OutDir "dhat-heap.json"
if (Test-Path $heapFile) { Remove-Item $heapFile }

Write-Host ""
Write-Host "==> Launching app with dhat heap profiling." -ForegroundColor Cyan
Write-Host "    Exercise the UI, then close the window to flush the profile."
Write-Host ""

& $bin

Write-Host ""
if (Test-Path $heapFile) {
    $size = (Get-Item $heapFile).Length
    Write-Host ("==> Heap profile written: {0} ({1:N0} bytes)" -f $heapFile, $size) -ForegroundColor Green
    Write-Host "    Open scripts/profile.html (Memory card) and drop the file on the viewer."
} else {
    Write-Warning "Expected $heapFile but the file was not produced."
    Write-Warning "Make sure the app closed via the window close button or Ctrl+C (not kill)."
}
