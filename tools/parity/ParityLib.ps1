Set-StrictMode -Version Latest

function New-ParityDirectory {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Add-ParityWin32Types {
    if (-not ('GodlyParity.Win32' -as [type])) {
        Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace GodlyParity {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public static class Win32 {
        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

        public class WindowInfo {
            public IntPtr Hwnd;
            public uint ProcessId;
            public string Title;
            public string ClassName;
            public RECT Rect;
            public bool Visible;
        }

        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc enumProc, IntPtr lParam);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextLength(IntPtr hWnd);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder className, int maxCount);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int width, int height, bool repaint);
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int x, int y, int width, int height, uint flags);
        [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
        [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool attach);
        [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
        [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();

        public static WindowInfo[] ListWindows() {
            var windows = new List<WindowInfo>();
            EnumWindows((hWnd, lParam) => {
                var titleLength = GetWindowTextLength(hWnd);
                var title = new StringBuilder(Math.Max(titleLength + 1, 2));
                GetWindowText(hWnd, title, title.Capacity);

                var className = new StringBuilder(256);
                GetClassName(hWnd, className, className.Capacity);

                uint pid;
                GetWindowThreadProcessId(hWnd, out pid);

                RECT rect;
                GetWindowRect(hWnd, out rect);

                windows.Add(new WindowInfo {
                    Hwnd = hWnd,
                    ProcessId = pid,
                    Title = title.ToString(),
                    ClassName = className.ToString(),
                    Rect = rect,
                    Visible = IsWindowVisible(hWnd)
                });
                return true;
            }, IntPtr.Zero);
            return windows.ToArray();
        }
    }
}
"@
    }

    Add-Type -AssemblyName System.Drawing
}

function Wait-ParityWindow {
    param(
        [Parameter(Mandatory)][string]$TitleLike,
        [string]$ProcessName = '',
        [int]$TimeoutSeconds = 45
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $windows = Get-Process |
            Where-Object {
                $_.MainWindowHandle -ne 0 -and
                $_.MainWindowTitle -like $TitleLike -and
                ($ProcessName -eq '' -or $_.ProcessName -like $ProcessName)
            } |
            Sort-Object StartTime -Descending

        $window = @($windows | Select-Object -First 1)
        if ($window.Count -gt 0) {
            return $window[0]
        }

        Start-Sleep -Milliseconds 250
    }

    throw "Window matching '$TitleLike' was not found within $TimeoutSeconds seconds."
}

function Wait-ParityTopLevelWindow {
    param(
        [Parameter(Mandatory)][string]$TitleLike,
        [string]$ProcessName = '',
        [int]$MinWidth = 1,
        [int]$MinHeight = 1,
        [int]$TimeoutSeconds = 45
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $matches = @()
        foreach ($window in [GodlyParity.Win32]::ListWindows()) {
            if (-not $window.Visible -or $window.Title -notlike $TitleLike) {
                continue
            }
            $process = Get-Process -Id ([int]$window.ProcessId) -ErrorAction SilentlyContinue
            if ($null -eq $process) {
                continue
            }
            if ($ProcessName -ne '' -and $process.ProcessName -notlike $ProcessName) {
                continue
            }

            $width = [Math]::Max(1, $window.Rect.Right - $window.Rect.Left)
            $height = [Math]::Max(1, $window.Rect.Bottom - $window.Rect.Top)
            if ($width -lt $MinWidth -or $height -lt $MinHeight) {
                continue
            }
            $matches += [pscustomobject]@{
                Id = [int]$window.ProcessId
                ProcessName = $process.ProcessName
                MainWindowHandle = $window.Hwnd
                MainWindowTitle = $window.Title
                ClassName = $window.ClassName
                Width = $width
                Height = $height
                Area = ($width * $height)
            }
        }

        if ($matches.Count -gt 0) {
            return @($matches | Sort-Object Area -Descending | Select-Object -First 1)[0]
        }

        Start-Sleep -Milliseconds 250
    }

    throw "Top-level window matching '$TitleLike' was not found within $TimeoutSeconds seconds."
}

function Wait-ParityProcessWindow {
    param(
        [Parameter(Mandatory)][System.Diagnostics.Process]$Process,
        [int]$MinWidth = 1,
        [int]$MinHeight = 1,
        [int]$TimeoutSeconds = 45
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $Process.Refresh()
        if (-not $Process.HasExited) {
            $matches = @()
            foreach ($window in [GodlyParity.Win32]::ListWindows()) {
                if (-not $window.Visible -or [int]$window.ProcessId -ne $Process.Id) {
                    continue
                }

                $width = [Math]::Max(1, $window.Rect.Right - $window.Rect.Left)
                $height = [Math]::Max(1, $window.Rect.Bottom - $window.Rect.Top)
                if ($width -lt $MinWidth -or $height -lt $MinHeight) {
                    continue
                }
                $matches += [pscustomobject]@{
                    Id = $Process.Id
                    ProcessName = $Process.ProcessName
                    MainWindowHandle = $window.Hwnd
                    MainWindowTitle = $window.Title
                    ClassName = $window.ClassName
                    Width = $width
                    Height = $height
                    Area = ($width * $height)
                }
            }

            if ($matches.Count -gt 0) {
                return @($matches | Sort-Object Area -Descending | Select-Object -First 1)[0]
            }
        }
        Start-Sleep -Milliseconds 250
    }

    throw "Process $($Process.Id) did not create a main window within $TimeoutSeconds seconds."
}

function Set-ParityWindowBounds {
    param(
        [Parameter(Mandatory)]$WindowProcess,
        [int]$X,
        [int]$Y,
        [int]$Width,
        [int]$Height
    )

    $targetHwnd = $WindowProcess.MainWindowHandle
    $targetProcessId = [uint32]0
    $targetThread = [GodlyParity.Win32]::GetWindowThreadProcessId(
        $targetHwnd,
        [ref]$targetProcessId
    )
    $foregroundHwnd = [GodlyParity.Win32]::GetForegroundWindow()
    $foregroundProcessId = [uint32]0
    $foregroundThread = [GodlyParity.Win32]::GetWindowThreadProcessId(
        $foregroundHwnd,
        [ref]$foregroundProcessId
    )
    $currentThread = [GodlyParity.Win32]::GetCurrentThreadId()
    $attachedTarget = $false
    $attachedForeground = $false

    try {
        if ($targetThread -ne 0 -and $targetThread -ne $currentThread) {
            $attachedTarget = [GodlyParity.Win32]::AttachThreadInput($currentThread, $targetThread, $true)
        }
        if ($foregroundThread -ne 0 -and $foregroundThread -ne $currentThread -and $foregroundThread -ne $targetThread) {
            $attachedForeground = [GodlyParity.Win32]::AttachThreadInput($currentThread, $foregroundThread, $true)
        }

        [GodlyParity.Win32]::ShowWindowAsync($targetHwnd, 9) | Out-Null
        [GodlyParity.Win32]::MoveWindow(
            $targetHwnd,
            $X,
            $Y,
            $Width,
            $Height,
            $true
        ) | Out-Null
        [GodlyParity.Win32]::SetWindowPos(
            $targetHwnd,
            ([IntPtr]::new(-1)),
            0,
            0,
            0,
            0,
            0x0043
        ) | Out-Null
        [GodlyParity.Win32]::BringWindowToTop($targetHwnd) | Out-Null
        [GodlyParity.Win32]::SetForegroundWindow($targetHwnd) | Out-Null
    } finally {
        if ($attachedForeground) {
            [GodlyParity.Win32]::AttachThreadInput($currentThread, $foregroundThread, $false) | Out-Null
        }
        if ($attachedTarget) {
            [GodlyParity.Win32]::AttachThreadInput($currentThread, $targetThread, $false) | Out-Null
        }
    }
}

function Get-ParityWindowRect {
    param([Parameter(Mandatory)]$WindowProcess)

    $rect = New-Object GodlyParity.RECT
    [GodlyParity.Win32]::GetWindowRect($WindowProcess.MainWindowHandle, [ref]$rect) | Out-Null
    $width = [Math]::Max(1, $rect.Right - $rect.Left)
    $height = [Math]::Max(1, $rect.Bottom - $rect.Top)

    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Width = $width
        Height = $height
    }
}

function Test-ParityBitmapLooksBlank {
    param([Parameter(Mandatory)][System.Drawing.Bitmap]$Bitmap)

    $first = $Bitmap.GetPixel(0, 0)
    $stepX = [Math]::Max(1, [int]($Bitmap.Width / 96))
    $stepY = [Math]::Max(1, [int]($Bitmap.Height / 64))
    for ($y = 0; $y -lt $Bitmap.Height; $y += $stepY) {
        for ($x = 0; $x -lt $Bitmap.Width; $x += $stepX) {
            $pixel = $Bitmap.GetPixel($x, $y)
            $dr = [Math]::Abs([int]$pixel.R - [int]$first.R)
            $dg = [Math]::Abs([int]$pixel.G - [int]$first.G)
            $db = [Math]::Abs([int]$pixel.B - [int]$first.B)
            if (($dr + $dg + $db) -gt 12) {
                return $false
            }
        }
    }

    return $true
}

function Capture-ParityWindow {
    param(
        [Parameter(Mandatory)]$WindowProcess,
        [Parameter(Mandatory)][string]$Path,
        [ValidateSet('Window', 'Screen')][string]$Mode = 'Window'
    )

    $rect = Get-ParityWindowRect -WindowProcess $WindowProcess
    $bitmap = New-Object System.Drawing.Bitmap $rect.Width, $rect.Height
    $graphics = $null
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $captured = $false
        if ($Mode -eq 'Window') {
            $hdc = $graphics.GetHdc()
            try {
                $captured = [GodlyParity.Win32]::PrintWindow(
                    $WindowProcess.MainWindowHandle,
                    $hdc,
                    2
                )
            } finally {
                $graphics.ReleaseHdc($hdc)
            }
            if ($captured -and (Test-ParityBitmapLooksBlank -Bitmap $bitmap)) {
                $captured = $false
            }
        }

        if (-not $captured) {
            [GodlyParity.Win32]::BringWindowToTop($WindowProcess.MainWindowHandle) | Out-Null
            [GodlyParity.Win32]::SetForegroundWindow($WindowProcess.MainWindowHandle) | Out-Null
            Start-Sleep -Milliseconds 150
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        }
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        if ($null -ne $graphics) {
            $graphics.Dispose()
        }
        $bitmap.Dispose()
    }

    return $rect
}

function ConvertTo-ParityRect {
    param([Parameter(Mandatory)][string]$Spec)

    $parts = @($Spec.Split(',') | ForEach-Object { $_.Trim() })
    if ($parts.Count -ne 4) {
        throw "Crop spec '$Spec' must be x,y,width,height."
    }

    [pscustomobject]@{
        X = [int]$parts[0]
        Y = [int]$parts[1]
        Width = [int]$parts[2]
        Height = [int]$parts[3]
    }
}

function Get-DefaultTerminalManagerCropRect {
    param(
        [int]$Width,
        [int]$Height
    )

    [pscustomobject]@{
        X = [Math]::Min(241, [Math]::Max(0, $Width - 1))
        Y = [Math]::Min(333, [Math]::Max(0, $Height - 1))
        Width = [Math]::Min(832, [Math]::Max(1, $Width - [Math]::Min(241, [Math]::Max(0, $Width - 1))))
        Height = [Math]::Min(216, [Math]::Max(1, $Height - [Math]::Min(333, [Math]::Max(0, $Height - 1))))
    }
}

function Get-DefaultWindowsTerminalCropRect {
    param(
        [int]$Width,
        [int]$Height,
        [int]$CropWidth,
        [int]$CropHeight
    )

    $x = 36
    $y = 131
    if (($x + $CropWidth) -gt $Width) {
        $x = [Math]::Max(0, $Width - $CropWidth)
    }
    if (($y + $CropHeight) -gt $Height) {
        $y = [Math]::Max(0, $Height - $CropHeight)
    }

    [pscustomobject]@{
        X = $x
        Y = $y
        Width = $CropWidth
        Height = $CropHeight
    }
}

function Save-ParityImageCrop {
    param(
        [Parameter(Mandatory)][string]$SourcePath,
        [Parameter(Mandatory)][string]$DestPath,
        [Parameter(Mandatory)]$Rect
    )

    $source = [System.Drawing.Bitmap]::FromFile($SourcePath)
    try {
        if ($Rect.X -lt 0 -or $Rect.Y -lt 0 -or $Rect.Width -le 0 -or $Rect.Height -le 0) {
            throw "Invalid crop rect $($Rect | ConvertTo-Json -Compress)."
        }
        if (($Rect.X + $Rect.Width) -gt $source.Width -or ($Rect.Y + $Rect.Height) -gt $source.Height) {
            throw "Crop rect $($Rect | ConvertTo-Json -Compress) exceeds image $SourcePath ($($source.Width)x$($source.Height))."
        }

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

function Compare-ParityImages {
    param(
        [Parameter(Mandatory)][string]$ReferencePath,
        [Parameter(Mandatory)][string]$ActualPath,
        [Parameter(Mandatory)][string]$DiffPath,
        [int]$Tolerance = 8
    )

    $reference = [System.Drawing.Bitmap]::FromFile($ReferencePath)
    $actual = [System.Drawing.Bitmap]::FromFile($ActualPath)
    $diff = $null
    try {
        if ($reference.Width -ne $actual.Width -or $reference.Height -ne $actual.Height) {
            throw "Image dimensions differ: reference=$($reference.Width)x$($reference.Height), actual=$($actual.Width)x$($actual.Height)."
        }

        $diff = New-Object System.Drawing.Bitmap $reference.Width, $reference.Height
        $total = [int64]($reference.Width * $reference.Height)
        $changed = [int64]0
        $maxDelta = 0
        $sumDelta = 0.0
        $minX = $reference.Width
        $minY = $reference.Height
        $maxX = -1
        $maxY = -1

        for ($y = 0; $y -lt $reference.Height; $y++) {
            for ($x = 0; $x -lt $reference.Width; $x++) {
                $a = $reference.GetPixel($x, $y)
                $b = $actual.GetPixel($x, $y)
                $dr = [Math]::Abs([int]$a.R - [int]$b.R)
                $dg = [Math]::Abs([int]$a.G - [int]$b.G)
                $db = [Math]::Abs([int]$a.B - [int]$b.B)
                $pixelMax = [Math]::Max($dr, [Math]::Max($dg, $db))
                $pixelMean = ($dr + $dg + $db) / 3.0
                $sumDelta += $pixelMean
                $maxDelta = [Math]::Max($maxDelta, $pixelMax)

                if ($pixelMax -gt $Tolerance) {
                    $changed += 1
                    $minX = [Math]::Min($minX, $x)
                    $minY = [Math]::Min($minY, $y)
                    $maxX = [Math]::Max($maxX, $x)
                    $maxY = [Math]::Max($maxY, $y)
                    $hot = [Math]::Min(255, 80 + ($pixelMax * 2))
                    $diff.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255, $hot, 0, 0))
                } else {
                    $gray = [int](($a.R * 0.2126) + ($a.G * 0.7152) + ($a.B * 0.0722))
                    $dim = [Math]::Max(0, [Math]::Min(255, [int]($gray * 0.18)))
                    $diff.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255, $dim, $dim, $dim))
                }
            }
        }

        $diff.Save($DiffPath, [System.Drawing.Imaging.ImageFormat]::Png)

        $bbox = $null
        if ($changed -gt 0) {
            $bbox = [pscustomobject]@{
                X = $minX
                Y = $minY
                Width = ($maxX - $minX + 1)
                Height = ($maxY - $minY + 1)
            }
        }

        [pscustomobject]@{
            reference = $ReferencePath
            actual = $ActualPath
            diff = $DiffPath
            width = $reference.Width
            height = $reference.Height
            tolerance = $Tolerance
            total_pixels = $total
            changed_pixels = $changed
            changed_ratio = if ($total -eq 0) { 0.0 } else { [double]$changed / [double]$total }
            max_channel_delta = $maxDelta
            mean_channel_delta = if ($total -eq 0) { 0.0 } else { $sumDelta / [double]$total }
            difference_bounds = $bbox
        }
    } finally {
        if ($null -ne $diff) {
            $diff.Dispose()
        }
        $reference.Dispose()
        $actual.Dispose()
    }
}

function Invoke-ParitySelfTest {
    param([Parameter(Mandatory)][string]$OutDir)

    New-ParityDirectory -Path $OutDir
    $aPath = Join-Path $OutDir 'selftest-a.png'
    $bPath = Join-Path $OutDir 'selftest-b.png'
    $cropPath = Join-Path $OutDir 'selftest-crop.png'
    $diffPath = Join-Path $OutDir 'selftest-diff.png'

    $a = New-Object System.Drawing.Bitmap 4, 4
    $b = New-Object System.Drawing.Bitmap 4, 4
    try {
        for ($y = 0; $y -lt 4; $y++) {
            for ($x = 0; $x -lt 4; $x++) {
                $a.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255, 10, 10, 10))
                $b.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255, 10, 10, 10))
            }
        }
        $b.SetPixel(2, 1, [System.Drawing.Color]::FromArgb(255, 40, 10, 10))
        $a.Save($aPath, [System.Drawing.Imaging.ImageFormat]::Png)
        $b.Save($bPath, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $a.Dispose()
        $b.Dispose()
    }

    Save-ParityImageCrop -SourcePath $aPath -DestPath $cropPath -Rect ([pscustomobject]@{
        X = 1
        Y = 1
        Width = 2
        Height = 2
    })

    $result = Compare-ParityImages -ReferencePath $aPath -ActualPath $bPath -DiffPath $diffPath -Tolerance 8
    if ($result.changed_pixels -ne 1 -or $result.max_channel_delta -ne 30) {
        throw "Parity self-test failed: $($result | ConvertTo-Json -Compress)"
    }

    $tmCrop = Get-DefaultTerminalManagerCropRect -Width 1280 -Height 800
    if ($tmCrop.X -ne 241 -or $tmCrop.Y -ne 333 -or $tmCrop.Width -ne 832 -or $tmCrop.Height -ne 216) {
        throw "Terminal-manager default crop self-test failed: $($tmCrop | ConvertTo-Json -Compress)"
    }

    $wtCrop = Get-DefaultWindowsTerminalCropRect -Width 1280 -Height 800 -CropWidth 832 -CropHeight 216
    if ($wtCrop.X -ne 36 -or $wtCrop.Y -ne 131 -or $wtCrop.Width -ne 832 -or $wtCrop.Height -ne 216) {
        throw "Windows Terminal default crop self-test failed: $($wtCrop | ConvertTo-Json -Compress)"
    }

    return $result
}
