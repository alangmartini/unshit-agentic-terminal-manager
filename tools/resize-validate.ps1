param(
    [string]$OutDir = '.omx/logs/resize-validate',
    [string]$CargoArgs = 'run',
    [int]$InitialWidth = 1280,
    [int]$InitialHeight = 800,
    [int]$ResizedWidth = 1600,
    [int]$ResizedHeight = 900,
    [switch]$MaximizeAfter,
    [string]$Marker = 'TMRESIZEMARKER123',
    [string]$InputCommand = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function New-Directory {
    param([string]$Path)
    if (-not (Test-Path $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Add-Win32Types {
    if (-not ('ResizeValidate.Win32' -as [type])) {
        Add-Type @"
using System;
using System.Runtime.InteropServices;
namespace ResizeValidate {
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }
    public struct POINT {
        public int X;
        public int Y;
    }
    public static class Win32 {
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
        [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int X, int Y, int nWidth, int nHeight, bool bRepaint);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
        [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int dwAttribute, out RECT pvAttribute, int cbAttribute);
    }
}
"@
    }
}

function Wait-ForWindow {
    param([int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $window = Get-Process -Name 'terminal-manager' -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne 0 } |
            Sort-Object StartTime -Descending |
            Select-Object -First 1
        if ($null -ne $window) { return $window }
        Start-Sleep -Milliseconds 300
    }
    throw "terminal-manager window not found within $TimeoutSeconds seconds."
}

function Get-WindowRect {
    param([System.Diagnostics.Process]$WindowProcess)
    $rect = New-Object ResizeValidate.RECT
    $dwmOk = [ResizeValidate.Win32]::DwmGetWindowAttribute(
        $WindowProcess.MainWindowHandle,
        9,
        [ref]$rect,
        [System.Runtime.InteropServices.Marshal]::SizeOf([type][ResizeValidate.RECT])
    ) -eq 0
    if (-not $dwmOk) {
        [ResizeValidate.Win32]::GetClientRect($WindowProcess.MainWindowHandle, [ref]$rect) | Out-Null
        $origin = New-Object ResizeValidate.POINT
        $origin.X = 0
        $origin.Y = 0
        [ResizeValidate.Win32]::ClientToScreen($WindowProcess.MainWindowHandle, [ref]$origin) | Out-Null
        $rect.Right = $origin.X + $rect.Right
        $rect.Bottom = $origin.Y + $rect.Bottom
        $rect.Left = $origin.X
        $rect.Top = $origin.Y
    }
    return $rect
}

function Capture-Window {
    param([System.Diagnostics.Process]$WindowProcess, [string]$Path)
    $rect = Get-WindowRect -WindowProcess $WindowProcess
    $width = [Math]::Max(1, $rect.Right - $rect.Left)
    $height = [Math]::Max(1, $rect.Bottom - $rect.Top)
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
    return @{ Left = $rect.Left; Top = $rect.Top; Width = $width; Height = $height }
}

function Get-TerminalCropRect {
    param([int]$Width, [int]$Height)
    # Terminal viewport starts after fixed sidebar + tab chrome. Keep crop
    # inside the terminal content area and above the statusbar/taskbar so
    # sidebar/status text cannot mask a blank terminal.
    $x = [Math]::Min([Math]::Max(390, [int][Math]::Round($Width * 0.15)), $Width - 120)
    $y = [Math]::Min([Math]::Max(108, [int][Math]::Round($Height * 0.07)), $Height - 120)
    $w = [Math]::Max(80, $Width - $x - 24)
    $h = [Math]::Max(80, $Height - $y - 250)
    return @{ X = $x; Y = $y; Width = $w; Height = $h }
}

function Save-Crop {
    param([string]$SourcePath, [string]$DestPath, [hashtable]$Rect)
    $source = [System.Drawing.Bitmap]::FromFile($SourcePath)
    try {
        $safe = New-Object System.Drawing.Rectangle(
            $Rect.X,
            $Rect.Y,
            [Math]::Min($Rect.Width, $source.Width - $Rect.X),
            [Math]::Min($Rect.Height, $source.Height - $Rect.Y)
        )
        $cropped = $source.Clone($safe, $source.PixelFormat)
        try {
            $cropped.Save($DestPath, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally {
            $cropped.Dispose()
        }
    } finally {
        $source.Dispose()
    }
}

function Measure-TerminalText {
    param([string]$Path)
    $image = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $bg = $image.GetPixel([Math]::Max(0, $image.Width - 10), [Math]::Max(0, $image.Height - 10))
        $threshold = [Math]::Max(4, [int]($image.Width * 0.004))
        $rowsWithInk = 0
        $inkPixels = 0
        $inkRows = New-Object System.Collections.Generic.List[int]
        for ($y = 0; $y -lt $image.Height; $y++) {
            $rowInk = 0
            for ($x = 0; $x -lt $image.Width; $x++) {
                $p = $image.GetPixel($x, $y)
                $delta = [Math]::Abs([int]$p.R - [int]$bg.R) + [Math]::Abs([int]$p.G - [int]$bg.G) + [Math]::Abs([int]$p.B - [int]$bg.B)
                if ($delta -gt 48) {
                    $rowInk++
                    $inkPixels++
                }
            }
            if ($rowInk -ge $threshold) {
                $rowsWithInk++
                $inkRows.Add($y)
            }
        }
        $bands = New-Object System.Collections.Generic.List[object]
        if ($inkRows.Count -gt 0) {
            $start = $inkRows[0]
            $prev = $inkRows[0]
            for ($i = 1; $i -lt $inkRows.Count; $i++) {
                $cur = $inkRows[$i]
                if (($cur - $prev) -gt 18) {
                    $bands.Add([pscustomobject]@{ start = $start; end = $prev; height = $prev - $start + 1 })
                    $start = $cur
                }
                $prev = $cur
            }
            $bands.Add([pscustomobject]@{ start = $start; end = $prev; height = $prev - $start + 1 })
        }
        $largeGap = 0
        if ($bands.Count -gt 1) {
            for ($i = 1; $i -lt $bands.Count; $i++) {
                $gap = $bands[$i].start - $bands[$i - 1].end
                if ($gap -gt $largeGap) { $largeGap = $gap }
            }
        }
        $strayBottomBand = $false
        if ($bands.Count -gt 1) {
            $last = $bands[$bands.Count - 1]
            $prevBand = $bands[$bands.Count - 2]
            $gapToLast = $last.start - $prevBand.end
            $strayBottomBand = ($gapToLast -gt 80 -and $last.start -gt [int]($image.Height * 0.55))
        }
        return [pscustomobject]@{
            width = $image.Width
            height = $image.Height
            ink_pixels = $inkPixels
            rows_with_ink = $rowsWithInk
            bands = $bands
            largest_blank_gap = $largeGap
            stray_bottom_band = $strayBottomBand
            passed = ($inkPixels -gt 600 -and $rowsWithInk -gt 12 -and -not $strayBottomBand)
        }
    } finally {
        $image.Dispose()
    }
}

function Click-Terminal {
    param([System.Diagnostics.Process]$WindowProcess)
    $rect = Get-WindowRect -WindowProcess $WindowProcess
    $x = $rect.Left + [int](($rect.Right - $rect.Left) * 0.32)
    $y = $rect.Top + [int](($rect.Bottom - $rect.Top) * 0.20)
    [ResizeValidate.Win32]::SetCursorPos($x, $y) | Out-Null
    [ResizeValidate.Win32]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [ResizeValidate.Win32]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
}

Add-Win32Types
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$repoRoot = (Get-Location).Path
$absoluteOutDir = if ([System.IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $repoRoot $OutDir }
New-Directory -Path $absoluteOutDir

$tracePath = Join-Path $absoluteOutDir 'terminal-trace.log'
$runLogPath = Join-Path $absoluteOutDir 'cargo-run.log'
Get-Process -Name 'terminal-manager' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process -Name 'unshit-ptyd' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
$command = "Set-Location '$repoRoot'; `$env:TM_TRACE_TERMINAL='1'; `$env:TM_TRACE_TERMINAL_FILE='$tracePath'; cargo $CargoArgs *> '$runLogPath'"
$cargo = Start-Process powershell -ArgumentList '-NoProfile', '-Command', $command -PassThru -WindowStyle Normal
$window = $null

try {
    $window = Wait-ForWindow -TimeoutSeconds 60
    [ResizeValidate.Win32]::ShowWindowAsync($window.MainWindowHandle, 9) | Out-Null
    [ResizeValidate.Win32]::MoveWindow($window.MainWindowHandle, 60, 60, $InitialWidth, $InitialHeight, $true) | Out-Null
    [ResizeValidate.Win32]::SetForegroundWindow($window.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 1800
    Click-Terminal -WindowProcess $window
    $typed = if ($InputCommand.Length -gt 0) { $InputCommand } else { "echo $Marker" }
    [System.Windows.Forms.SendKeys]::SendWait("$typed{ENTER}")
    Start-Sleep -Milliseconds 1200

    $beforePath = Join-Path $absoluteOutDir 'before.png'
    $beforeRect = Capture-Window -WindowProcess $window -Path $beforePath
    $beforeCrop = Join-Path $absoluteOutDir 'before-terminal.png'
    Save-Crop -SourcePath $beforePath -DestPath $beforeCrop -Rect (Get-TerminalCropRect -Width $beforeRect.Width -Height $beforeRect.Height)
    $before = Measure-TerminalText -Path $beforeCrop

    if ($MaximizeAfter) {
        [ResizeValidate.Win32]::ShowWindowAsync($window.MainWindowHandle, 3) | Out-Null
    } else {
        [ResizeValidate.Win32]::MoveWindow($window.MainWindowHandle, 20, 20, $ResizedWidth, $ResizedHeight, $true) | Out-Null
    }
    Start-Sleep -Milliseconds 1800

    $afterPath = Join-Path $absoluteOutDir 'after.png'
    $afterRect = Capture-Window -WindowProcess $window -Path $afterPath
    $afterCrop = Join-Path $absoluteOutDir 'after-terminal.png'
    Save-Crop -SourcePath $afterPath -DestPath $afterCrop -Rect (Get-TerminalCropRect -Width $afterRect.Width -Height $afterRect.Height)
    $after = Measure-TerminalText -Path $afterCrop
    $inkRatio = if ($before.ink_pixels -gt 0) { [double]$after.ink_pixels / [double]$before.ink_pixels } else { 0.0 }

    $result = [pscustomobject]@{
        passed = ($before.passed -and $after.passed -and $inkRatio -ge 0.40)
        marker = $Marker
        input_command = $typed
        before = $before
        after = $after
        after_to_before_ink_ratio = $inkRatio
        maximize_after = [bool]$MaximizeAfter
        captures = @($beforePath, $afterPath)
        crops = @($beforeCrop, $afterCrop)
        trace = $tracePath
        log = $runLogPath
    }
    $jsonPath = Join-Path $absoluteOutDir 'verdict.json'
    $result | ConvertTo-Json -Depth 5 | Set-Content -Path $jsonPath
    $result | ConvertTo-Json -Depth 5
    if (-not $result.passed) {
        exit 1
    }
} finally {
    if ($null -ne $cargo) {
        Stop-Process -Id $cargo.Id -Force -ErrorAction SilentlyContinue
    }
    Get-Process -Name 'terminal-manager' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-Process -Name 'unshit-ptyd' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}
