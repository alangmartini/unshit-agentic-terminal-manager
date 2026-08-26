#Requires -Version 5.1
<#
.SYNOPSIS
  Check that the app takes keyboard focus when its deferred window appears.

.DESCRIPTION
  Deferring the window until it can paint means it is mapped by `set_visible`
  rather than created visible, and mapping a window does not focus it. The app
  calls `focus_window` to compensate, but that is `SetForegroundWindow`, a
  request Windows is entitled to refuse. If it were refused the app would look
  perfectly correct and silently drop the first thing the user typed -- a
  failure no screenshot of a right-looking UI would catch.

  Reports the foreground window twice: at the instant the window appears, and
  again after a settle. The first is expected to be false -- the reveal and the
  focus request are not processed in the same instant -- so the second is the
  verdict. Also screenshots the first frame the user would actually see.
#>
[CmdletBinding()]
param(
    [string]$Out,
    [string]$ExeDir,
    [int]$SettleMs = 2500
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FocusWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", EntryPoint="GetWindowThreadProcessId")] public static extern uint GetWindowPid(IntPtr h, out uint pid);
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
[FocusWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
if (-not $Out) { $Out = Join-Path $repoRoot 'target\startup-filmstrip\focus-check.png' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null

$launched = $null
$isolation = Enter-TmIsolation -Tag 'focus'
$env:RUST_LOG = 'info'

try {
    $errLog = "$Out.stderr.log"
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $appExe -PassThru -RedirectStandardError $errLog `
        -RedirectStandardOutput "$Out.stdout.log"
    $launched = $proc

    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode))" }
        $h = [FocusWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
    }
    if ($handle -eq [IntPtr]::Zero) { throw "No window within 20s" }
    $appearedMs = $clock.Elapsed.TotalMilliseconds

    $fg = [FocusWin]::GetForegroundWindow()
    $isForeground = ($fg -eq $handle)
    Write-Host ("Window appeared at {0:N0} ms" -f $appearedMs) -ForegroundColor Cyan
    Write-Host ("Foreground on appear: {0}" -f $isForeground) `
        -ForegroundColor $(if ($isForeground) { 'Green' } else { 'Red' })

    # Deliberately no SendKeys. Keystrokes go to whichever window holds focus,
    # and whether that is this app is the entire question being asked -- so on
    # the failing path the test types into an unrelated window, which on a
    # developer machine means someone's live shell. Name the holder instead.
    Start-Sleep -Milliseconds $SettleMs
    $fgAfter = [FocusWin]::GetForegroundWindow()
    $fgPid = 0
    [FocusWin]::GetWindowPid($fgAfter, [ref]$fgPid) | Out-Null
    $fgName = try { (Get-Process -Id $fgPid -ErrorAction Stop).ProcessName } catch { "pid $fgPid" }
    $isApp = ($fgAfter -eq $handle)
    Write-Host ("Foreground after settle: {0} (pid {1}) -- {2}" -f $fgName, $fgPid,
        $(if ($isApp) { 'this IS the app' } else { 'NOT the app' })) `
        -ForegroundColor $(if ($isApp) { 'Green' } else { 'Red' })

    $rect = New-Object FocusWin+RECT
    [FocusWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $h2 = $rect.Bottom - $rect.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $h2
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [FocusWin]::PrintWindow($handle, $hdc, 2) | Out-Null
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()

    Write-Host ("Saved $Out ({0}x{1})" -f $w, $h2)
    Write-Host 'Screenshot is for eyeballing the first visible frame; the focus verdict is above.' -ForegroundColor DarkGray
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    $env:RUST_LOG = $null
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
