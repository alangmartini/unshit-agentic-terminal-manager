#Requires -Version 5.1

Set-StrictMode -Version Latest

$script:DesktopRegressionSuites = [ordered]@{}

function Initialize-DesktopRegressionWin32 {
    if (-not ('DesktopRegressionWin32' -as [type])) {
        Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class DesktopRegressionWin32 {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(
        IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint dwData, int dwExtraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, int dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();

    public const int SM_CXSCREEN = 0;
    public const int SM_CYSCREEN = 1;

    public const uint SWP_NOACTIVATE = 0x0010;
    public const uint SWP_NOZORDER = 0x0004;
    public const uint SWP_SHOWWINDOW = 0x0040;

    public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    public const uint MOUSEEVENTF_LEFTUP = 0x0004;

    public const uint KEYEVENTF_KEYUP = 0x0002;
    public const byte VK_LWIN = 0x5B;
    public const byte VK_LEFT = 0x25;

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public static readonly IntPtr HWND_TOP = IntPtr.Zero;
    public static readonly IntPtr DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = new IntPtr(-4);
    public static readonly IntPtr DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE = new IntPtr(-3);
}
'@
    }

    Enable-DesktopRegressionDpiAwareness
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName System.Windows.Forms
}

function Enable-DesktopRegressionDpiAwareness {
    try {
        if ([DesktopRegressionWin32]::SetProcessDpiAwarenessContext([DesktopRegressionWin32]::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)) {
            return
        }
    } catch {}
    try {
        if ([DesktopRegressionWin32]::SetProcessDpiAwarenessContext([DesktopRegressionWin32]::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE)) {
            return
        }
    } catch {}
    try {
        [void][DesktopRegressionWin32]::SetProcessDPIAware()
    } catch {}
}

function Register-DesktopRegressionSuite {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Title,
        [Parameter(Mandatory = $true)][string]$Covers,
        [string[]]$Tags = @(),
        [Parameter(Mandatory = $true)][scriptblock]$ScriptBlock
    )

    if ($script:DesktopRegressionSuites.Contains($Name)) {
        throw "Desktop regression suite '$Name' is already registered"
    }

    $script:DesktopRegressionSuites[$Name] = [pscustomobject]@{
        Name = $Name
        Title = $Title
        Covers = $Covers
        Tags = $Tags
        ScriptBlock = $ScriptBlock
    }
}

function Get-DesktopRegressionSuites {
    return @($script:DesktopRegressionSuites.Values)
}

function New-DesktopRegressionContext {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$ExePath,
        [Parameter(Mandatory = $true)][string]$ArtifactsRoot,
        [double]$Tolerance = 2.0,
        [int]$DragDelta = 220,
        [double]$SnapLitRatioThreshold = 0.01,
        [double]$SnapMidLitRatioThreshold = 0.005,
        [string]$SnapShell = "bash",
        [int]$SnapFillLines = 200,
        [int]$SnapTabbarPx = 88,
        [int]$SnapStatusbarPx = 32,
        [int]$SnapSidebarPx = 252,
        [int]$SnapStripeHeightPx = 12
    )

    $runId = Get-Date -Format "yyyyMMdd-HHmmss"
    $runArtifactsDir = Join-Path $ArtifactsRoot "windows\desktop-regression\$runId"
    if (-not (Test-Path $runArtifactsDir)) {
        New-Item -ItemType Directory -Path $runArtifactsDir | Out-Null
    }

    return [pscustomobject]@{
        RepoRoot = $RepoRoot
        ExePath = $ExePath
        ArtifactsRoot = $ArtifactsRoot
        RunArtifactsDir = $runArtifactsDir
        RunId = $runId
        Tolerance = $Tolerance
        DragDelta = $DragDelta
        SnapLitRatioThreshold = $SnapLitRatioThreshold
        SnapMidLitRatioThreshold = $SnapMidLitRatioThreshold
        SnapShell = $SnapShell
        SnapFillLines = $SnapFillLines
        SnapTabbarPx = $SnapTabbarPx
        SnapStatusbarPx = $SnapStatusbarPx
        SnapSidebarPx = $SnapSidebarPx
        SnapStripeHeightPx = $SnapStripeHeightPx
    }
}

function Stop-DesktopRegressionStaleApps {
    Get-Process -Name terminal-manager -ErrorAction SilentlyContinue |
        Stop-Process -ErrorAction SilentlyContinue
}

function Start-DesktopRegressionApp {
    param([Parameter(Mandatory = $true)]$Context)

    Stop-DesktopRegressionStaleApps

    if (-not (Test-Path $Context.ExePath)) {
        throw "Missing built binary: $($Context.ExePath)"
    }

    $proc = Start-Process -FilePath $Context.ExePath -WorkingDirectory $Context.RepoRoot -PassThru
    Write-Host "started pid=$($proc.Id)"

    $hwnd = Get-DesktopRegressionProcessWindow -ProcessId $proc.Id
    Start-Sleep -Milliseconds 500

    return [pscustomobject]@{
        Process = $proc
        WindowHandle = $hwnd
    }
}

function Stop-DesktopRegressionApp {
    param($Session)

    if ($Session -and $Session.Process -and -not $Session.Process.HasExited) {
        Stop-Process -Id $Session.Process.Id -ErrorAction SilentlyContinue
    }
}

function Get-DesktopRegressionProcessWindow {
    param([int]$ProcessId, [int]$TimeoutMs = 10000)

    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while ((Get-Date) -lt $deadline) {
        $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if ($proc) {
            $windows = New-Object 'System.Collections.Generic.List[System.IntPtr]'
            $callback = [DesktopRegressionWin32+EnumWindowsProc]{
                param([IntPtr]$hWnd, [IntPtr]$lParam)
                [uint32]$ownerPid = 0
                [void][DesktopRegressionWin32]::GetWindowThreadProcessId($hWnd, [ref]$ownerPid)
                if ($ownerPid -eq $ProcessId -and [DesktopRegressionWin32]::IsWindowVisible($hWnd)) {
                    $windows.Add($hWnd)
                }
                return $true
            }
            [void][DesktopRegressionWin32]::EnumWindows($callback, [IntPtr]::Zero)

            foreach ($candidate in $windows) {
                $title = New-Object System.Text.StringBuilder 512
                [void][DesktopRegressionWin32]::GetWindowText($candidate, $title, $title.Capacity)
                if ($title.ToString() -like "*terminal.mgr*") {
                    return $candidate
                }
            }

            if ($proc.MainWindowHandle -ne 0) {
                return $proc.MainWindowHandle
            }
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Window did not appear for pid=$ProcessId"
}

function Get-DesktopRegressionScreenSize {
    return [pscustomobject]@{
        Width = [DesktopRegressionWin32]::GetSystemMetrics([DesktopRegressionWin32]::SM_CXSCREEN)
        Height = [DesktopRegressionWin32]::GetSystemMetrics([DesktopRegressionWin32]::SM_CYSCREEN)
    }
}

function Get-DesktopRegressionRect {
    param([IntPtr]$Handle)

    $rect = New-Object DesktopRegressionWin32+RECT
    if (-not [DesktopRegressionWin32]::GetWindowRect($Handle, [ref]$rect)) {
        throw "GetWindowRect failed"
    }
    return $rect
}

function Format-DesktopRegressionRect {
    param($Rect)

    return ("L{0} T{1} R{2} B{3} W{4} H{5}" -f `
        $Rect.Left, $Rect.Top, $Rect.Right, $Rect.Bottom, `
        ($Rect.Right - $Rect.Left), ($Rect.Bottom - $Rect.Top))
}

function Set-DesktopRegressionWindowRect {
    param(
        [IntPtr]$Handle,
        [int]$X,
        [int]$Y,
        [int]$Width,
        [int]$Height
    )

    [void][DesktopRegressionWin32]::SetWindowPos(
        $Handle,
        [DesktopRegressionWin32]::HWND_TOP,
        $X,
        $Y,
        $Width,
        $Height,
        [DesktopRegressionWin32]::SWP_NOACTIVATE -bor
            [DesktopRegressionWin32]::SWP_NOZORDER -bor
            [DesktopRegressionWin32]::SWP_SHOWWINDOW)
}

function Focus-DesktopRegressionWindow {
    param([IntPtr]$Handle)

    $r = Get-DesktopRegressionRect -Handle $Handle
    $clickX = [int](($r.Left + $r.Right) / 2)
    $clickY = $r.Top + 8
    [void][DesktopRegressionWin32]::SetCursorPos($clickX, $clickY)
    Start-Sleep -Milliseconds 50
    [void][DesktopRegressionWin32]::mouse_event([DesktopRegressionWin32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 30
    [void][DesktopRegressionWin32]::mouse_event([DesktopRegressionWin32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 200
    [void][DesktopRegressionWin32]::SetForegroundWindow($Handle)
    Start-Sleep -Milliseconds 250
}

function Assert-DesktopRegressionClose {
    param([double]$Actual, [double]$Expected, [double]$Tolerance, [string]$Name)

    if ([Math]::Abs($Actual - $Expected) -gt $Tolerance) {
        throw "$Name differs by $([Math]::Abs($Actual - $Expected)) > $Tolerance"
    }
}

function Assert-DesktopRegressionTrue {
    param([bool]$Condition, [string]$Message)

    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-DesktopRegressionLeftEdgeDrag {
    param(
        [IntPtr]$Handle,
        [int]$FromY,
        [int]$FromX,
        [int]$ToX
    )

    if ($FromX -eq $ToX) {
        throw "invalid drag distance"
    }
    [void][DesktopRegressionWin32]::SetCursorPos($FromX, $FromY)
    Start-Sleep -Milliseconds 40
    [void][DesktopRegressionWin32]::mouse_event([DesktopRegressionWin32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    $direction = if ($ToX -gt $FromX) { 1 } else { -1 }
    for ($x = $FromX; ; $x += (5 * $direction)) {
        [void][DesktopRegressionWin32]::SetCursorPos($x, $FromY)
        Start-Sleep -Milliseconds 6
        if (($direction -eq 1 -and $x -ge $ToX) -or ($direction -eq -1 -and $x -le $ToX)) {
            break
        }
    }
    [void][DesktopRegressionWin32]::SetCursorPos($ToX, $FromY)
    [void][DesktopRegressionWin32]::mouse_event([DesktopRegressionWin32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 250
}

function New-DesktopRegressionArtifactPath {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$SuiteName,
        [Parameter(Mandatory = $true)][string]$Name,
        [string]$Extension = "png"
    )

    $safeName = $Name -replace '[^a-zA-Z0-9_.-]', '-'
    return Join-Path $Context.RunArtifactsDir "$SuiteName-$safeName.$Extension"
}

function Capture-DesktopRegressionScreen {
    param([string]$Path)

    $screen = Get-DesktopRegressionScreenSize
    $bmp = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
    $bmp.Save($Path)
    $g.Dispose()
    $bmp.Dispose()
}

function Send-DesktopRegressionWinLeft {
    [void][DesktopRegressionWin32]::keybd_event([DesktopRegressionWin32]::VK_LWIN, 0, 0, 0)
    Start-Sleep -Milliseconds 30
    [void][DesktopRegressionWin32]::keybd_event([DesktopRegressionWin32]::VK_LEFT, 0, 0, 0)
    Start-Sleep -Milliseconds 30
    [void][DesktopRegressionWin32]::keybd_event([DesktopRegressionWin32]::VK_LEFT, 0, [DesktopRegressionWin32]::KEYEVENTF_KEYUP, 0)
    Start-Sleep -Milliseconds 30
    [void][DesktopRegressionWin32]::keybd_event([DesktopRegressionWin32]::VK_LWIN, 0, [DesktopRegressionWin32]::KEYEVENTF_KEYUP, 0)
}

function Send-DesktopRegressionFillCommand {
    param([string]$Shell, [int]$Lines)

    $cmd = switch ($Shell) {
        "bash"  { "yes XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX | head -$Lines" }
        "pwsh"  { "1..$Lines | %{'XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX'}" }
        "cmd"   { "for /L %i in (1,1,$Lines) do @echo XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX" }
        default { "yes XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX | head -$Lines" }
    }
    [System.Windows.Forms.SendKeys]::SendWait("$cmd~")
}

function Send-DesktopRegressionClearCommand {
    param([string]$Shell)

    $cmd = switch ($Shell) {
        "bash"  { "clear" }
        "pwsh"  { "Clear-Host" }
        "cmd"   { "cls" }
        default { "clear" }
    }
    [System.Windows.Forms.SendKeys]::SendWait("$cmd~")
}

function Get-DesktopRegressionStripeLitRatio {
    param(
        [System.Drawing.Bitmap]$Bitmap,
        [int]$X,
        [int]$Y,
        [int]$Width,
        [int]$Height,
        [int]$LitSumThreshold = 240
    )

    if ($Width -le 0 -or $Height -le 0) { return 0.0 }
    $maxX = [Math]::Min($X + $Width, $Bitmap.Width) - 1
    $maxY = [Math]::Min($Y + $Height, $Bitmap.Height) - 1
    if ($maxX -lt $X -or $maxY -lt $Y) { return 0.0 }

    $lit = 0
    $total = 0
    for ($iy = $Y; $iy -le $maxY; $iy++) {
        for ($ix = $X; $ix -le $maxX; $ix++) {
            $px = $Bitmap.GetPixel($ix, $iy)
            if (($px.R + $px.G + $px.B) -ge $LitSumThreshold) { $lit++ }
            $total++
        }
    }
    if ($total -eq 0) { return 0.0 }
    return [double]$lit / [double]$total
}

function Get-DesktopRegressionMaxStripeLitRatio {
    param(
        [System.Drawing.Bitmap]$Bitmap,
        [int]$X,
        [int]$Y,
        [int]$Width,
        [int]$Height,
        [int]$StripeHeight,
        [int]$StepPx
    )

    if ($Width -le 0 -or $Height -le 0 -or $StripeHeight -le 0) { return 0.0 }
    $maxLit = 0.0
    $endY = $Y + $Height - $StripeHeight
    for ($iy = $Y; $iy -le $endY; $iy += [Math]::Max(1, $StepPx)) {
        $lit = Get-DesktopRegressionStripeLitRatio `
            -Bitmap $Bitmap `
            -X $X `
            -Y $iy `
            -Width $Width `
            -Height $StripeHeight
        if ($lit -gt $maxLit) { $maxLit = $lit }
    }
    return $maxLit
}
