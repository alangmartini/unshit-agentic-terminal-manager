#Requires -Version 5.1
<#
.SYNOPSIS
  Drive the agents subtab end to end and capture the sidebar.

.DESCRIPTION
  Verification shot for the sidebar "agents" subtab (issue #186). Without
  any agent CLI installed and without provider hooks, the app must still
  file a pane under `agents` purely from the guest window title: the script
  sets the workspace shell to a PowerShell running a small -File script
  that reports a Claude Code style title (ConPTY forwards SetConsoleTitle
  as OSC 0), opens a new tab with it, then opens the agents-subtab context
  menu so "New agent" and "Kill all agents" are visible in the capture.
  Pass -NoMenu for the bare tree and -ExtraDispatch to append commands
  (e.g. 'agent.new:bogus' to land agent.launch_failed).

  Drives the app through TM_STARTUP_DISPATCH (no synthesized input, no focus
  stealing) and captures with PrintWindow(PW_RENDERFULLCONTENT). Runs under a
  throwaway TM_PROFILE so the installed app's daemon, sessions and config are
  never touched. Afterwards it prints the `agent.*` lines from the isolated
  profile's agent-events.jsonl, which is the "did telemetry land" check.

.EXAMPLE
  pwsh scripts/agents-tab-shot.ps1
  pwsh scripts/agents-tab-shot.ps1 -Title 'Codex' -NoMenu
#>
[CmdletBinding()]
param(
    [string]$Out = "agents-tab-shot.png",
    # Guest title the spawned PowerShell reports. Default mimics Claude Code's
    # idle title; pass 'Codex' or 'gemini' to exercise the needle path.
    [string]$Title = "$([char]0x2733) Claude Code",
    # Skip the context menu so the capture shows only the split sidebar.
    [switch]$NoMenu,
    # Which subtab menu to open: agents or terminals.
    [string]$MenuKind = "agents",
    # Extra `;`-separated dispatch commands appended after the tab is open,
    # e.g. 'agent.new:bogus' to land agent.launch_failed plus its toast.
    [string]$ExtraDispatch = "",
    [double]$AnchorX = 60,
    [double]$AnchorY = 150,
    [string]$ExeDir = "",
    [int]$SettleMs = 9000,
    [int]$Width = 900,
    [int]$Height = 560
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class AgentsShotWin {
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
[AgentsShotWin]::SetProcessDPIAware() | Out-Null

if (-not $ExeDir) {
    # Worktrees usually build into the main checkout's target dir via
    # CARGO_TARGET_DIR; honour it before falling back to ./target.
    if ($env:CARGO_TARGET_DIR) { $ExeDir = Join-Path $env:CARGO_TARGET_DIR 'debug' }
    else { $ExeDir = Join-Path $repoRoot 'target\debug' }
}
$exe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
if (-not (Test-Path $exe)) { throw "Missing exe: $exe (run cargo build first)" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $repoRoot $Out }

# Workspace 0's shell becomes a PowerShell that only sets its window title
# and waits. `shell.set_workspace` takes a JSON ShellSpec. The guest script
# goes through -File: ptyd appends `-NoExit -Command "Set-Location ..."` to
# every PowerShell spawn, and after -File those land in $args instead of
# clashing with -Command/-EncodedCommand (which made PowerShell bail out
# before the title was ever set). Writing the title into a file also keeps
# it clear of the dispatch separator (`;`) and the JSON quoting.
$safeTitle = $Title.Replace("'", "''")
$guestScript = Join-Path $env:TEMP ("tm-agents-shot-{0}.ps1" -f $PID)
@"
`$t = '$safeTitle'
# UTF-8 on the way out so the status glyph survives (the OEM code page would
# turn it into '?', which still classifies but through the needle path).
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
# Set the console title (ConPTY re-emits it as OSC 0) and write the OSC 0
# sequence directly, then print a line so the frame flushes.
`$Host.UI.RawUI.WindowTitle = `$t
[Console]::Out.Write([string][char]27 + ']0;' + `$t + [string][char]7)
[Console]::Out.Flush()
Write-Host 'agent ready'
while (`$true) { Start-Sleep -Seconds 5 }
"@ | Set-Content -LiteralPath $guestScript -Encoding UTF8
$spec = @{ program = 'powershell.exe'; args = @('-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $guestScript) } | ConvertTo-Json -Compress
$dispatch = "shell.set_workspace:0:$spec;tab.new"
if ($ExtraDispatch) { $dispatch += ";$ExtraDispatch" }
if (-not $NoMenu) {
    $dispatch += ";ctx_menu.open_subtab:0:${MenuKind}:${AnchorX}:${AnchorY}"
}

$launched = $null
$isolation = Enter-TmIsolation -Tag 'agentsshot'
$errLog = "$Out.err.txt"
$env:TM_STARTUP_DISPATCH = $dispatch
try {
    try {
        $proc = Start-Process -FilePath $exe -WorkingDirectory $repoRoot -PassThru -RedirectStandardError $errLog
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
    }
    $launched = $proc
    Write-Host "Launched pid=$($proc.Id) title=$Title menu=$(-not $NoMenu)"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)); see $errLog" }
        $h = [AgentsShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }

    # Deterministic size, no activation (SWP_NOACTIVATE = 0x10, SWP_NOZORDER = 0x4).
    [AgentsShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds $SettleMs

    # The first handle can be the splash window, which is destroyed once the
    # GPU surface is up; resolve again after settling and reposition.
    $again = [AgentsShotWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    [AgentsShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds 1200

    $rect = New-Object AgentsShotWin+RECT
    [AgentsShotWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $hh = $rect.Bottom - $rect.Top
    if ($w -lt 200 -or $hh -lt 200) { throw "Window rect too small ($w x $hh); handle is not the main window" }

    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [AgentsShotWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host ("Saved {0} ({1}x{2})" -f $Out, $w, $hh)

    if (Test-Path $errLog) {
        $errText = Get-Content $errLog -Raw
        if ($errText -and $errText.Trim()) { Write-Host "--- stderr ---`n$errText" }
    }
    # The isolated config dir goes away on exit; surface the classification
    # events first so the capture doubles as the "did telemetry land" check.
    $events = Join-Path $isolation.ConfigDir 'agent-events.jsonl'
    if (Test-Path $events) {
        Write-Host '--- agent-events.jsonl ---'
        Get-Content $events | Select-Object -Last 8
    } else {
        Write-Warning "No agent-events.jsonl under $($isolation.ConfigDir): the pane was never classified"
    }
    $renderer = Join-Path $isolation.ConfigDir 'renderer-events.jsonl'
    if (Test-Path $renderer) {
        Write-Host '--- renderer-events.jsonl (ctx menu) ---'
        Get-Content $renderer | Select-String 'ctx_menu' | Select-Object -Last 3
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
    Remove-Item -LiteralPath $guestScript -Force -ErrorAction SilentlyContinue
}
