#Requires -Version 5.1
<#
.SYNOPSIS
  Capture the sidebar and tab strip with guest titles that carry status glyphs.

.DESCRIPTION
  Verification shot for symbol rendering in UI labels. Claude Code reports
  window titles such as "✳ Workspace" (idle) and "◐ Create PR" (busy); the
  UI font lacks those symbols, and before the symbol fallback in
  `unshit_core::text_fallback` the ✳ resolved to Segoe UI Emoji's color
  glyph and rendered as a solid box. The script opens two tabs whose guest
  PowerShell reports those titles (ConPTY forwards SetConsoleTitle as OSC 0),
  so the sidebar rows and tab labels show the glyphs, then captures the
  window with PrintWindow(PW_RENDERFULLCONTENT).

  Drives the app through TM_STARTUP_DISPATCH (no synthesized input, no focus
  stealing) under a throwaway TM_PROFILE so the installed app's daemon,
  sessions and config are never touched. Afterwards it prints the
  `renderer.symbol_fallback` lines from the isolated profile's
  renderer-events.jsonl, which is the "did the fallback fire" check.

.EXAMPLE
  pwsh scripts/title-glyph-shot.ps1
  pwsh scripts/title-glyph-shot.ps1 -ExeDir C:\path\to\target\debug
#>
[CmdletBinding()]
param(
    [string]$Out = "title-glyph-shot.png",
    # Guest titles for the two tabs. Defaults mimic Claude Code's idle and
    # busy titles: an emoji-capable symbol and a plain geometric one.
    [string[]]$Titles = @("$([char]0x2733) Workspace", "$([char]0x25D0) Create PR"),
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
public class TitleGlyphShotWin {
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
[TitleGlyphShotWin]::SetProcessDPIAware() | Out-Null

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

# Each tab's shell becomes a PowerShell that only sets its window title and
# waits. The guest script goes through -File: ptyd appends
# `-NoExit -Command "Set-Location ..."` to every PowerShell spawn, and after
# -File those land in $args instead of clashing with -Command. The title
# lives in the file so it never meets the dispatch separator (`;`).
$guestScripts = @()
$dispatchParts = @()
$i = 0
foreach ($title in $Titles) {
    $safeTitle = $title.Replace("'", "''")
    $guestScript = Join-Path $env:TEMP ("tm-title-glyph-shot-{0}-{1}.ps1" -f $PID, $i)
@"
`$t = '$safeTitle'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
`$Host.UI.RawUI.WindowTitle = `$t
[Console]::Out.Write([string][char]27 + ']0;' + `$t + [string][char]7)
[Console]::Out.Flush()
Write-Host 'title set'
while (`$true) { Start-Sleep -Seconds 5 }
"@ | Set-Content -LiteralPath $guestScript -Encoding UTF8
    $guestScripts += $guestScript
    $spec = @{ program = 'powershell.exe'; args = @('-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $guestScript) } | ConvertTo-Json -Compress
    $dispatchParts += "shell.set_workspace:0:$spec"
    $dispatchParts += "tab.new"
    $i++
}
$dispatch = $dispatchParts -join ';'

$launched = $null
$isolation = Enter-TmIsolation -Tag 'titleglyph'
$errLog = "$Out.err.txt"
$env:TM_STARTUP_DISPATCH = $dispatch
try {
    try {
        $proc = Start-Process -FilePath $exe -WorkingDirectory $repoRoot -PassThru -RedirectStandardError $errLog
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
    }
    $launched = $proc
    Write-Host "Launched pid=$($proc.Id) titles=$($Titles -join ' | ')"

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode)); see $errLog" }
        $h = [TitleGlyphShotWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }

    # Deterministic size, no activation (SWP_NOACTIVATE = 0x10, SWP_NOZORDER = 0x4).
    [TitleGlyphShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds $SettleMs

    # The first handle can be the splash window, which is destroyed once the
    # GPU surface is up; resolve again after settling and reposition.
    $again = [TitleGlyphShotWin]::LargestVisibleWindow([uint32]$proc.Id)
    if ($again -ne [IntPtr]::Zero) { $handle = $again }
    [TitleGlyphShotWin]::SetWindowPos($handle, [IntPtr]::Zero, 40, 320, $Width, $Height, 0x14) | Out-Null
    Start-Sleep -Milliseconds 1200

    $rect = New-Object TitleGlyphShotWin+RECT
    [TitleGlyphShotWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $hh = $rect.Bottom - $rect.Top
    if ($w -lt 200 -or $hh -lt 200) { throw "Window rect too small ($w x $hh); handle is not the main window" }

    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [TitleGlyphShotWin]::PrintWindow($handle, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host ("Saved {0} ({1}x{2})" -f $Out, $w, $hh)

    if (Test-Path $errLog) {
        $errText = Get-Content $errLog -Raw
        if ($errText -and $errText.Trim()) { Write-Host "--- stderr ---`n$errText" }
    }
    # The isolated config dir goes away on exit; surface the telemetry first
    # so the capture doubles as the "did the fallback fire" check.
    $renderer = Join-Path $isolation.ConfigDir 'renderer-events.jsonl'
    if (Test-Path $renderer) {
        Write-Host '--- renderer-events.jsonl (symbol_fallback) ---'
        $lines = Get-Content $renderer | Select-String 'symbol_fallback'
        if ($lines) { $lines | Select-Object -Last 5 } else { Write-Warning 'no renderer.symbol_fallback event landed' }
    } else {
        Write-Warning "No renderer-events.jsonl under $($isolation.ConfigDir)"
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    if ($launched -and -not $launched.HasExited) {
        try { $launched.Kill() } catch {}
        try { $launched.WaitForExit(5000) | Out-Null } catch {}
    }
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
    foreach ($gs in $guestScripts) { Remove-Item -LiteralPath $gs -Force -ErrorAction SilentlyContinue }
}
