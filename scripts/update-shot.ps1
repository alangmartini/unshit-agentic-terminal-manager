#Requires -Version 5.1
<#
.SYNOPSIS
  Drive the self-update flow end to end against a fake release feed and capture it.

.DESCRIPTION
  Verification shot for Settings > Updates and the update-available dialog
  (specs/self-update.md). No network is involved: the script writes a fake
  GitHub `releases/latest` JSON to a temp dir advertising version 99.0.0 with
  a `file://` asset URL that points at a harmless stand-in installer (a copy
  of hostname.exe, size and sha256 digest included), and hands it to the app
  through TM_UPDATE_FEED_URL. TM_UPDATE_INSTALL_SCOPE=user makes the debug
  build behave like a registered per-user install so the in-place install
  path is exercised; pass -Unmanaged for the "open release page" variant.

  Modes:
    dialog    (default) the delayed startup check runs after 500 ms and the
              once-per-version offer dialog is captured.
    settings  Settings > Updates opens via TM_STARTUP_DISPATCH, a manual
              check runs, and the section is captured with its install button.
    install   like settings, then `update.install`: the app downloads the
              stand-in, verifies it, persists the layout, launches the
              stand-in installer with the silent hand-off switches, shuts its
              isolated daemon down and exits. The script asserts the exit, the
              telemetry chain in update-events.jsonl and that workspaces.json
              still carries the tab. Nothing is installed: hostname.exe just
              prints and exits.

  Drives the app through TM_STARTUP_DISPATCH (no synthesized input, no focus
  stealing) and captures with PrintWindow(PW_RENDERFULLCONTENT). Runs under a
  throwaway TM_PROFILE so the installed app's daemon, sessions and config are
  never touched. Afterwards it prints the `update.*` lines from the isolated
  profile's update-events.jsonl, which is the "did telemetry land" check.

.EXAMPLE
  pwsh scripts/update-shot.ps1
  pwsh scripts/update-shot.ps1 -Mode settings -Out update-settings.png
  pwsh scripts/update-shot.ps1 -Mode install
#>
[CmdletBinding()]
param(
    [ValidateSet('dialog', 'settings', 'install')]
    [string]$Mode = 'dialog',
    [string]$Out = "update-shot.png",
    # Behave like a copy that was not set up by the installer.
    [switch]$Unmanaged,
    [string]$ExeDir = "",
    # Use a real feed (e.g. the GitHub releases/latest URL) instead of the fake
    # v99.0.0 one; only meaningful with -Mode settings (check, no install).
    [string]$FeedUrl = "",
    [int]$SettleMs = 7000,
    [int]$Width = 1000,
    [int]$Height = 640
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class UpdateShotWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
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
[UpdateShotWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) {
    if ($env:CARGO_TARGET_DIR) { $ExeDir = Join-Path $env:CARGO_TARGET_DIR 'debug' }
    else { $ExeDir = Join-Path $repoRoot 'target\debug' }
}
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $repoRoot $Out }

function To-FileUrl([string]$path) {
    return 'file:///' + ($path -replace '\\', '/')
}

# --- fake feed: version 99.0.0 with a stand-in installer -------------------
$feedDir = Join-Path $env:TEMP ("tm-update-feed-{0}" -f $PID)
New-Item -ItemType Directory -Force -Path $feedDir | Out-Null
$fakeInstaller = Join-Path $feedDir 'terminal-manager-99.0.0-setup.exe'
Copy-Item (Join-Path $env:SystemRoot 'System32\hostname.exe') $fakeInstaller -Force
$digest = (Get-FileHash -Algorithm SHA256 -LiteralPath $fakeInstaller).Hash.ToLower()
$size = (Get-Item -LiteralPath $fakeInstaller).Length
$feed = @{
    tag_name   = 'v99.0.0'
    name       = '99.0.0 (fake feed)'
    html_url   = 'https://github.com/alangmartini/unshit-agentic-terminal-manager/releases'
    draft      = $false
    prerelease = $false
    assets     = @(
        @{
            name                 = 'terminal-manager-99.0.0-setup.exe'
            browser_download_url = (To-FileUrl $fakeInstaller)
            size                 = $size
            digest               = "sha256:$digest"
        }
    )
}
$feedPath = Join-Path $feedDir 'latest.json'
# UTF-8 without a BOM: PowerShell 5's `-Encoding UTF8` adds one, which is not JSON.
[System.IO.File]::WriteAllText($feedPath, ($feed | ConvertTo-Json -Depth 4), (New-Object System.Text.UTF8Encoding $false))

switch ($Mode) {
    'dialog'   { $dispatch = ''; $startupDelay = 500 }
    'settings' { $dispatch = 'settings.section:updates;update.check'; $startupDelay = 600000 }
    'install'  { $dispatch = 'settings.section:updates;update.install'; $startupDelay = 600000 }
}

$launched = $null
$isolation = Enter-TmIsolation -Tag 'updshot'
$errLog = "$Out.err.txt"
if ($FeedUrl) { $env:TM_UPDATE_FEED_URL = $FeedUrl } else { $env:TM_UPDATE_FEED_URL = To-FileUrl $feedPath }
$env:TM_UPDATE_STARTUP_DELAY_MS = "$startupDelay"
if ($Unmanaged) { $env:TM_UPDATE_INSTALL_SCOPE = 'none' } else { $env:TM_UPDATE_INSTALL_SCOPE = 'user' }
if ($dispatch) { $env:TM_STARTUP_DISPATCH = $dispatch }
# The isolated profile's data dir (downloads land there) is not under
# TM_CONFIG_DIR; remember it so the run leaves nothing behind.
$dataDir = Join-Path $env:LOCALAPPDATA ("com.godly.terminal.{0}" -f $isolation.Token)
try {
    try {
        $proc = Start-Process -FilePath $exe -WorkingDirectory $repoRoot -PassThru -RedirectStandardError $errLog
        $null = $proc.Handle   # cache the handle so ExitCode is readable after exit
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
    }
    $launched = $proc
    Write-Host "Launched pid=$($proc.Id) mode=$Mode unmanaged=$Unmanaged feed=$feedPath"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) {
            if ($Mode -eq 'install') { break }
            throw "Process exited early (code $($proc.ExitCode)); see $errLog"
        }
        $h = [UpdateShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }

    if ($Mode -eq 'install') {
        # The app downloads the stand-in, hands off and exits on its own.
        $exited = $proc.WaitForExit(40000)
        if (-not $exited) { throw 'install mode: the app did not exit within 40s after update.install' }
        Write-Host "App exited on its own with code $($proc.ExitCode) after $([int]$clock.Elapsed.TotalSeconds)s"
    } else {
        if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }
        # Deterministic size, no activation (SWP_NOACTIVATE = 0x10, SWP_NOZORDER = 0x4).
        [UpdateShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
        Start-Sleep -Milliseconds $SettleMs
        # The first handle can be the splash window; resolve again after settling.
        $again = [UpdateShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($again -ne [IntPtr]::Zero) { $handle = $again }
        [UpdateShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
        Start-Sleep -Milliseconds 1200

        $rect = New-Object UpdateShotWin+RECT
        [UpdateShotWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
        $w = $rect.Right - $rect.Left
        $hh = $rect.Bottom - $rect.Top
        if ($w -lt 200 -or $hh -lt 200) { throw "Window rect too small ($w x $hh); handle is not the main window" }

        $bmp = New-Object System.Drawing.Bitmap $w, $hh
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        [UpdateShotWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
        $g.ReleaseHdc($hdc); $g.Dispose()
        # A blank surface must fail loudly instead of passing as verified.
        $colors = New-Object 'System.Collections.Generic.HashSet[int]'
        for ($y = 0; $y -lt $hh; $y += 16) { for ($x = 0; $x -lt $w; $x += 16) { [void]$colors.Add($bmp.GetPixel($x, $y).ToArgb()) } }
        $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        if ($colors.Count -lt 12) { throw "Capture looks blank ($($colors.Count) distinct sampled colors)" }
        Write-Host ("Saved {0} ({1}x{2}, {3} sampled colors)" -f $Out, $w, $hh, $colors.Count)
    }

    if (Test-Path $errLog) {
        $errText = Get-Content $errLog -Raw
        if ($errText -and $errText.Trim()) { Write-Host "--- stderr ---`n$errText" }
    }

    $events = Join-Path $isolation.ConfigDir 'update-events.jsonl'
    if (Test-Path $events) {
        Write-Host '--- update-events.jsonl ---'
        Get-Content $events | Select-Object -Last 14
    } else {
        Write-Warning "No update-events.jsonl under $($isolation.ConfigDir): the updater never ran"
    }

    if ($Mode -eq 'install') {
        $lines = @(Get-Content $events)
        foreach ($needle in '"event":"update.check_completed"', '"event":"update.download_completed"', '"event":"update.layout_persisted"', '"event":"update.install_launched"', '"event":"update.daemon_shutdown"', '"event":"update.exiting"') {
            if (-not ($lines | Where-Object { $_ -like "*$needle*" })) { throw "install mode: telemetry is missing $needle" }
        }
        if (-not ($lines | Where-Object { $_ -like '*"event":"update.daemon_shutdown"*"outcome":"ok"*' })) {
            throw 'install mode: the daemon shutdown did not report outcome ok'
        }
        $ws = Join-Path $isolation.ConfigDir 'workspaces.json'
        $persisted = Get-Content -LiteralPath $ws -Raw | ConvertFrom-Json
        $tabCount = @($persisted.workspaces[0].tabs).Count
        if ($tabCount -lt 1) { throw "install mode: workspaces.json lost its tabs (count $tabCount)" }
        $downloaded = Join-Path $dataDir 'updates\terminal-manager-99.0.0-setup.exe'
        if (-not (Test-Path -LiteralPath $downloaded)) { throw "install mode: downloaded installer not found at $downloaded" }
        Write-Host "install mode OK: exited, telemetry chain complete, $tabCount tab(s) persisted, installer downloaded to $downloaded"
    }
} finally {
    # Cleanup must not hide the assertion that failed: restore the preference
    # before the exception surfaces at script scope.
    $savedErrorAction = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
    Remove-Item Env:TM_UPDATE_FEED_URL -ErrorAction SilentlyContinue
    Remove-Item Env:TM_UPDATE_STARTUP_DELAY_MS -ErrorAction SilentlyContinue
    Remove-Item Env:TM_UPDATE_INSTALL_SCOPE -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force -LiteralPath $feedDir -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $dataDir) { Remove-Item -Recurse -Force -LiteralPath $dataDir -ErrorAction SilentlyContinue }
    $roaming = Join-Path $env:APPDATA ("com.godly.terminal.{0}" -f $isolation.Token)
    if (Test-Path -LiteralPath $roaming) { Remove-Item -Recurse -Force -LiteralPath $roaming -ErrorAction SilentlyContinue }
    $ErrorActionPreference = $savedErrorAction
}
