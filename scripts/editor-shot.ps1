param(
  [string]$Out = "editor-shot.png",
  [string]$File = "",
  [int]$SettleMs = 6000
)

# Launch an isolated instance with a file opened in the built-in editor
# (via the TM_STARTUP_DISPATCH hook) and capture a window screenshot.

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinEd {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[WinEd]::SetProcessDPIAware() | Out-Null

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exe  = Join-Path $root "target\debug\terminal-manager.exe"
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if ([string]::IsNullOrWhiteSpace($File)) { $File = Join-Path $root "src\state.rs" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $root $Out }

. (Join-Path $PSScriptRoot "lib\tm-isolation.ps1")
$iso = Enter-TmIsolation -Tag "edshot"
$ptydExe = Join-Path (Split-Path -Parent $exe) "unshit-ptyd.exe"

$errLog = "$Out.err.txt"
$env:TM_STARTUP_DISPATCH = "editor.open:$File"
try {
  $proc = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru -RedirectStandardError $errLog
} finally {
  Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
}
Write-Output "Launched pid=$($proc.Id), waiting for window..."

$h = [IntPtr]::Zero
for ($i = 0; $i -lt 80; $i++) {
  Start-Sleep -Milliseconds 250
  $proc.Refresh()
  if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode))" }
  if ($proc.MainWindowHandle -ne 0) { $h = $proc.MainWindowHandle; break }
}
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $proc.Id -Force; throw "No window appeared" }

[WinEd]::ShowWindow($h, 3) | Out-Null   # SW_MAXIMIZE
Start-Sleep -Milliseconds $SettleMs

$r = New-Object WinEd+RECT
[WinEd]::GetWindowRect($h, [ref]$r) | Out-Null
$w  = $r.Right - $r.Left
$hh = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $hh)))
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()

Stop-Process -Id $proc.Id -Force
Exit-TmIsolation -Isolation $iso -PtydExe $ptydExe
Write-Output "Saved $Out ($w x $hh)"
