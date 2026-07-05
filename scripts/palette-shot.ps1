param(
  [string]$Out = "palette-shot.png",
  [int]$SettleMs = 6000,
  [string]$Keys = "^+p"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  public const uint LEFTDOWN = 0x0002, LEFTUP = 0x0004;
  public static void Click(int x, int y) {
    SetCursorPos(x, y);
    mouse_event(LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
    mouse_event(LEFTUP, 0, 0, 0, UIntPtr.Zero);
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[Win]::SetProcessDPIAware() | Out-Null   # capture true physical pixels (app is per-monitor DPI aware)

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exe  = Join-Path $root "target\debug\terminal-manager.exe"
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }

# Resolve output to an absolute path before we change anything.
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $root $Out }

# Run in a throwaway instance profile so this shot never attaches to a
# real session's daemon or config.
. (Join-Path $PSScriptRoot "lib\tm-isolation.ps1")
$iso = Enter-TmIsolation -Tag "shot"
$ptydExe = Join-Path (Split-Path -Parent $exe) "unshit-ptyd.exe"

$errLog = "$Out.err.txt"
$proc = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru -RedirectStandardError $errLog
Write-Output "Launched pid=$($proc.Id), waiting for window..."

$h = [IntPtr]::Zero
for ($i = 0; $i -lt 80; $i++) {
  Start-Sleep -Milliseconds 250
  $proc.Refresh()
  if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode))" }
  if ($proc.MainWindowHandle -ne 0) { $h = $proc.MainWindowHandle; break }
}
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $proc.Id -Force; throw "No window appeared" }

[Win]::ShowWindow($h, 3) | Out-Null     # SW_MAXIMIZE for an apples-to-apples shot
Start-Sleep -Milliseconds $SettleMs    # let GPU + PTY settle

# A real mouse click focuses the winit window reliably (SetForegroundWindow is
# often denied to a non-foreground caller). Click an empty spot in the terminal
# pane (right-center), away from the sidebar and titlebar.
$r = New-Object Win+RECT
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
$cx = $r.Left + [int](($r.Right - $r.Left) * 0.62)
$cy = $r.Top  + [int](($r.Bottom - $r.Top) * 0.55)
[Win]::SetForegroundWindow($h) | Out-Null
[Win]::Click($cx, $cy)
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait($Keys)
Start-Sleep -Milliseconds 900

$r = New-Object Win+RECT
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
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
