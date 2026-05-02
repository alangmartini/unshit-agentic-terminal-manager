#Requires -Version 5.1
<#
  Kill the terminal-manager UI and the unshit-ptyd session daemon.

  The daemon outlives the UI by design (sessions survive restarts), so the
  OS close button only stops the UI and the daemon keeps running with
  every previous PTY attached. When you need a clean slate (e.g. after
  changing the default shell, after a daemon crash, or before re-running
  `cargo run --release` to pick up new daemon code), call this script.
#>

[CmdletBinding()]
param(
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"

$targets = @("terminal-manager", "unshit-ptyd")
$killed = 0

foreach ($name in $targets) {
    $procs = Get-Process -Name $name -ErrorAction SilentlyContinue
    if (-not $procs) {
        if (-not $Quiet) { Write-Host "no $name running" }
        continue
    }
    foreach ($p in $procs) {
        try {
            Stop-Process -Id $p.Id -Force -ErrorAction Stop
            $killed++
            if (-not $Quiet) { Write-Host "killed $name pid=$($p.Id)" }
        } catch {
            Write-Warning "failed to kill $name pid=$($p.Id): $($_.Exception.Message)"
        }
    }
}

if (-not $Quiet) { Write-Host "done ($killed process(es) killed)" }
