<#
.SYNOPSIS
  Launch terminal-manager, right-click a sidebar workspace to open the new
  design-#2 context menu, hover "New terminal" to reveal the shell flyout,
  and screenshot each state for visual-parity review.

.NOTES
  Captures land in -OutDir (default artifacts/sidebar-menu/latest):
    app.png         baseline window, no menu
    menu-open.png   root context menu open (cursor parked off-menu)
    flyout.png      "New terminal" hovered, shell flyout revealed
    menu-crop.png   tight crop around the menu region
  The app runs against a unique TM_PTYD_SOCKET so it never touches a live
  daemon. Foreground screen capture is used, so keep the window uncovered.
#>
param(
    [string]$OutDir = 'artifacts/sidebar-menu/latest',
    [string]$ExePath = '',
    [int]$WindowX = 40,
    [int]$WindowY = 40,
    [int]$WindowWidth = 1280,
    [int]$WindowHeight = 800,
    # Right-click target, in client-area pixels (lands on a workspace head row).
    [int]$ClickX = 120,
    [int]$ClickY = 96,
    # Offset from the click point to the "New terminal" row, for the flyout shot.
    [int]$NewTermDx = 0,
    [int]$NewTermDy = 64,
    # Where to park the cursor for the clean menu-open shot (client pixels).
    [int]$ParkX = 760,
    [int]$ParkY = 470,
    [int]$StartupMs = 6000,
    [int]$MenuSettleMs = 700,
    [int]$WindowTimeoutSeconds = 40
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.Drawing

$scriptDir = Split-Path -Parent $PSCommandPath
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..')).Path

function New-Directory { param([string]$Path)
    if (-not (Test-Path $Path)) { New-Item -ItemType Directory -Path $Path -Force | Out-Null }
}

if (-not ('SbShot.Win32' -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
namespace SbShot {
  public struct RECT { public int Left, Top, Right, Bottom; }
  public struct POINT { public int X, Y; }
  public static class Win32 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, IntPtr extra);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT val, int sz);
  }
}
"@
}

$MOUSEEVENTF_RIGHTDOWN = 0x0008
$MOUSEEVENTF_RIGHTUP   = 0x0010

function Move-Cursor { param([int]$X, [int]$Y)
    [SbShot.Win32]::SetCursorPos($X, $Y) | Out-Null
    Start-Sleep -Milliseconds 120
}
function Invoke-RightClick { param([int]$X, [int]$Y)
    Move-Cursor -X $X -Y $Y
    [SbShot.Win32]::mouse_event($MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [SbShot.Win32]::mouse_event($MOUSEEVENTF_RIGHTUP, 0, 0, 0, [IntPtr]::Zero)
}

function Get-ClientOrigin { param([System.Diagnostics.Process]$Proc)
    $p = New-Object SbShot.POINT; $p.X = 0; $p.Y = 0
    [SbShot.Win32]::ClientToScreen($Proc.MainWindowHandle, [ref]$p) | Out-Null
    return $p
}

function Wait-ForWindow { param([System.Diagnostics.Process]$Proc, [int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $Proc.Refresh()
        if ($Proc.HasExited) { throw "terminal-manager exited before a window appeared." }
        if ($Proc.MainWindowHandle -ne 0) { return $Proc }
        $byName = Get-Process -Name 'terminal-manager' -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if ($null -ne $byName) { return $byName }
        Start-Sleep -Milliseconds 250
    }
    throw "terminal-manager window not found within $TimeoutSeconds s."
}

function Capture-Window { param([System.Diagnostics.Process]$Proc, [string]$Path)
    $rect = New-Object SbShot.RECT
    $dwmOk = [SbShot.Win32]::DwmGetWindowAttribute($Proc.MainWindowHandle, 9, [ref]$rect,
        [System.Runtime.InteropServices.Marshal]::SizeOf([type][SbShot.RECT])) -eq 0
    if (-not $dwmOk) { [SbShot.Win32]::GetWindowRect($Proc.MainWindowHandle, [ref]$rect) | Out-Null }
    $w = [Math]::Max(1, $rect.Right - $rect.Left)
    $h = [Math]::Max(1, $rect.Bottom - $rect.Top)
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    try {
        $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
        $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $g.Dispose(); $bmp.Dispose() }
    return @{ Left = $rect.Left; Top = $rect.Top; Width = $w; Height = $h }
}

function Save-Crop { param([string]$Src, [string]$Dst, [int]$X, [int]$Y, [int]$W, [int]$H)
    $img = [System.Drawing.Bitmap]::FromFile($Src)
    try {
        $X = [Math]::Max(0, [Math]::Min($X, $img.Width - 1))
        $Y = [Math]::Max(0, [Math]::Min($Y, $img.Height - 1))
        $W = [Math]::Max(1, [Math]::Min($W, $img.Width - $X))
        $H = [Math]::Max(1, [Math]::Min($H, $img.Height - $Y))
        $r = New-Object System.Drawing.Rectangle($X, $Y, $W, $H)
        $c = $img.Clone($r, $img.PixelFormat)
        try { $c.Save($Dst, [System.Drawing.Imaging.ImageFormat]::Png) } finally { $c.Dispose() }
    } finally { $img.Dispose() }
}

# --- Resolve binary -------------------------------------------------------
if ($ExePath -eq '') { $ExePath = Join-Path $repoRoot 'target\debug\terminal-manager.exe' }
if (-not (Test-Path $ExePath)) { throw "terminal-manager.exe not found at $ExePath. Build it first." }

$absOut = if ([System.IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $repoRoot $OutDir }
New-Directory -Path $absOut

$runId = [Guid]::NewGuid().ToString('N').Substring(0, 8)
$socket = "\\.\pipe\unshit-ptyd-shot-$PID-$runId"

$proc = $null
try {
    $prev = [Environment]::GetEnvironmentVariable('TM_PTYD_SOCKET', 'Process')
    [Environment]::SetEnvironmentVariable('TM_PTYD_SOCKET', $socket, 'Process')
    try {
        $proc = Start-Process -FilePath $ExePath -WorkingDirectory $repoRoot -PassThru -WindowStyle Normal
    } finally {
        [Environment]::SetEnvironmentVariable('TM_PTYD_SOCKET', $prev, 'Process')
    }

    $win = Wait-ForWindow -Proc $proc -TimeoutSeconds $WindowTimeoutSeconds

    # SWP_NOZORDER(0x4) | SWP_NOACTIVATE? we DO want activate for screen capture.
    [SbShot.Win32]::SetWindowPos($win.MainWindowHandle, [IntPtr]::Zero,
        $WindowX, $WindowY, $WindowWidth, $WindowHeight, 0x0040) | Out-Null
    Start-Sleep -Milliseconds 400
    [SbShot.Win32]::ShowWindowAsync($win.MainWindowHandle, 5) | Out-Null
    [SbShot.Win32]::SetForegroundWindow($win.MainWindowHandle) | Out-Null

    Write-Host "Waiting ${StartupMs}ms for app to settle..."
    Start-Sleep -Milliseconds $StartupMs

    $origin = Get-ClientOrigin -Proc $win
    Write-Host ("client origin = {0},{1}" -f $origin.X, $origin.Y)

    # Baseline (no menu).
    [SbShot.Win32]::SetForegroundWindow($win.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 200
    Capture-Window -Proc $win -Path (Join-Path $absOut 'app.png') | Out-Null

    # Open the context menu via right-click, then park the cursor off-menu.
    Invoke-RightClick -X ($origin.X + $ClickX) -Y ($origin.Y + $ClickY)
    Start-Sleep -Milliseconds $MenuSettleMs
    Capture-Window -Proc $win -Path (Join-Path $absOut 'menu-raw.png') | Out-Null
    Move-Cursor -X ($origin.X + $ParkX) -Y ($origin.Y + $ParkY)
    Start-Sleep -Milliseconds $MenuSettleMs
    $menuRect = Capture-Window -Proc $win -Path (Join-Path $absOut 'menu-open.png')

    # Hover "New terminal" to reveal the flyout.
    Move-Cursor -X ($origin.X + $ClickX + $NewTermDx) -Y ($origin.Y + $ClickY + $NewTermDy)
    Start-Sleep -Milliseconds $MenuSettleMs
    Capture-Window -Proc $win -Path (Join-Path $absOut 'flyout.png') | Out-Null

    # Tight crop around the menu (client offset + generous box) for review.
    $cropX = $ClickX - 8
    $cropY = $ClickY - 8
    Save-Crop -Src (Join-Path $absOut 'flyout.png') -Dst (Join-Path $absOut 'menu-crop.png') `
        -X ($cropX + ($origin.X - $menuRect.Left)) -Y ($cropY + ($origin.Y - $menuRect.Top)) -W 470 -H 360

    Write-Host "Saved captures to $absOut"
}
finally {
    if ($null -ne $proc) {
        try { $proc.Refresh(); if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } } catch {}
    }
    Get-Process -Name 'unshit-ptyd' -ErrorAction SilentlyContinue |
        Where-Object { $_.StartTime -gt (Get-Date).AddMinutes(-5) } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
}
