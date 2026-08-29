#Requires -Version 5.1
<#
.SYNOPSIS
  Measure how much of GPU bring-up the prewarm actually hides.

.DESCRIPTION
  Comparing two benchmark runs cannot answer this honestly: adapter and device
  creation vary by hundreds of milliseconds between launches on this hardware,
  so a before/after difference is mostly drift.

  The app reports both halves of the answer within a single run instead:

    renderer.prewarm_ready   total_ms   how long GPU bring-up took
    renderer.prewarm_used    waited_ms  how long the event-loop thread waited

  The difference is the part that happened underneath other startup work --
  time the old code spent with the event-loop thread blocked. Both numbers come
  from the same launch, so machine load cancels out.

  Also reports the stage table, so the saving can be read against the whole.
#>
[CmdletBinding()]
param(
    [int]$Iterations = 5,
    [string]$ExeDir,
    [int]$SettleMs = 3000
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
$workDir = Join-Path $repoRoot 'target\startup-overlap'
New-Item -ItemType Directory -Force -Path $workDir | Out-Null

$launched = $null
$isolation = Enter-TmIsolation -Tag 'overlap'
$env:RUST_LOG = 'info'
$env:TM_STARTUP_TRACE = '1'
$rows = New-Object System.Collections.Generic.List[object]

try {
    $realLayout = Join-Path $env:APPDATA 'com.godly.terminal\workspaces.json'
    if (Test-Path $realLayout) {
        Copy-Item -LiteralPath $realLayout `
            -Destination (Join-Path $isolation.ConfigDir 'workspaces.json') -Force
    }

    for ($i = 1; $i -le $Iterations; $i++) {
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'SilentlyContinue'
        try { & $ptydExe --shutdown --force --socket $isolation.PipePath | Out-Null } catch {}
        $ErrorActionPreference = $prev
        $global:LASTEXITCODE = 0

        $errLog = Join-Path $workDir "run-$i.stderr.log"
        $proc = Start-Process -FilePath $appExe -PassThru -RedirectStandardError $errLog `
            -RedirectStandardOutput (Join-Path $workDir "run-$i.stdout.log")
        $launched = $proc
        Start-Sleep -Milliseconds $SettleMs
        try { if (-not $proc.HasExited) { $proc.Kill() } } catch {}
        try { $proc.WaitForExit(5000) | Out-Null } catch {}
        $launched = $null

        $text = Get-Content -LiteralPath $errLog -Raw -ErrorAction SilentlyContinue
        if (-not $text) { Write-Warning "run ${i}: no output"; continue }

        function Get-Json([string]$body, [string]$eventName) {
            $line = ($body -split "`n" | Where-Object { $_ -match [regex]::Escape($eventName) } |
                Select-Object -First 1)
            if (-not $line) { return $null }
            $idx = $line.IndexOf('{"event"')
            if ($idx -lt 0) { return $null }
            return $line.Substring($idx) | ConvertFrom-Json
        }

        $ready = Get-Json $text 'renderer.prewarm_ready'
        $used = Get-Json $text 'renderer.prewarm_used'
        $startup = Get-Json $text '"event":"app.startup"'

        if (-not $ready -or -not $used) {
            Write-Warning "run ${i}: prewarm did not run (discarded or unsupported)"
            continue
        }

        $stages = @{}
        if ($startup) { foreach ($s in $startup.stages) { $stages[$s.stage] = $s.at_ms } }

        $rows.Add([pscustomobject]@{
                Run        = $i
                GpuMs      = [math]::Round($ready.total_ms, 1)
                WaitedMs   = [math]::Round($used.waited_ms, 1)
                HiddenMs   = [math]::Round($ready.total_ms - $used.waited_ms, 1)
                WindowMs   = if ($stages.ContainsKey('first_layout_done')) { [math]::Round($stages['first_layout_done'], 1) } else { $null }
                FirstFrame = if ($startup) { [math]::Round($startup.total_ms, 1) } else { $null }
            })
        Write-Host ("run {0}: gpu {1,7:N1} ms, waited {2,7:N1} ms, hidden {3,7:N1} ms, first frame {4,7:N1} ms" -f `
                $i, $ready.total_ms, $used.waited_ms, ($ready.total_ms - $used.waited_ms), $startup.total_ms)
    }

    if ($rows.Count -gt 0) {
        $hidden = ($rows | ForEach-Object { $_.HiddenMs } | Sort-Object)
        $median = $hidden[[int]([math]::Floor($hidden.Count / 2))]
        Write-Host ''
        Write-Host '=== GPU bring-up hidden behind other startup work ===' -ForegroundColor Cyan
        Write-Host ("  median  {0,7:N1} ms   (min {1:N1}, max {2:N1})" -f $median, $hidden[0], $hidden[-1])
        Write-Host ''
        Write-Host 'Window is mapped at first_layout_done; first frame follows.' -ForegroundColor DarkGray
        $rows | Format-Table -AutoSize | Out-String | Write-Host
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    $env:RUST_LOG = $null
    $env:TM_STARTUP_TRACE = $null
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
