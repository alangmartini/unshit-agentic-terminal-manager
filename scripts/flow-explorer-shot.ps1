#Requires -Version 5.1
<#
.SYNOPSIS
  Launch an isolated instance with a Flow Explorer pane open and capture it.

.DESCRIPTION
  Drives the app through TM_STARTUP_DISPATCH (no synthesized input, no focus
  stealing) and captures the window with PrintWindow(PW_RENDERFULLCONTENT),
  which works even when another window covers the app. The default dispatch
  opens the committed fixture flow; pass -Dispatch to chain view commands,
  e.g. "flow.open:<path>;flow.view:panes;flow.select:0:ui.cmd-enter".

  Runs under a throwaway TM_PROFILE so the installed app's daemon, sessions
  and config are never touched.
#>
[CmdletBinding()]
param(
    [string]$Out = "flow-explorer-shot.png",
    [string]$Dispatch = "",
    [string]$Fixture = "",
    [string]$ExeDir = "",
    [int]$SettleMs = 7000,
    [int]$Width = 1400,
    [int]$Height = 900
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FlowShotWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
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
    return bestArea >= 40000 ? best : IntPtr.Zero;
  }
}
"@
[FlowShotWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\debug' }
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if (-not $Fixture) { $Fixture = Join-Path $repoRoot 'tests\fixtures\flow-explorer\send-a-prompt.json' }
if (-not $Dispatch) { $Dispatch = "flow.open:$Fixture" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $repoRoot $Out }

$launched = $null
$isolation = Enter-TmIsolation -Tag 'flowshot'
$errLog = "$Out.err.txt"
$env:TM_STARTUP_DISPATCH = $Dispatch
try {
    try {
        $proc = Start-Process -FilePath $exe -WorkingDirectory $repoRoot -PassThru -RedirectStandardError $errLog
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
    }
    $launched = $proc
    Write-Host "Launched pid=$($proc.Id) dispatch=$Dispatch"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)); see $errLog" }
        $h = [FlowShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }

    # Deterministic size, no activation (SWP_NOACTIVATE = 0x10, SWP_NOZORDER = 0x4).
    [FlowShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 120, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds $SettleMs

    # The first handle can be the splash window, which is destroyed once the
    # GPU surface is up; resolve again after settling and reposition.
    $again = [FlowShotWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    [FlowShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 120, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds 800

    $rect = New-Object FlowShotWin+RECT
    [FlowShotWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $hh = $rect.Bottom - $rect.Top
    if ($w -lt 200 -or $hh -lt 200) { throw "Window rect too small ($w x $hh); handle is not the main window" }

    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [FlowShotWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)

    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    for ($y = 0; $y -lt $bmp.Height; $y += [math]::Max(1, [int]($bmp.Height / 24))) {
        for ($x = 0; $x -lt $bmp.Width; $x += [math]::Max(1, [int]($bmp.Width / 24))) {
            [void]$seen.Add($bmp.GetPixel($x, $y).ToArgb())
        }
    }
    $bmp.Dispose()
    Write-Host ("Saved {0} ({1}x{2}, {3} distinct colours in a 24x24 sample)" -f $Out, $w, $hh, $seen.Count)
    if ($seen.Count -lt 4) { Write-Warning 'Capture looks flat; the pane may not have rendered.' }
    if (Test-Path $errLog) {
        $errText = Get-Content $errLog -Raw
        if ($errText -and $errText.Trim()) { Write-Host "--- stderr ---`n$errText" }
    }
    # The isolated config dir is deleted on exit; surface the flow telemetry
    # first so a capture doubles as the "did the event land" check.
    $events = Join-Path $isolation.ConfigDir 'flow-events.jsonl'
    if (Test-Path $events) {
        Write-Host '--- flow-events.jsonl (tail) ---'
        Get-Content $events -Tail 8
    } else {
        Write-Warning "No flow-events.jsonl under $($isolation.ConfigDir)"
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
