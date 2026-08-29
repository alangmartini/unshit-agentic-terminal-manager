#Requires -Version 5.1
<#
.SYNOPSIS
  Prove the window answers messages while the startup placeholder is up.

.DESCRIPTION
  A window showing a correct picture and a window that is actually running
  look identical in a screenshot. The difference is whether it answers the
  window manager, and the cheapest honest test of that is `SendMessageTimeout`
  with `SMTO_ABORTIFHUNG`: it goes through the target's message queue, so it
  only returns if the queue is being pumped.

  This is the same measurement that originally diagnosed the bug, inverted. It
  used to be run as `PrintWindow` and it blocked for 1.2 seconds; here the
  probe should return in single-digit milliseconds, repeatedly, for the whole
  interval between the window appearing and the GPU landing.

  Reports the slowest response seen. Anything above a few hundred ms means the
  loop is parking somewhere it should not.
#>
[CmdletBinding()]
param(
    [int]$Probes = 40,
    [int]$GapMs = 25,
    [string]$ExeDir
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class LiveWin {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", SetLastError=true)]
  public static extern IntPtr SendMessageTimeoutW(IntPtr hWnd, uint msg, IntPtr wParam,
      IntPtr lParam, uint flags, uint timeoutMs, out IntPtr result);
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  // WM_NULL: the window does not have to understand it, only receive it.
  public const uint WM_NULL = 0x0000;
  public const uint SMTO_ABORTIFHUNG = 0x0002;
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

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'
$workDir = Join-Path $repoRoot 'target\splash-responsive'
New-Item -ItemType Directory -Force -Path $workDir | Out-Null

$launched = $null
$isolation = Enter-TmIsolation -Tag 'responsive'
$env:RUST_LOG = 'info'

try {
    $realLayout = Join-Path $env:APPDATA 'com.godly.terminal\workspaces.json'
    if (Test-Path $realLayout) {
        Copy-Item -LiteralPath $realLayout `
            -Destination (Join-Path $isolation.ConfigDir 'workspaces.json') -Force
    }

    $errLog = Join-Path $workDir 'stderr.log'
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $appExe -PassThru -RedirectStandardError $errLog `
        -RedirectStandardOutput (Join-Path $workDir 'stdout.log')
    $launched = $proc

    $handle = [IntPtr]::Zero
    while ($clock.Elapsed.TotalMilliseconds -lt 20000) {
        if ($proc.HasExited) { throw "Process exited early (code $($proc.ExitCode))" }
        $h = [LiveWin]::LargestVisibleWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { $handle = $h; break }
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No window within 20s' }
    Write-Host ("Window appeared at {0:N0} ms" -f $clock.Elapsed.TotalMilliseconds) -ForegroundColor Cyan

    $worst = 0.0
    $hung = 0
    $samples = New-Object System.Collections.Generic.List[double]
    for ($i = 0; $i -lt $Probes; $i++) {
        $out = [IntPtr]::Zero
        $t = [System.Diagnostics.Stopwatch]::StartNew()
        $r = [LiveWin]::SendMessageTimeoutW($handle, [LiveWin]::WM_NULL, [IntPtr]::Zero,
            [IntPtr]::Zero, [LiveWin]::SMTO_ABORTIFHUNG, 1000, [ref]$out)
        $t.Stop()
        $ms = $t.Elapsed.TotalMilliseconds
        $samples.Add($ms)
        if ($ms -gt $worst) { $worst = $ms }
        if ($r -eq [IntPtr]::Zero) { $hung++ }
        $spin = [System.Diagnostics.Stopwatch]::StartNew()
        while ($spin.Elapsed.TotalMilliseconds -lt $GapMs) { }
    }
    $probedUntil = $clock.Elapsed.TotalMilliseconds

    $sorted = $samples | Sort-Object
    $median = $sorted[[int]([math]::Floor($sorted.Count / 2))]
    Write-Host ("Probed {0} times up to {1:N0} ms" -f $Probes, $probedUntil)
    Write-Host ("  median reply {0:N2} ms, worst {1:N2} ms, hung/timed out: {2}" -f $median, $worst, $hung)

    $text = Get-Content -LiteralPath $errLog -Raw -ErrorAction SilentlyContinue
    foreach ($name in @('renderer.splash_shown', 'renderer.splash_swapped')) {
        $line = ($text -split "`n" | Where-Object { $_ -match [regex]::Escape($name) } | Select-Object -First 1)
        if ($line) {
            $idx = $line.IndexOf('{"event"')
            if ($idx -ge 0) { Write-Host ("  " + $line.Substring($idx)) -ForegroundColor DarkGray }
        }
    }

    Write-Host ''
    if ($hung -eq 0 -and $worst -lt 300) {
        Write-Host 'The window answered every probe: it is running, not just showing a picture.' -ForegroundColor Green
    } else {
        Write-Host ("Not responsive: {0} probe(s) hung, worst {1:N0} ms." -f $hung, $worst) -ForegroundColor Red
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
