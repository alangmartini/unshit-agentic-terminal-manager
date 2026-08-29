#Requires -Version 5.1
<#
.SYNOPSIS
  Photograph one cold start at fixed offsets, so what the user actually sees
  can be checked instead of inferred from timings.

.DESCRIPTION
  Stage timings say when the window was created and when the first frame
  landed. They do not say what occupies the gap, and the gap is the whole user
  experience: this is how we found that the window used to exist from ~210ms
  while the first paint was ~1.4s away, and that it spent that entire interval
  unable to answer a message, because GPU bring-up runs on the event-loop
  thread. A window that appears instantly and then hangs white for a second is
  worse than one that appears late and drawn.

  Launches the release build under a throwaway instance profile, polls for the
  window with no sleep so the measurement is not quantised by the poll
  interval, and captures at a series of offsets from launch. Each capture
  records the offset it actually happened at, not the one requested -- capture
  is not free, and on a hung window PrintWindow blocks until the app pumps
  messages again, which is itself the measurement.

  It also greps the run's stderr for the background-work telemetry, so a shot
  showing branches filled in is backed by the event that filled them.
#>
[CmdletBinding()]
param(
    [int[]]$OffsetsMs = @(120, 300, 600, 1000, 1500, 2500, 4000),
    [string]$OutDir,
    [string]$ExeDir,
    [switch]$KeepProfile
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class ShotWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

  // Process.MainWindowHandle picks the first top-level window Windows happens
  // to associate with the process, which during winit startup can be a 22x22
  // helper window rather than the app. Enumerate instead and take the largest
  // visible top-level window owned by the pid.
  public static IntPtr LargestVisibleWindow(uint targetPid) {
    IntPtr best = IntPtr.Zero;
    long bestArea = 0;
    EnumWindows(delegate(IntPtr h, IntPtr p) {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid != targetPid) return true;
      if (!IsWindowVisible(h)) return true;
      RECT r; if (!GetWindowRect(h, out r)) return true;
      long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (area > bestArea) { bestArea = area; best = h; }
      return true;
    }, IntPtr.Zero);
    // Anything smaller than a plausible window is a helper, not the app.
    return bestArea >= 40000 ? best : IntPtr.Zero;
  }
}
"@
[ShotWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'target\startup-filmstrip' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $appExe)) { throw "Missing $appExe (cargo build --release -p terminal-manager -p unshit-ptyd)" }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Get-ChildItem -Path $OutDir -Filter '*.png' -ErrorAction SilentlyContinue | Remove-Item -Force

$launched = $null
$isolation = Enter-TmIsolation -Tag 'filmstrip'
$env:RUST_LOG = 'info'
$env:TM_STARTUP_TRACE = '1'

try {
    # Copy the real layout in: the deferred git and pane work only shows up
    # against a profile that actually has several workspaces and panes.
    $realLayout = Join-Path $env:APPDATA 'com.godly.terminal\workspaces.json'
    if (Test-Path $realLayout) {
        Copy-Item -LiteralPath $realLayout -Destination (Join-Path $isolation.ConfigDir 'workspaces.json') -Force
        Write-Host "Seeded layout from the installed profile." -ForegroundColor DarkGray
    } else {
        Write-Host "No installed layout found; running against an empty profile." -ForegroundColor Yellow
    }

    $errLog = Join-Path $OutDir 'stderr.log'
    $outLog = Join-Path $OutDir 'stdout.log'

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $appExe -PassThru `
        -RedirectStandardError $errLog -RedirectStandardOutput $outLog
    $launched = $proc
    $spawnMs = $clock.Elapsed.TotalMilliseconds

    # No sleep in this loop: a 250ms poll cannot observe a 200ms window.
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 15000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)). See $errLog" }
        $h = [ShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
    }
    if ($handle -eq [IntPtr]::Zero) { throw "No window within 15s" }
    $windowMs = $clock.Elapsed.TotalMilliseconds

    Write-Host ''
    Write-Host ("Start-Process returned at   {0,7:N1} ms" -f $spawnMs) -ForegroundColor DarkGray
    Write-Host ("Window handle observed at  {0,7:N1} ms" -f $windowMs) -ForegroundColor Cyan
    Write-Host ('  (includes PowerShell process-creation overhead, so it reads' ) -ForegroundColor DarkGray
    Write-Host ('   higher than the in-process window_created stage timing)') -ForegroundColor DarkGray
    Write-Host ''

    $shots = @()
    foreach ($offset in $OffsetsMs) {
        while ($clock.Elapsed.TotalMilliseconds -lt $offset) { }
        $at = $clock.Elapsed.TotalMilliseconds

        $rect = New-Object ShotWin+RECT
        if (-not [ShotWin]::GetWindowRect($handle, [ref]$rect)) { continue }
        $w = $rect.Right - $rect.Left
        $h = $rect.Bottom - $rect.Top
        if ($w -le 0 -or $h -le 0) { continue }

        # PrintWindow, not CopyFromScreen: the screen at these coordinates also
        # contains whatever is stacked behind an unpainted window, and an early
        # frame is exactly when the window has nothing of its own to show. A
        # screen grab therefore reports another app's pixels as ours -- which
        # is how a fully-rendered UI first appeared at 267ms in a run whose own
        # telemetry says nothing was painted until 1.6s.
        $bmp = New-Object System.Drawing.Bitmap $w, $h
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        $ok = [ShotWin]::PrintWindow($handle, $hdc, 2)  # PW_RENDERFULLCONTENT
        $g.ReleaseHdc($hdc)
        $g.Dispose()
        if (-not $ok) { Write-Host ("  PrintWindow failed at t={0:N0}" -f $at) -ForegroundColor Yellow }

        $path = Join-Path $OutDir ("t{0:D5}ms.png" -f [int]$at)
        $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()

        $shots += [pscustomobject]@{ RequestedMs = $offset; ActualMs = [math]::Round($at, 1); Path = $path }
        Write-Host ("  captured t={0,6:N0} ms  ({1}x{2})" -f $at, $w, $h)
    }

    Start-Sleep -Milliseconds 500
    try { if (-not $proc.HasExited) { $proc.Kill() } } catch {}
    try { $proc.WaitForExit(5000) | Out-Null } catch {}

    Write-Host ''
    Write-Host '=== background-work telemetry ===' -ForegroundColor Cyan
    $wanted = @('git.branches_resolved', 'panes.background_reattached', 'app.startup')
    foreach ($ev in $wanted) {
        $hits = Select-String -Path $errLog -SimpleMatch $ev -ErrorAction SilentlyContinue
        if (-not $hits) {
            Write-Host ("  MISSING: {0}" -f $ev) -ForegroundColor Red
            continue
        }
        foreach ($hit in $hits) {
            $line = $hit.Line -replace '^.*?(\{")', '$1'
            if ($line.Length -gt 400) { $line = $line.Substring(0, 400) + ' ...' }
            Write-Host "  $line" -ForegroundColor Green
        }
    }

    Write-Host ''
    Write-Host ("Frames in $OutDir") -ForegroundColor Cyan
    $shots | Format-Table -AutoSize | Out-String | Write-Host
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    # In the finally block, not after the captures: a run that throws early
    # (no window appeared, capture failed) otherwise leaves the app alive
    # holding a lock on the very .exe the next `cargo build` has to replace.
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    $env:RUST_LOG = $null
    $env:TM_STARTUP_TRACE = $null
    if (-not $KeepProfile) { Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe }
}
