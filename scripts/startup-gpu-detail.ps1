#Requires -Version 5.1
<#
.SYNOPSIS
  Dump every GPU bring-up sub-phase for one launch per backend.

.DESCRIPTION
  `startup-backend-probe.ps1` compares backends at the summary level. This
  prints the full breakdown -- instance, adapter, device, surface config,
  pipeline compilation, backdrop probe -- because the summary hid where the
  time actually went: on the tested machine, device creation and shader
  compilation are two separate ~1s costs with completely different remedies.

  Also prints the selected present mode, which is the reason the D3D12
  preference exists in the first place. A backend that cannot offer Mailbox
  is not a drop-in swap however fast it starts.
#>
[CmdletBinding()]
param(
    [string[]]$Backends = @('dx12', 'vulkan'),
    [int]$Repeats = 1,
    [string]$ExeDir,
    [int]$SettleMs = 4000
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'

$isolation = Enter-TmIsolation -Tag 'gpudetail'
$env:RUST_LOG = 'info'
$env:TM_STARTUP_TRACE = '1'

$events = @(
    'renderer.gpu_init'
    'renderer.adapter_device'
    'renderer.surface_configured'
    'renderer.pipelines_built'
    'frame_pacer'
)

try {
    Copy-Item -LiteralPath (Join-Path $env:APPDATA 'com.godly.terminal\workspaces.json') `
        -Destination (Join-Path $isolation.ConfigDir 'workspaces.json') -Force -ErrorAction SilentlyContinue

    foreach ($backend in $Backends) {
        for ($r = 1; $r -le $Repeats; $r++) {
            $prev = $ErrorActionPreference
            $ErrorActionPreference = 'SilentlyContinue'
            try { & $ptydExe --shutdown --force --socket $isolation.PipePath | Out-Null } catch {}
            $ErrorActionPreference = $prev
            $global:LASTEXITCODE = 0

            $env:UNSHIT_RENDER_BACKEND = $backend
            $log = Join-Path $isolation.ConfigDir "stderr-$backend-$r.log"
            $out = Join-Path $isolation.ConfigDir "stdout-$backend-$r.log"
            $proc = Start-Process -FilePath $appExe -PassThru -RedirectStandardError $log -RedirectStandardOutput $out
            Start-Sleep -Milliseconds $SettleMs
            try { if (-not $proc.HasExited) { $proc.Kill() } } catch {}
            try { $proc.WaitForExit(5000) | Out-Null } catch {}

            Write-Host ''
            Write-Host "=== $backend (run $r) ===" -ForegroundColor Cyan
            foreach ($ev in $events) {
                $hits = Select-String -Path $log -Pattern $ev -ErrorAction SilentlyContinue
                foreach ($hit in $hits) {
                    $line = $hit.Line -replace '^.*?(\{")', '$1'
                    Write-Host "  $line"
                }
            }
            $startup = Select-String -Path $log -Pattern '"event":"app.startup"' -ErrorAction SilentlyContinue |
                Select-Object -First 1
            if ($startup) {
                $sj = ($startup.Line -replace '^.*?(\{"event")', '$1') | ConvertFrom-Json
                Write-Host ("  first_frame_total_ms={0:N0}" -f $sj.total_ms) -ForegroundColor Yellow
                foreach ($stage in $sj.stages) {
                    if ($stage.delta_ms -ge 5) {
                        Write-Host ("    {0,-24} +{1,8:N1} ms  (at {2,8:N1})" -f $stage.stage, $stage.delta_ms, $stage.at_ms)
                    }
                }
            }
            Start-Sleep -Milliseconds 200
        }
    }
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    $env:UNSHIT_RENDER_BACKEND = $null
    $env:RUST_LOG = $null
    $env:TM_STARTUP_TRACE = $null
    Exit-TmIsolation -Isolation $isolation -PtydExe $ptydExe
}
