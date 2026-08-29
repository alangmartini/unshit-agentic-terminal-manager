#Requires -Version 5.1
<#
.SYNOPSIS
  Compare cold-start GPU bring-up cost across renderer backends.

.DESCRIPTION
  Adapter and device creation is the largest single block of cold-start
  latency, and how much it costs is a property of the *driver*, not of our
  code: the same machine can differ several-fold between D3D12 and Vulkan.
  This probe launches the app once per backend in an isolated instance
  profile and prints the `renderer.gpu_init` breakdown for each, so the
  default backend choice can be made from measurements on the target
  machine rather than from assumption.

  Each launch is a true cold start (daemon shut down first) and runs in a
  throwaway profile, so it can never touch the installed app's sessions.
#>
[CmdletBinding()]
param(
    [string[]]$Backends = @('dx12', 'vulkan', 'gl'),
    [int]$Repeats = 2,
    [string]$ExeDir
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
$appExe = Join-Path $ExeDir 'terminal-manager.exe'
$ptydExe = Join-Path $ExeDir 'unshit-ptyd.exe'

$isolation = Enter-TmIsolation -Tag 'gpuprobe'
$env:RUST_LOG = 'info'
$env:TM_STARTUP_TRACE = '1'

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
            Start-Sleep -Seconds 5
            try { if (-not $proc.HasExited) { $proc.Kill() } } catch {}
            try { $proc.WaitForExit(5000) | Out-Null } catch {}

            $gpu = Select-String -Path $log -Pattern 'renderer.gpu_init' -ErrorAction SilentlyContinue |
                Select-Object -First 1
            $startup = Select-String -Path $log -Pattern '"event":"app.startup"' -ErrorAction SilentlyContinue |
                Select-Object -First 1

            if ($gpu) {
                $json = ($gpu.Line -replace '^.*?(\{"event")', '$1') | ConvertFrom-Json
                $firstFrame = 'n/a'
                if ($startup) {
                    $sj = ($startup.Line -replace '^.*?(\{"event")', '$1') | ConvertFrom-Json
                    $firstFrame = '{0:N0}' -f $sj.total_ms
                }
                Write-Host ("{0,-7} run {1}: backend={2,-6} instance={3,7:N1}ms adapter+device={4,7:N1}ms gpu_total={5,7:N1}ms first_frame={6}ms" -f `
                        $backend, $r, $json.backend, $json.instance_ms, $json.adapter_device_ms, $json.total_ms, $firstFrame)
            } else {
                Write-Host ("{0,-7} run {1}: no gpu_init line (backend unavailable?)" -f $backend, $r)
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
