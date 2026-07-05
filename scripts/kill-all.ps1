#Requires -Version 5.1
<#
  Kill terminal-manager UI and unshit-ptyd session daemon processes.

  By default this is REPO-SCOPED: it only stops processes whose
  executable lives inside this repository (target\, target-*\ build
  dirs). The installed Terminal Manager and its daemon — your real,
  persistent session — are never touched.

  Use -All to also stop the installed app and every other instance
  regardless of where it runs from.

  The daemon outlives the UI by design (sessions survive restarts), so
  the OS close button only stops the UI and the daemon keeps running
  with every previous PTY attached. When you need a clean dev slate
  (e.g. after changing the default shell, after a daemon crash, or
  before re-running `cargo run --release` to pick up new daemon code),
  call this script.
#>

[CmdletBinding()]
param(
    [switch]$Quiet,
    [switch]$All
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$targets = @("terminal-manager.exe", "unshit-ptyd.exe")
$killed = 0
$spared = 0

foreach ($name in $targets) {
    $procs = @(Get-CimInstance Win32_Process -Filter "Name='$name'" -ErrorAction SilentlyContinue)
    if (-not $procs) {
        if (-not $Quiet) { Write-Host "no $name running" }
        continue
    }
    foreach ($p in $procs) {
        $exePath = $p.ExecutablePath
        $inRepo = $exePath -and $exePath.StartsWith("$repoRoot\", [System.StringComparison]::OrdinalIgnoreCase)
        if (-not $All -and -not $inRepo) {
            $spared++
            if (-not $Quiet) {
                $shown = if ($exePath) { $exePath } else { "<unknown path>" }
                Write-Host "spared $name pid=$($p.ProcessId) ($shown; not a repo build — use -All to include)"
            }
            continue
        }
        try {
            Stop-Process -Id $p.ProcessId -Force -ErrorAction Stop
            $killed++
            if (-not $Quiet) { Write-Host "killed $name pid=$($p.ProcessId) ($exePath)" }
        } catch {
            Write-Warning "failed to kill $name pid=$($p.ProcessId): $($_.Exception.Message)"
        }
    }
}

if (-not $Quiet) {
    $sparedNote = if ($spared -gt 0) { ", $spared spared" } else { "" }
    Write-Host "done ($killed process(es) killed$sparedNote)"
}
