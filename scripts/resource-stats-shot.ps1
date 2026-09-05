#Requires -Version 5.1
<#
.SYNOPSIS
  Put real load in a pane and capture the live status-bar / pane-header
  resource figures.

.DESCRIPTION
  Verification shot for the resource monitor: the status bar's cpu / mem /
  throughput and the pane header's `pid · cpu · mem` must show the pane's
  whole process tree, and unknown figures must read `--`, never `0.0`.

  Seeds the clipboard with a PowerShell one-liner that starts a hidden,
  self-terminating busy loop as a *child* of the pane's shell, then drives
  the app through TM_STARTUP_DISPATCH (split so the header is visible, paste
  so the shell runs the one-liner). No synthesized input, no focus stealing.
  Captures with PrintWindow(PW_RENDERFULLCONTENT) under a throwaway
  TM_PROFILE so the installed app's daemon, sessions and config are never
  touched. Finishes by tailing resource-events.jsonl from the isolated
  profile so the capture doubles as the "did telemetry land" check.

.EXAMPLE
  pwsh scripts/resource-stats-shot.ps1
#>
[CmdletBinding()]
param(
    [string]$Out = "resource-stats-shot.png",
    [string]$ExeDir = "",
    # Long enough for the busy loop to start and for at least two sampler
    # ticks (CPU needs a baseline tick before it shows a number).
    [int]$SettleMs = 12000,
    [int]$BusySeconds = 45,
    [int]$Width = 1100,
    [int]$Height = 600
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ResShotWin {
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
[ResShotWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\debug' }
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $repoRoot $Out }

# A hidden child of the pane's shell that burns one core, then exits on its
# own so a failed cleanup cannot leave it behind. The trailing CR is the
# Enter key: the paste runs the line.
$busy = "`$d=(Get-Date).AddSeconds($BusySeconds); while((Get-Date) -lt `$d){}"
$oneLiner = "Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile','-Command','$busy'`r"
Set-Clipboard -Value $oneLiner

# The pane header only renders in multi-pane tabs; the paste lands in the
# active pane's already-spawned shell.
$dispatch = "pane.split_right;terminal.paste"

$launched = $null
$isolation = Enter-TmIsolation -Tag 'resshot'
$errLog = "$Out.err.txt"
$env:TM_STARTUP_DISPATCH = $dispatch
try {
    try {
        $proc = Start-Process -FilePath $exe -WorkingDirectory $repoRoot -PassThru -RedirectStandardError $errLog
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
    }
    $launched = $proc
    Write-Host "Launched pid=$($proc.Id) dispatch=$dispatch"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)); see $errLog" }
        $h = [ResShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }

    # Deterministic size, no activation (SWP_NOACTIVATE = 0x10, SWP_NOZORDER = 0x4).
    [ResShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds $SettleMs

    # The first handle can be the splash window, which is destroyed once the
    # GPU surface is up; resolve again after settling and reposition.
    $again = [ResShotWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    [ResShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds 1500

    $rect = New-Object ResShotWin+RECT
    [ResShotWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $hh = $rect.Bottom - $rect.Top
    if ($w -lt 200 -or $hh -lt 200) { throw "Window rect too small ($w x $hh); handle is not the main window" }

    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [ResShotWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host ("Saved {0} ({1}x{2})" -f $Out, $w, $hh)

    if (Test-Path $errLog) {
        $errText = Get-Content $errLog -Raw
        if ($errText -and $errText.Trim()) { Write-Host "--- stderr (resource lines) ---"; ($errText -split "`n") | Select-String '"resource\.' | Select-Object -Last 8 }
    }
    $events = Join-Path $isolation.ConfigDir 'resource-events.jsonl'
    if (Test-Path $events) {
        Write-Host '--- resource-events.jsonl ---'
        Get-Content $events | Select-Object -Last 8
    } else {
        Write-Warning "No resource-events.jsonl under $($isolation.ConfigDir)"
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
