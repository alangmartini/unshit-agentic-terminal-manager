#Requires -Version 5.1
<#
  Merge verification for PR #191: prove that this branch's Git diff review
  (review.*) and the editor diff pane that landed on base in #193 (diff.*)
  both still open after the namespace split.

  Drives the app with TM_STARTUP_DISPATCH rather than SendKeys: Windows
  denies foreground stealing, so synthesized keys can land in whatever
  window happens to be focused, including the user's live terminal.

  Usage: scripts\merge-verify-shot.ps1 -Dispatch "review.open" -Out shot.png
#>
param(
  [Parameter(Mandatory = $true)][string]$Dispatch,
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$SettleMs = 7000,
  [string]$ExeDir = "target\debug"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ShotWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[ShotWin]::SetProcessDPIAware() | Out-Null

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exe = Join-Path $root (Join-Path $ExeDir "terminal-manager.exe")
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (build first)" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $root $Out }

. (Join-Path $PSScriptRoot "lib\tm-isolation.ps1")
$iso = Enter-TmIsolation -Tag "mergeverify"
$ptydExe = Join-Path (Split-Path -Parent $exe) "unshit-ptyd.exe"

$env:TM_STARTUP_DISPATCH = $Dispatch
$errLog = "$Out.err.txt"
$proc = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru -RedirectStandardError $errLog
$procId = $proc.Id
Write-Output "Launched pid=$procId with TM_STARTUP_DISPATCH=$Dispatch"

try {
  # The pre-first-frame handle can be the splash window (destroyed later) or a
  # 22x22 placeholder, so settle first and only then trust MainWindowHandle.
  $h = [IntPtr]::Zero
  for ($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    $proc.Refresh()
    if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode))" }
    if ($proc.MainWindowHandle -ne 0) { $h = $proc.MainWindowHandle; break }
  }
  if ($h -eq [IntPtr]::Zero) { throw "No window appeared" }

  Start-Sleep -Milliseconds $SettleMs
  $proc.Refresh()
  if ($proc.HasExited) { throw "Process exited during settle (code $($proc.ExitCode))" }
  $h = $proc.MainWindowHandle
  [ShotWin]::ShowWindow($h, 3) | Out-Null   # SW_MAXIMIZE
  Start-Sleep -Milliseconds 1500

  $r = New-Object ShotWin+RECT
  [ShotWin]::GetWindowRect($h, [ref]$r) | Out-Null
  $w = $r.Right - $r.Left
  $hh = $r.Bottom - $r.Top
  if ($w -lt 100 -or $hh -lt 100) { throw "Window rect is $w x $hh; handle never resolved" }

  $bmp = New-Object System.Drawing.Bitmap $w, $hh
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $hh)))
  $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  Write-Output "Saved $Out ($w x $hh)"
} finally {
  Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
  Remove-Item Env:\TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
  Exit-TmIsolation -Isolation $iso -PtydExe $ptydExe
}
