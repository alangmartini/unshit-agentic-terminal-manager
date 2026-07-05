#Requires -Version 5.1
<#
  Instance isolation for scripts that launch terminal-manager.exe.

  Dot-source this file, call Enter-TmIsolation before Start-Process and
  Exit-TmIsolation when done. The launched app then runs in a throwaway
  instance profile: its own daemon pipe, its own notify pipe, and a temp
  config dir — it can never attach to the installed app's (or a dev
  instance's) sessions or overwrite their workspaces.json.

  If a script dies before Exit-TmIsolation, the leftover daemon sits on
  an ephemeral pipe nobody reconnects to; `scripts\kill-all.ps1` (repo
  scoped by default) sweeps such strays.
#>

function Enter-TmIsolation {
    param(
        # Short prefix for the profile name, e.g. "shot" or "bench".
        [string]$Tag = "shot"
    )

    $token = "{0}{1}x{2}" -f $Tag, $PID, (Get-Random -Maximum 99999)
    $configDir = Join-Path $env:TEMP (Join-Path "tm-isolated" $token)
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    # Set the pipe explicitly (instead of relying on the TM_PROFILE
    # derivation) so this script knows the exact path for --shutdown.
    $pipe = "\\.\pipe\unshit-ptyd-$token"

    $env:TM_PROFILE = $token
    $env:TM_CONFIG_DIR = $configDir
    $env:TM_PTYD_SOCKET = $pipe

    [pscustomobject]@{
        Token     = $token
        ConfigDir = $configDir
        PipePath  = $pipe
    }
}

function Exit-TmIsolation {
    param(
        [Parameter(Mandatory = $true)]$Isolation,
        # Path to unshit-ptyd.exe (sibling of the launched app exe) used
        # to shut down the session daemon this run spawned.
        [string]$PtydExe
    )

    if ($PtydExe -and (Test-Path -LiteralPath $PtydExe)) {
        try {
            # --force: the daemon refuses a plain shutdown while sessions
            # are alive, and shot scripts stop the UI without closing them.
            & $PtydExe --shutdown --force --socket $Isolation.PipePath 2>$null | Out-Null
        } catch {
            Write-Warning "failed to shut down isolated daemon on $($Isolation.PipePath): $($_.Exception.Message)"
        }
    }
    try {
        Remove-Item -Recurse -Force -LiteralPath $Isolation.ConfigDir -ErrorAction Stop
    } catch {}

    $env:TM_PROFILE = $null
    $env:TM_CONFIG_DIR = $null
    $env:TM_PTYD_SOCKET = $null
}
