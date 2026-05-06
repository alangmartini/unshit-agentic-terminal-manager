param(
    [int]$DurationSeconds = 5,
    [string]$OutDir = '.omx/logs/visual-loop',
    [string]$CargoArgs = 'run',
    [int]$FrameCount = 6
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
    if (-not ('VisualLoop.Win32' -as [type])) {
        Add-Type @"
using System;
using System.Runtime.InteropServices;
namespace VisualLoop {
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
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
        [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int dwAttribute, out RECT pvAttribute, int cbAttribute);
    }
}
"@
    }
}

function Wait-ForWindow {
    param(
        [string]$TitleLike,
        [int]$TimeoutSeconds
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $window = Get-Process -Name 'terminal-manager' -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne 0 } |
            Sort-Object StartTime -Descending |
            Select-Object -First 1
        if ($null -ne $window) {
            return $window
        }

        $window = Get-Process |
            Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle -like $TitleLike } |
            Sort-Object StartTime -Descending |
            Select-Object -First 1
        if ($null -ne $window) {
            return $window
        }
        Start-Sleep -Milliseconds 300
    }

    throw "Window matching '$TitleLike' not found within $TimeoutSeconds seconds."
}

function Capture-Window {
    param(
        [System.Diagnostics.Process]$WindowProcess,
        [string]$Path
    )

    $rect = New-Object VisualLoop.RECT
    $dwmOk = [VisualLoop.Win32]::DwmGetWindowAttribute(
        $WindowProcess.MainWindowHandle,
        9,
        [ref]$rect,
        [System.Runtime.InteropServices.Marshal]::SizeOf([type][VisualLoop.RECT])
    ) -eq 0
    if ($dwmOk) {
        $left = $rect.Left
        $top = $rect.Top
        $width = [Math]::Max(1, $rect.Right - $rect.Left)
        $height = [Math]::Max(1, $rect.Bottom - $rect.Top)
    } else {
        [VisualLoop.Win32]::GetClientRect($WindowProcess.MainWindowHandle, [ref]$rect) | Out-Null
        $origin = New-Object VisualLoop.POINT
        $origin.X = 0
        $origin.Y = 0
        [VisualLoop.Win32]::ClientToScreen($WindowProcess.MainWindowHandle, [ref]$origin) | Out-Null
        $left = $origin.X
        $top = $origin.Y
        $width = [Math]::Max(1, $rect.Right - $rect.Left)
        $height = [Math]::Max(1, $rect.Bottom - $rect.Top)
    }

    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }

    return @{
        Left = $left
        Top = $top
        Width = $width
        Height = $height
    }
}

function Get-TerminalCropRect {
    param(
        [int]$Width,
        [int]$Height
    )

    # Fixed layout for terminal-manager default 1280x800 launch.
    # Ratios target the active terminal viewport starting at the prompt row.
    $x = [int][Math]::Round($Width * 0.30)
    $y = [int][Math]::Round($Height * 0.13)
    $w = [int][Math]::Round($Width * 0.69)
    $h = [int][Math]::Round($Height * 0.25)
    return @{ X = $x; Y = $y; Width = $w; Height = $h }
}

function Save-Crop {
    param(
        [string]$SourcePath,
        [string]$DestPath,
        [hashtable]$Rect
    )

    $source = [System.Drawing.Bitmap]::FromFile($SourcePath)
    try {
        $cropRect = New-Object System.Drawing.Rectangle($Rect.X, $Rect.Y, $Rect.Width, $Rect.Height)
        $cropped = $source.Clone($cropRect, $source.PixelFormat)
        try {
            $cropped.Save($DestPath, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally {
            $cropped.Dispose()
        }
    } finally {
        $source.Dispose()
    }
}

function Get-Luminance {
    param([System.Drawing.Color]$Color)
    return (($Color.R * 0.2126) + ($Color.G * 0.7152) + ($Color.B * 0.0722)) / 255.0
}

function Measure-FrameDiff {
    param(
        [string]$PathA,
        [string]$PathB
    )

    $a = [System.Drawing.Bitmap]::FromFile($PathA)
    $b = [System.Drawing.Bitmap]::FromFile($PathB)
    try {
        if ($a.Width -ne $b.Width -or $a.Height -ne $b.Height) {
            throw 'Image dimensions differ.'
        }

        $total = [double]($a.Width * $a.Height)
        $changed = 0.0
        for ($y = 0; $y -lt $a.Height; $y++) {
            for ($x = 0; $x -lt $a.Width; $x++) {
                $ca = $a.GetPixel($x, $y)
                $cb = $b.GetPixel($x, $y)
                $dr = [Math]::Abs([int]$ca.R - [int]$cb.R)
                $dg = [Math]::Abs([int]$ca.G - [int]$cb.G)
                $db = [Math]::Abs([int]$ca.B - [int]$cb.B)
                if (($dr + $dg + $db) -gt 12) {
                    $changed += 1.0
                }
            }
        }

        return $changed / $total
    } finally {
        $a.Dispose()
        $b.Dispose()
    }
}

function Get-RowBands {
    param([string]$Path)

    $image = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $bgSample = $image.GetPixel(
            [Math]::Max(0, $image.Width - 8),
            [Math]::Max(0, $image.Height - 8)
        )
        $rowInfo = @()
        for ($y = 0; $y -lt $image.Height; $y++) {
            $count = 0
            $minX = $image.Width
            $maxX = -1
            for ($x = 0; $x -lt $image.Width; $x++) {
                $pixel = $image.GetPixel($x, $y)
                $dr = [Math]::Abs([int]$pixel.R - [int]$bgSample.R)
                $dg = [Math]::Abs([int]$pixel.G - [int]$bgSample.G)
                $db = [Math]::Abs([int]$pixel.B - [int]$bgSample.B)
                if (($dr + $dg + $db) -gt 42) {
                    $count += 1
                    if ($x -lt $minX) { $minX = $x }
                    if ($x -gt $maxX) { $maxX = $x }
                }
            }
            $rowInfo += [pscustomobject]@{
                Y = $y
                Count = $count
                MinX = $minX
                MaxX = $maxX
            }
        }

        $threshold = [Math]::Max(6, [int]($image.Width * 0.015))
        $bands = @()
        $active = $null
        foreach ($row in $rowInfo) {
            if ($row.Count -ge $threshold) {
                if ($null -eq $active) {
                    $active = [ordered]@{
                        StartY = $row.Y
                        EndY = $row.Y
                        MaxCount = $row.Count
                        MinX = $row.MinX
                        MaxX = $row.MaxX
                    }
                } else {
                    $active.EndY = $row.Y
                    $active.MaxCount = [Math]::Max($active.MaxCount, $row.Count)
                    $active.MinX = [Math]::Min($active.MinX, $row.MinX)
                    $active.MaxX = [Math]::Max($active.MaxX, $row.MaxX)
                }
            } elseif ($null -ne $active) {
                $bands += [pscustomobject]$active
                $active = $null
            }
        }
        if ($null -ne $active) {
            $bands += [pscustomobject]$active
        }

        return $bands
    } finally {
        $image.Dispose()
    }
}

function Get-StartupVerdict {
    param(
        [string[]]$CropPaths,
        [double[]]$DiffRatios
    )

    $analysisCrop = if ($CropPaths.Count -gt 1) { $CropPaths[1] } else { $CropPaths[0] }
    $bands = @(Get-RowBands -Path $analysisCrop)
    $textBands = @(
        $bands | Where-Object {
            $_.StartY -ge 10 -and
            (($_.MaxX - $_.MinX) -gt 100) -and
            $_.MaxCount -gt 40
        }
    )
    $stable = $true
    foreach ($ratio in @($DiffRatios | Select-Object -Last 2)) {
        if ($ratio -gt 0.01) {
            $stable = $false
            break
        }
    }

    $row0Ok = $false
    $row1Ok = $false
    if ($textBands.Count -ge 2) {
        $row0 = $textBands[0]
        $row1 = $textBands[1]
        $row0Width = $row0.MaxX - $row0.MinX
        $row1Width = $row1.MaxX - $row1.MinX
        $row0Ok = $row0.MinX -lt 80 -and $row0Width -gt 140
        $row1Ok = $row1.MinX -lt 80 -and $row1Width -gt 320
    } elseif ($textBands.Count -eq 1) {
        $row0 = $textBands[0]
        $row0Width = $row0.MaxX - $row0.MinX
        $row0Ok = $row0.MinX -lt 80 -and $row0Width -gt 320
        $row1Ok = $true
    }

    return [pscustomobject]@{
        stable = $stable
        row0_ok = $row0Ok
        row1_ok = $row1Ok
        passed = ($stable -and $row0Ok -and $row1Ok)
        bands = $bands
    }
}

Add-Win32Types
Add-Type -AssemblyName System.Drawing

$repoRoot = (Get-Location).Path
$absoluteOutDir = if ([System.IO.Path]::IsPathRooted($OutDir)) {
    $OutDir
} else {
    Join-Path $repoRoot $OutDir
}
New-Directory -Path $absoluteOutDir

$env:TM_TRACE_TERMINAL = '1'
$command = "Set-Location '$repoRoot'; cargo $CargoArgs"
$cargo = Start-Process powershell -ArgumentList '-NoProfile', '-Command', $command -PassThru -WindowStyle Normal

$window = $null
$captures = @()
$crops = @()
$diffRatios = @()
$verdict = $null

try {
    $window = Wait-ForWindow -TitleLike 'terminal manager*' -TimeoutSeconds 60
    [VisualLoop.Win32]::ShowWindowAsync($window.MainWindowHandle, 9) | Out-Null
    [VisualLoop.Win32]::SetForegroundWindow($window.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 2500

    $frameDelayMs = [Math]::Max(600, [int][Math]::Round(($DurationSeconds * 1000.0) / [Math]::Max(1, $FrameCount - 1)))
    for ($i = 0; $i -lt $FrameCount; $i++) {
        $capturePath = Join-Path $absoluteOutDir ("frame-{0:D2}.png" -f $i)
        $rect = Capture-Window -WindowProcess $window -Path $capturePath
        $cropRect = Get-TerminalCropRect -Width $rect.Width -Height $rect.Height
        $cropPath = Join-Path $absoluteOutDir ("frame-{0:D2}-terminal.png" -f $i)
        Save-Crop -SourcePath $capturePath -DestPath $cropPath -Rect $cropRect
        $captures += $capturePath
        $crops += $cropPath

        if ($i -gt 0) {
            $diffRatios += Measure-FrameDiff -PathA $crops[$i - 1] -PathB $cropPath
        }

        if ($i -lt ($FrameCount - 1)) {
            Start-Sleep -Milliseconds $frameDelayMs
        }
    }

    $verdict = Get-StartupVerdict -CropPaths $crops -DiffRatios $diffRatios
} finally {
    Stop-Process -Id $cargo.Id -Force -ErrorAction SilentlyContinue
    Get-Process |
        Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle -like 'terminal manager*' } |
        Stop-Process -Force -ErrorAction SilentlyContinue
}

$result = [pscustomobject]@{
    cwd = $repoRoot
    cargo_pid = $cargo.Id
    duration_seconds = $DurationSeconds
    frame_count = $FrameCount
    captures = $captures
    crops = $crops
    diff_ratios = $diffRatios
    stable = $verdict.stable
    row0_ok = $verdict.row0_ok
    row1_ok = $verdict.row1_ok
    passed = $verdict.passed
    bands = $verdict.bands
}

$jsonPath = Join-Path $absoluteOutDir 'verdict.json'
$result | ConvertTo-Json -Depth 5 | Set-Content -Path $jsonPath
$result | ConvertTo-Json -Depth 5
