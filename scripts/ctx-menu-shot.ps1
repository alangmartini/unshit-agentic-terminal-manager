#Requires -Version 5.1
<#
.SYNOPSIS
  Open the workspace context menu near a window edge and capture it.

.DESCRIPTION
  Regression shot for context-menu placement: a menu opened close to the
  bottom (or right) edge must be pulled back inside the window so its danger
  zone -- "Kill all terminals" / "Remove workspace" -- stays clickable.

  Drives the app through TM_STARTUP_DISPATCH (no synthesized input, no focus
  stealing) and captures with PrintWindow(PW_RENDERFULLCONTENT). Runs under a
  throwaway TM_PROFILE so the installed app's daemon, sessions and config are
  never touched.

.EXAMPLE
  pwsh scripts/ctx-menu-shot.ps1 -AnchorX 40 -AnchorY 470
#>
[CmdletBinding()]
param(
    [string]$Out = "ctx-menu-shot.png",
    # Cursor anchor in CSS px. The default sits low enough that an unclamped
    # menu runs off the bottom of the 560px-tall window.
    [double]$AnchorX = 40,
    [double]$AnchorY = 470,
    [int]$Workspace = 0,
    [string]$ExeDir = "",
    [int]$SettleMs = 7000,
    [int]$Width = 900,
    [int]$Height = 560
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class CtxShotWin {
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
[CtxShotWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\debug' }
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $repoRoot $Out }

# A second workspace is what makes "Remove workspace" appear at all -- the row
# is hidden while only one workspace exists, so the shot would prove nothing.
$dispatch = "workspace.add;ctx_menu.open_workspace:${Workspace}:${AnchorX}:${AnchorY}"

$launched = $null
$isolation = Enter-TmIsolation -Tag 'ctxshot'
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
        $h = [CtxShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }

    # Deterministic size, no activation (SWP_NOACTIVATE = 0x10, SWP_NOZORDER = 0x4).
    [CtxShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds $SettleMs

    # The first handle can be the splash window, which is destroyed once the
    # GPU surface is up; resolve again after settling and reposition.
    $again = [CtxShotWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    [CtxShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds 1200

    $rect = New-Object CtxShotWin+RECT
    [CtxShotWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $hh = $rect.Bottom - $rect.Top
    if ($w -lt 200 -or $hh -lt 200) { throw "Window rect too small ($w x $hh); handle is not the main window" }

    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [CtxShotWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host ("Saved {0} ({1}x{2})" -f $Out, $w, $hh)

    if (Test-Path $errLog) {
        $errText = Get-Content $errLog -Raw
        if ($errText -and $errText.Trim()) { Write-Host "--- stderr ---`n$errText" }
    }
    # The isolated config dir goes away on exit; surface the placement event
    # first so the capture doubles as the "did telemetry land" check.
    $events = Join-Path $isolation.ConfigDir 'renderer-events.jsonl'
    if (Test-Path $events) {
        Write-Host '--- renderer-events.jsonl (ctx menu) ---'
        Get-Content $events | Select-String 'ctx_menu' | Select-Object -Last 5
    } else {
        Write-Warning "No renderer-events.jsonl under $($isolation.ConfigDir)"
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
