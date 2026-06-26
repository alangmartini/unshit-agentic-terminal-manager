param(
  [string]$Out = "preview-software-renderer.png",
  [int]$SettleMs = 5000,
  [switch]$Software   # set TM_FORCE_SOFTWARE_RENDERER=1 to exercise the fallback path
)

# Visual verification of the software/CPU-renderer fallback path. Launches
# terminal-manager.exe (optionally with TM_FORCE_SOFTWARE_RENDERER=1 so the
# software adapter is selected even on a GPU machine), waits for the window,
# maximizes it, and grabs it with CopyFromScreen. RUST_LOG=info routes the
# "renderer adapter selected (...)" tier line to <Out>.err.txt so the active
# tier is recorded alongside the screenshot. No SendKeys, so it is robust in
# non-interactive/background sessions. Mirrors scripts/qp-attach-shot.ps1.

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class SoftwareRendererShot {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[SoftwareRendererShot]::SetProcessDPIAware() | Out-Null

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exe  = Join-Path $root "target\debug\terminal-manager.exe"
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $root $Out }

$errLog = "$Out.err.txt"
$env:RUST_LOG = "info"
if ($Software) { $env:TM_FORCE_SOFTWARE_RENDERER = "1" }
else { $env:TM_FORCE_SOFTWARE_RENDERER = $null }

$proc = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru -RedirectStandardError $errLog
$tier = if ($Software) { "software (forced)" } else { "hardware (default)" }
Write-Output "Launched pid=$($proc.Id) [$tier], waiting for window..."

$h = [IntPtr]::Zero
for ($i = 0; $i -lt 80; $i++) {
  Start-Sleep -Milliseconds 250
  $proc.Refresh()
  if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)); see $errLog" }
  if ($proc.MainWindowHandle -ne 0) { $h = $proc.MainWindowHandle; break }
}
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $proc.Id -Force; throw "No window appeared; see $errLog" }

[SoftwareRendererShot]::ShowWindow($h, 3) | Out-Null     # SW_MAXIMIZE
Start-Sleep -Milliseconds $SettleMs

$r = New-Object SoftwareRendererShot+RECT
[SoftwareRendererShot]::GetWindowRect($h, [ref]$r) | Out-Null
$w  = $r.Right - $r.Left
$hh = $r.Bottom - $r.Top
$shot = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($shot)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $hh)))
$shot.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $shot.Dispose()

Stop-Process -Id $proc.Id -Force
Write-Output "Saved $Out ($w x $hh); tier log in $errLog"
