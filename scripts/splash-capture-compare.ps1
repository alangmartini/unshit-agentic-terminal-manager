#Requires -Version 5.1
<#
.SYNOPSIS
  Capture the startup placeholder two different ways and compare them.

.DESCRIPTION
  The placeholder is drawn with GDI into the window's redirection bitmap. The
  real UI is drawn by DXGI. `PrintWindow(PW_RENDERFULLCONTENT)` is known to
  capture the second correctly -- every previous filmstrip frame proves it --
  but whether it captures the first is exactly the question, and getting that
  wrong means chasing a rendering bug that only exists in the screenshot tool.

  So this grabs the same instant both ways: `PrintWindow` (asks the window to
  reproduce itself) and `CopyFromScreen` (takes what the compositor is actually
  showing). Where they disagree, `CopyFromScreen` is the truth, because it is
  the same thing the user's eyes get.

  `CopyFromScreen` reads screen coordinates, so it only tells the truth about a
  window nothing is covering. The app takes focus when the placeholder appears,
  which makes it the top window -- but do not run this with another window
  dragged over it.
#>
[CmdletBinding()]
param(
    [int]$AtMs = 700,
    [string]$ExeDir,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class CmpWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
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
[CmpWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'target\splash-compare' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Save-Distinct([System.Drawing.Bitmap]$bmp, [string]$path, [string]$label) {
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    # Sample a coarse grid rather than every pixel: the question is only
    # "is this one flat colour or a layout", and 400 samples answers it
    # without the per-pixel cost on a 1280x800 image.
    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    for ($y = 0; $y -lt $bmp.Height; $y += [math]::Max(1, [int]($bmp.Height / 20))) {
        for ($x = 0; $x -lt $bmp.Width; $x += [math]::Max(1, [int]($bmp.Width / 20))) {
            [void]$seen.Add($bmp.GetPixel($x, $y).ToArgb())
        }
    }
    Write-Host ("  {0,-14} {1,3} distinct colours in a 20x20 sample -> {2}" -f $label, $seen.Count, (Split-Path -Leaf $path))
    return $seen.Count
}

$launched = $null
$isolation = Enter-TmIsolation -Tag 'splashcmp'
$env:RUST_LOG = 'info'

try {
    $realLayout = Join-Path $env:APPDATA 'com.godly.terminal\workspaces.json'
    if (Test-Path $realLayout) {
        Copy-Item -LiteralPath $realLayout `
            -Destination (Join-Path $isolation.ConfigDir 'workspaces.json') -Force
        Write-Host 'Seeded layout from the installed profile.'
    }

    $errLog = Join-Path $OutDir 'stderr.log'
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $appExe -PassThru -RedirectStandardError $errLog `
        -RedirectStandardOutput (Join-Path $OutDir 'stdout.log')
    $launched = $proc

    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode))" }
        $h = [CmpWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }
    Write-Host ("Window appeared at {0:N0} ms" -f $clock.Elapsed.TotalMilliseconds)

    while ($clock.Elapsed.TotalMilliseconds -lt $AtMs) { }
    $shotMs = $clock.Elapsed.TotalMilliseconds

    $rect = New-Object CmpWin+RECT
    [CmpWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $h2 = $rect.Bottom - $rect.Top
    Write-Host ("Captured at {0:N0} ms ({1}x{2})" -f $shotMs, $w, $h2)

    $printed = New-Object System.Drawing.Bitmap $w, $h2
    $g = [System.Drawing.Graphics]::FromImage($printed)
    $hdc = $g.GetHdc()
    [CmpWin]::PrintWindow($handle, $hdc, 2) | Out-Null
    $g.ReleaseHdc($hdc); $g.Dispose()

    $screen = New-Object System.Drawing.Bitmap $w, $h2
    $g2 = [System.Drawing.Graphics]::FromImage($screen)
    $g2.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $w, $h2))
    $g2.Dispose()

    $a = Save-Distinct $printed (Join-Path $OutDir 'printwindow.png') 'PrintWindow'
    $b = Save-Distinct $screen (Join-Path $OutDir 'fromscreen.png') 'CopyFromScreen'
    $printed.Dispose(); $screen.Dispose()

    Write-Host ''
    if ($b -gt 1 -and $a -le 1) {
        Write-Host 'The placeholder IS on screen; PrintWindow does not capture GDI content.' -ForegroundColor Yellow
    } elseif ($b -le 1 -and $a -le 1) {
        Write-Host 'Both agree the window is one flat colour: the painter is the problem.' -ForegroundColor Red
    } else {
        Write-Host 'Both show a layout: the placeholder is drawing.' -ForegroundColor Green
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    $env:RUST_LOG = $null
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
