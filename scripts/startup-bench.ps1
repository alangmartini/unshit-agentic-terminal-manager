#Requires -Version 5.1
<#
.SYNOPSIS
  Cold-start benchmark for terminal-manager.exe.

.DESCRIPTION
  Measures wall-clock time from process launch to the app's top-level window
  becoming visible, over N iterations, and reports min/median/p95/max.

  "Visible" means the real window, found by enumerating the process's top-level
  windows -- not `Process.MainWindowHandle`, which during winit startup returns
  a 22x22 helper window that exists about a second before the app does.

  Every iteration is a true COLD start: the isolated instance's ptyd daemon is
  shut down before each launch, so the run pays daemon spawn cost the same way
  a real first-launch-of-the-day does.

  The run is fully isolated (see scripts/lib/tm-isolation.ps1): its own daemon
  pipe, notify pipe, and config dir. It can never attach to the installed app's
  sessions or overwrite its workspaces.json. To reproduce a realistic workload,
  the installed profile's workspaces.json is COPIED into the throwaway config
  dir (never written back).

.PARAMETER Iterations
  How many cold starts to time. Default 7.

.PARAMETER ExeDir
  Directory holding terminal-manager.exe and unshit-ptyd.exe.
  Default: target\release.

.PARAMETER WorkspacesFrom
  workspaces.json to seed the throwaway profile with. Default: the installed
  profile's file, which is what the user actually launches against. Pass
  'none' for an empty profile.

.PARAMETER JsonOut
  Optional path to write the raw results as JSON.

.PARAMETER SettleMs
  How long to let the app run after its window appears, so the in-process
  startup record (written on the first presented frame) lands before teardown.
#>
[CmdletBinding()]
param(
    [int]$Iterations = 7,
    [string]$ExeDir,
    [string]$WorkspacesFrom,
    [string]$JsonOut,
    [int]$SettleMs = 1500
)

# `Process.MainWindowHandle` is not this app's window. winit creates a 22x22
# helper window during startup and Windows associates it with the process
# first, so MainWindowHandle goes non-zero roughly a second before anything the
# user can see -- which is how this benchmark once reported a 110ms startup for
# a launch whose own telemetry says the window appeared at 1.7s. Enumerate the
# process's top-level windows and take the largest visible one instead.
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class BenchWin {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr LargestVisibleWindow(uint targetPid) {
    IntPtr best = IntPtr.Zero; long bestArea = 0;
    EnumWindows(delegate(IntPtr h, IntPtr p) {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid != targetPid || !IsWindowVisible(h)) return true;
      RECT r; if (!GetWindowRect(h, out r)) return true;
      long a = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (a > bestArea) { bestArea = a; best = h; }
      return true;
    }, IntPtr.Zero);
    // Smaller than any plausible app window means a helper, not the app.
    return bestArea >= 40000 ? best : IntPtr.Zero;
  }
}
"@

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
foreach ($exe in @($appExe, $ptydExe)) {
    if (-not (Test-Path -LiteralPath $exe)) { throw "missing $exe -- build first" }
}

if (-not $WorkspacesFrom) {
    $WorkspacesFrom = Join-Path $env:APPDATA 'com.godly.terminal\workspaces.json'
}

$isolation = Enter-TmIsolation -Tag 'bench'
Write-Host "isolated profile : $($isolation.Token)"
Write-Host "config dir       : $($isolation.ConfigDir)"
Write-Host "daemon pipe      : $($isolation.PipePath)"

# Startup telemetry lands next to the profile's config; keep it out of the way
# of the real instance and let each iteration append one line.
$env:TM_STARTUP_TRACE = '1'
$telemetryPath = Join-Path $isolation.ConfigDir 'startup-events.jsonl'

try {
    if ($WorkspacesFrom -ne 'none' -and (Test-Path -LiteralPath $WorkspacesFrom)) {
        Copy-Item -LiteralPath $WorkspacesFrom -Destination (Join-Path $isolation.ConfigDir 'workspaces.json') -Force
        $wsCount = (Get-Content -Raw -LiteralPath $WorkspacesFrom | ConvertFrom-Json).workspaces.Count
        Write-Host "seeded workspaces: $wsCount (copied from $WorkspacesFrom)"
    } else {
        Write-Host "seeded workspaces: 0 (empty profile)"
    }

    $samples = New-Object System.Collections.Generic.List[double]

    # Shutting down a daemon that is not running is the normal case on the
    # first iteration and is not an error worth failing the run over.
    function Stop-BenchDaemon {
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'SilentlyContinue'
        try { & $ptydExe --shutdown --force --socket $isolation.PipePath | Out-Null } catch {}
        $ErrorActionPreference = $prev
        $global:LASTEXITCODE = 0
    }

    for ($i = 1; $i -le $Iterations; $i++) {
        # Cold start: no daemon may be alive on our pipe when the app launches.
        Stop-BenchDaemon
        Start-Sleep -Milliseconds 150

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process -FilePath $appExe -PassThru
        $visibleMs = $null

        while ($sw.Elapsed.TotalSeconds -lt 30) {
            try { $proc.Refresh() } catch { break }
            if ($proc.HasExited) { break }
            if ([BenchWin]::LargestVisibleWindow([uint32]$proc.Id) -ne [IntPtr]::Zero) {
                $visibleMs = $sw.Elapsed.TotalMilliseconds
                break
            }
        }
        $sw.Stop()

        if ($null -eq $visibleMs) {
            Write-Warning "iteration ${i}: window never appeared"
        } else {
            $samples.Add($visibleMs)
            Write-Host ("iteration {0,2}: {1,8:N1} ms to visible window" -f $i, $visibleMs)
        }

        # Let the app finish coming up so its in-process startup record
        # (emitted on the first frame) actually gets written before teardown.
        Start-Sleep -Milliseconds $SettleMs

        try { if (-not $proc.HasExited) { $proc.Kill() } } catch {}
        try { $proc.WaitForExit(5000) | Out-Null } catch {}
        Start-Sleep -Milliseconds 100
    }

    if ($samples.Count -eq 0) { throw 'no successful iterations' }

    $sorted = $samples | Sort-Object
    function Percentile([object[]]$data, [double]$p) {
        $idx = [Math]::Min($data.Count - 1, [Math]::Max(0, [int][Math]::Ceiling($p * $data.Count) - 1))
        [double]$data[$idx]
    }
    $stats = [ordered]@{
        iterations = $samples.Count
        min_ms     = [Math]::Round($sorted[0], 1)
        median_ms  = [Math]::Round((Percentile $sorted 0.5), 1)
        p95_ms     = [Math]::Round((Percentile $sorted 0.95), 1)
        max_ms     = [Math]::Round($sorted[$sorted.Count - 1], 1)
        samples_ms = @($samples | ForEach-Object { [Math]::Round($_, 1) })
    }

    Write-Host ''
    Write-Host '=== window-visible, cold start ==='
    Write-Host ("  min    {0,8:N1} ms" -f $stats.min_ms)
    Write-Host ("  median {0,8:N1} ms" -f $stats.median_ms)
    Write-Host ("  p95    {0,8:N1} ms" -f $stats.p95_ms)
    Write-Host ("  max    {0,8:N1} ms" -f $stats.max_ms)

    if (Test-Path -LiteralPath $telemetryPath) {
        Write-Host ''
        Write-Host '=== in-process stage breakdown (last run) ==='
        Get-Content -LiteralPath $telemetryPath | Select-Object -Last 1
        Copy-Item -LiteralPath $telemetryPath -Destination (Join-Path $repoRoot 'target\startup-events.jsonl') -Force
        Write-Host "(all runs copied to target\startup-events.jsonl)"
    }

    if ($JsonOut) {
        $stats | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 -LiteralPath $JsonOut
        Write-Host "wrote $JsonOut"
    }
} finally {
    # Teardown shuts down a daemon that may already be gone; a native
    # non-zero exit there must not mask the benchmark's own result.
    $ErrorActionPreference = 'SilentlyContinue'
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
    $env:TM_STARTUP_TRACE = $null
}
