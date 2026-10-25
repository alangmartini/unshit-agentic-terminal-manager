#Requires -Version 5.1
<#
.SYNOPSIS
  Measure what the UI process spends while it is idle and while agent panes
  animate.

.DESCRIPTION
  Launches an isolated instance (throwaway TM_PROFILE, see
  lib/tm-isolation.ps1) with one visible workspace and -HiddenAgents
  background workspaces. Every pane runs lib/fake-agent.ps1, whose mode is
  switched per phase through two mode files: one for the visible pane, one
  for every hidden pane. Each phase is "<visible mode>:<hidden mode>", e.g.
  "idle:title" = background agents cycle their titles, visible pane still.

  For each phase the script samples process and per-thread CPU and folds in
  the app's own renderer.frame_summary records (frames, rebuilds, stage
  work) from the isolated profile's renderer-events.jsonl.

  No input is synthesized and the installed app's daemon and sessions are
  never touched.

.EXAMPLE
  powershell -File scripts/idle-cpu-bench.ps1
  powershell -File scripts/idle-cpu-bench.ps1 -Phases idle:idle,idle:title -PhaseSeconds 60
#>
[CmdletBinding()]
param(
    [string]$ExeDir = "",
    [string[]]$Phases = @('idle:idle', 'idle:title', 'spinner:idle', 'spinner:title'),
    [int]$HiddenAgents = 5,
    [int]$SettleSeconds = 25,
    [int]$WarmSeconds = 6,
    [int]$PhaseSeconds = 40,
    [int]$Width = 1600,
    [int]$Height = 1000,
    [string]$JsonOut = "",
    # Optional PNG prefix. Each phase saves two captures 700 ms apart
    # (<prefix>-<phase>-a.png / -b.png) before sampling, so a frozen
    # animation shows up as two identical images. PrintWindow, no focus change.
    [string]$Screenshot = "",
    # Optional file that receives the app's stderr (env_logger output).
    [string]$StderrLog = ""
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')
if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptyd = Join-Path $ExeDir 'unshit-ptyd.exe'
foreach ($binary in $exe, $ptyd) {
    if (-not (Test-Path -LiteralPath $binary)) { throw "missing $binary (cargo build --release -p terminal-manager -p unshit-ptyd)" }
}

Add-Type @"
using System; using System.Runtime.InteropServices;
public class IdleBenchWin {
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
"@

function Write-Utf8NoBom([string]$path, [string]$text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false))
}

# The fake agents poll their mode file, so a write can race a read.
function Set-AgentMode([string]$path, [string]$mode) {
    for ($attempt = 1; ; $attempt++) {
        try { Write-Utf8NoBom $path $mode; return } catch [System.IO.IOException] {
            if ($attempt -ge 20) { throw }
            Start-Sleep -Milliseconds 25
        }
    }
}

$isolation = Enter-TmIsolation -Tag "idlecpu"
$app = $null
$results = @()
try {
    # Every pane runs the fake agent and reads its mode from bench-mode.txt
    # in its own workspace directory. The script is copied to the 8.3 form
    # of the isolated config dir so its path has no spaces for the pane
    # command line to split.
    $shortConfigDir = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($isolation.ConfigDir).ShortPath
    $agentScript = Join-Path $shortConfigDir 'fake-agent.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'lib\fake-agent.ps1') -Destination $agentScript
    $agentShell = [ordered]@{
        program = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
        args    = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $agentScript)
    }

    $visibleModes = @()
    $hiddenModes = @()
    $paneId = 1
    $workspaces = @()
    for ($w = 0; $w -le $HiddenAgents; $w++) {
        $workspaceDir = Join-Path $isolation.ConfigDir ("agent-{0}" -f $w)
        New-Item -ItemType Directory -Force -Path $workspaceDir | Out-Null
        $modeFile = Join-Path $workspaceDir 'bench-mode.txt'
        Write-Utf8NoBom $modeFile 'idle'
        if ($w -eq 0) { $visibleModes += $modeFile } else { $hiddenModes += $modeFile }
        $workspaces += [ordered]@{
            num       = $w + 1
            name      = "bench-$w"
            path      = $workspaceDir
            collapsed = $false
            shell     = $agentShell
            tabs       = @([ordered]@{
                    id          = "bench-tab-$w"
                    name        = ""
                    subtitle    = ""
                    panes       = @(, @([ordered]@{ id = $paneId; title = ""; subtitle = ""; custom_title = $false }))
                    active_pane = $paneId
                    row_ratios  = @(1.0)
                    col_ratios  = @(, @(1.0))
                })
            active_tab = 0
        }
        $paneId++
    }
    $persisted = [ordered]@{
        workspaces               = $workspaces
        active_workspace         = 0
        # The eagerly spawned active pane resolves only the app default.
        default_shell            = $agentShell
        check_updates_on_startup = $false
    }
    Write-Utf8NoBom (Join-Path $isolation.ConfigDir 'workspaces.json') ($persisted | ConvertTo-Json -Depth 12)

    $env:TM_FRAME_SUMMARY_INTERVAL_MS = '10000'
    if ($StderrLog) {
        $app = Start-Process -FilePath $exe -PassThru -RedirectStandardError $StderrLog
    } else {
        $app = Start-Process -FilePath $exe -PassThru
    }
    $null = $app.Handle
    Start-Sleep -Seconds $SettleSeconds
    $app.Refresh()
    if ($app.MainWindowHandle -ne [IntPtr]::Zero) {
        # SWP_NOZORDER | SWP_NOACTIVATE: never steal focus from the user.
        [IdleBenchWin]::SetWindowPos($app.MainWindowHandle, [IntPtr]::Zero, 80, 80, $Width, $Height, 0x0014) | Out-Null
    }
    function Save-WindowShot([string]$path) {
        Add-Type -AssemblyName System.Drawing
        $bitmap = New-Object System.Drawing.Bitmap $Width, $Height
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $hdc = $graphics.GetHdc()
        [IdleBenchWin]::PrintWindow($app.MainWindowHandle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
        $graphics.ReleaseHdc($hdc)
        $graphics.Dispose()
        $bitmap.Save($path)
        $bitmap.Dispose()
    }
    $daemon = Get-CimInstance Win32_Process -Filter "Name = 'unshit-ptyd.exe'" |
        Where-Object { $_.CommandLine -like "*$($isolation.Token)*" -or $_.ParentProcessId -eq $app.Id } |
        Select-Object -First 1

    foreach ($phase in $Phases) {
        $parts = $phase.Split(':')
        foreach ($file in $visibleModes) { Set-AgentMode $file $parts[0] }
        foreach ($file in $hiddenModes) { Set-AgentMode $file $parts[1] }
        Start-Sleep -Seconds $WarmSeconds
        if ($Screenshot) {
            $shotBase = "{0}-{1}" -f $Screenshot, ($phase -replace '[^A-Za-z0-9]+', '-')
            Save-WindowShot "$shotBase-a.png"
            Start-Sleep -Milliseconds 700
            Save-WindowShot "$shotBase-b.png"
        }

        $app.Refresh()
        $threadsBefore = @{}
        foreach ($t in $app.Threads) { try { $threadsBefore[$t.Id] = $t.TotalProcessorTime.TotalMilliseconds } catch {} }
        $procBefore = $app.TotalProcessorTime.TotalMilliseconds
        $daemonBefore = if ($daemon) { (Get-Process -Id $daemon.ProcessId).TotalProcessorTime.TotalMilliseconds } else { 0 }
        $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $clock = [Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds $PhaseSeconds
        $elapsedMs = $clock.Elapsed.TotalMilliseconds
        $endMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $app.Refresh()
        $procAfter = $app.TotalProcessorTime.TotalMilliseconds
        $daemonAfter = if ($daemon) { (Get-Process -Id $daemon.ProcessId).TotalProcessorTime.TotalMilliseconds } else { 0 }
        $hottest = 0.0
        foreach ($t in $app.Threads) {
            try {
                if ($threadsBefore.ContainsKey($t.Id)) {
                    $delta = $t.TotalProcessorTime.TotalMilliseconds - $threadsBefore[$t.Id]
                    if ($delta -gt $hottest) { $hottest = $delta }
                }
            } catch {}
        }

        $summaries = @()
        $events = Join-Path $isolation.ConfigDir 'renderer-events.jsonl'
        if (Test-Path -LiteralPath $events) {
            $summaries = Get-Content -LiteralPath $events | ForEach-Object { $_ | ConvertFrom-Json } |
                Where-Object { $_.event -eq 'renderer.frame_summary' -and $_.timestamp_unix_ms - $_.window_ms -ge $startMs -and $_.timestamp_unix_ms -le $endMs }
        }
        $sum = { param($field) ($summaries | Measure-Object -Property $field -Sum).Sum }
        $windowMs = & $sum 'window_ms'
        $row = [pscustomobject]@{
            phase              = $phase
            process_cores      = [math]::Round(($procAfter - $procBefore) / $elapsedMs, 3)
            hottest_thread     = [math]::Round($hottest / $elapsedMs, 3)
            daemon_cores       = [math]::Round(($daemonAfter - $daemonBefore) / $elapsedMs, 3)
            summary_windows    = @($summaries).Count
            fps                = if ($windowMs) { [math]::Round((& $sum 'frames') * 1000.0 / $windowMs, 1) } else { $null }
            rebuilds_per_s     = if ($windowMs) { [math]::Round((& $sum 'rebuild_frames') * 1000.0 / $windowMs, 1) } else { $null }
            frame_work_cores   = if ($windowMs) { [math]::Round((& $sum 'work_us') / 1000.0 / $windowMs, 3) } else { $null }
            batch_ms_per_frame = if ((& $sum 'frames')) { [math]::Round((& $sum 'batch_build_us') / 1000.0 / (& $sum 'frames'), 2) } else { $null }
            tree_ms_per_rebuild = if ((& $sum 'rebuild_frames')) { [math]::Round((& $sum 'tree_build_us') / 1000.0 / (& $sum 'rebuild_frames'), 2) } else { $null }
            symbol_reshapes_per_s = if ($windowMs) { [math]::Round((& $sum 'symbol_fallbacks') * 1000.0 / $windowMs, 1) } else { $null }
        }
        $results += $row
        $row | Format-List | Out-String | Write-Host
    }
} finally {
    Remove-Item Env:\TM_FRAME_SUMMARY_INTERVAL_MS -ErrorAction SilentlyContinue
    if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptyd
}

$results | Format-Table -AutoSize | Out-String -Width 220 | Write-Host
if ($JsonOut) { Write-Utf8NoBom $JsonOut ($results | ConvertTo-Json -Depth 4) }
