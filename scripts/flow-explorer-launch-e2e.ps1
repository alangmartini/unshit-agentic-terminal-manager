#Requires -Version 5.1
<#
.SYNOPSIS
  End-to-end check of the Flow Explorer launch path under an isolated profile.

.DESCRIPTION
  1. Starts the app with TM_STARTUP_DISPATCH="flow.explain:<request>": the
     agent tab opens, the prompt file is written next to the future output
     under the profile's flows/ directory, and `flow.launch` is logged.
  2. Stands in for the agent: writes the committed fixture to the pending
     output path via <path>.tmp + rename (the contract the skill follows).
  3. Waits for the poll thread to open the pane (`flow.ready`), captures the
     window with PrintWindow(PW_RENDERFULLCONTENT) and prints the telemetry.

  No synthesized input, no focus stealing, and the installed app's daemon,
  sessions and config are never touched (throwaway TM_PROFILE). Pass
  -StripPathDir with the directory holding claude.cmd to keep a real agent
  from starting in the launched tab.
#>
[CmdletBinding()]
param(
    [string]$ExeDir = "",
    [string]$Request = "Send a Quick Prompt",
    [string]$Out = "flow-explorer-launch-e2e.png",
    [string]$StripPathDir = "",
    [string]$PrependPathDir = "",
    [int]$WaitSec = 40,
    [int]$Width = 1500,
    [int]$Height = 1500
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FlowE2eWin {
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
[FlowE2eWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\debug' }
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
$fixture = Join-Path $repoRoot 'tests\fixtures\flow-explorer\send-a-prompt.json'
$fixtureRepo = (Join-Path $repoRoot 'tests\fixtures\flow-explorer\repo').Replace('\', '/')
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $repoRoot $Out }

function Capture([IntPtr]$handle, [string]$path) {
    $rect = New-Object FlowE2eWin+RECT
    [FlowE2eWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $hh = $rect.Bottom - $rect.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [FlowE2eWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "Saved $path ($w x $hh)"
}

function Events([string]$path) {
    if (Test-Path $path) { Get-Content $path } else { @() }
}

$launched = $null
$isolation = Enter-TmIsolation -Tag 'flowe2e'
# data_dir() is not redirected by TM_CONFIG_DIR; the profile token namespaces it.
$dataDir = Join-Path $env:APPDATA ("com.godly.terminal." + $isolation.Token)
$events = Join-Path $isolation.ConfigDir 'flow-events.jsonl'
$errLog = "$Out.err.txt"
$savedPath = $env:PATH
try {
    if ($StripPathDir) {
        $env:PATH = (($env:PATH -split ';') | Where-Object { $_ -and ($_.TrimEnd('\') -ne $StripPathDir.TrimEnd('\')) }) -join ';'
    }
    if ($PrependPathDir) { $env:PATH = $PrependPathDir + ';' + $env:PATH }
    $env:TM_STARTUP_DISPATCH = "flow.explain:$Request"
    try {
        $proc = Start-Process -FilePath $exe -WorkingDirectory $repoRoot -PassThru -RedirectStandardError $errLog
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
        $env:PATH = $savedPath
    }
    $launched = $proc
    Write-Host "Launched pid=$($proc.Id) profile=$($isolation.Token)"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)); see $errLog" }
        $h = [FlowE2eWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }
    # Deterministic size, no activation (SWP_NOACTIVATE | SWP_NOZORDER).
    [FlowE2eWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 120, $Width, $Height, 0x14) | Out-Null

    # 1. the launch
    $clock.Restart()
    $launch = $null
    while ($clock.Elapsed.TotalSeconds -lt 30) {
        $launch = (Events $events) | Where-Object { $_ -match '"event":"flow.launch"' } | Select-Object -Last 1
        if ($launch) { break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $launch) { Write-Host "--- events ---"; Events $events; throw 'No flow.launch within 30s' }
    Write-Host "launch: $launch"
    $target = ($launch | ConvertFrom-Json).path
    if (-not (Test-Path $dataDir)) { Write-Warning "data dir not found at $dataDir" }
    $promptFile = [System.IO.Path]::ChangeExtension($target, $null).TrimEnd('.') + '.prompt.md'
    if (Test-Path $promptFile) {
        Write-Host "prompt file: $promptFile ($((Get-Item $promptFile).Length) bytes)"
    } else {
        Write-Warning "prompt file missing: $promptFile"
    }

    Start-Sleep -Seconds 4
    $again = [FlowE2eWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    [FlowE2eWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 120, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds 800
    Capture $handle ([System.IO.Path]::ChangeExtension($Out, $null).TrimEnd('.') + '-launched.png')

    # 2. stand in for the agent (tmp + rename, like the skill says)
    $json = Get-Content $fixture -Raw
    $json = $json -replace '"repo_root":\s*"[^"]*"', ('"repo_root": "' + $fixtureRepo + '"')
    [System.IO.File]::WriteAllText("$target.tmp", $json)
    Move-Item -LiteralPath "$target.tmp" -Destination $target -Force
    Write-Host "wrote fixture to $target"

    # 3. the poll opens the pane
    $clock.Restart()
    $ready = $null
    while ($clock.Elapsed.TotalSeconds -lt $WaitSec) {
        $ready = (Events $events) | Where-Object { $_ -match '"event":"flow\.(ready|parse_failed|timeout)"' } | Select-Object -Last 1
        if ($ready) { break }
        Start-Sleep -Milliseconds 500
    }
    Write-Host "--- flow-events.jsonl ---"
    Events $events
    if (-not $ready) { throw "No flow.ready within ${WaitSec}s" }
    if ($ready -notmatch '"event":"flow.ready"') { throw "Launch did not end in flow.ready: $ready" }
    Start-Sleep -Seconds 2
    $again = [FlowE2eWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    Capture $handle $Out
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
    if (Test-Path $dataDir) { Remove-Item -Recurse -Force -LiteralPath $dataDir }
}
