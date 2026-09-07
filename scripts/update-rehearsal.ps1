<#
.SYNOPSIS
  Rehearses the self-update installer hand-off end to end without touching the
  installed app.

.DESCRIPTION
  The installer half of self-update (PrepareToInstall waits for the app and for
  both executables to be free, [Files] replaces them, DeinitializeSetup
  relaunches the app) cannot be tested with the real .iss on a machine that has
  the app installed: same AppId, it would replace the user's copy. This script
  derives two "TM Rehearsal" installers from packaging\terminal-manager.iss
  (separate AppId, name, output name, harmless uninstall taskkill) with the
  current release binaries, one labelled 0.4.0-as-built and one 99.0.0, then:

    1. installs the first into %LOCALAPPDATA%\tm-rehearsal (silent, per-user);
    2. launches it under a throwaway profile (TM_PROFILE) against a file://
       feed whose asset is the 99.0.0 installer and dispatches update.install;
    3. asserts the app exits by itself, the installer log shows the parent
       wait, both executables free, "Installation process succeeded" and the
       relaunch, a new app process appears from the install dir, and the
       rehearsal's uninstall key reports DisplayVersion 99.0.0;
    4. stops the relaunched app and the isolated daemon, uninstalls, and
       removes the generated .iss files, installers, feed and profile dirs.

  Run after any change to src/updater, src/pty.rs shutdown, or the .iss [Code]
  section. Needs Inno Setup 6 and fresh target\release binaries
  (cargo build --release -p terminal-manager --bin terminal-manager
   -p unshit-ptyd --bin unshit-ptyd).

.PARAMETER ReleaseDir
  Directory holding terminal-manager.exe and unshit-ptyd.exe. Default target\release.
.PARAMETER Iscc
  Path to ISCC.exe. Default: the per-user Inno Setup 6 install.
.PARAMETER KeepArtifacts
  Leave the generated .iss files, installers and logs in place for inspection.
#>
[CmdletBinding()]
param(
    [string]$ReleaseDir = "",
    [string]$Iscc = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    [switch]$KeepArtifacts
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot 'lib\tm-isolation.ps1')

if (-not $ReleaseDir) { $ReleaseDir = Join-Path $repoRoot 'target\release' }
$ReleaseDir = (Resolve-Path $ReleaseDir).Path
foreach ($exe in 'terminal-manager.exe', 'unshit-ptyd.exe') {
    if (-not (Test-Path -LiteralPath (Join-Path $ReleaseDir $exe))) { throw "$exe missing under $ReleaseDir; build release first" }
}
if (-not (Test-Path -LiteralPath $Iscc)) { throw "ISCC.exe not found at $Iscc" }

$rehearsalGuid = '{7E1C2D3B-5A5A-4B4B-9C9C-00000000BEEF}'
$installDir = Join-Path $env:LOCALAPPDATA 'tm-rehearsal'
$regKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${rehearsalGuid}_is1"
$realRegKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{B3E1B6B2-7C44-4E2E-9C1A-0A1D2E3F4A5B}_is1'
$realVersionBefore = (Get-ItemProperty $realRegKey -ErrorAction SilentlyContinue).DisplayVersion
$work = Join-Path $env:TEMP ("tm-update-rehearsal-{0}" -f $PID)
New-Item -ItemType Directory -Force -Path $work | Out-Null
$issBase = Join-Path $repoRoot 'packaging\_rehearsal-base.iss'
$issTarget = Join-Path $repoRoot 'packaging\_rehearsal-target.iss'
$isolation = $null
$relaunched = $null
$feedDir = $null

function Replace-Required([string]$text, [string]$pattern, [string]$replacement, [string]$what) {
    if ($text -notmatch $pattern) { throw "packaging\terminal-manager.iss no longer contains $what; update scripts\update-rehearsal.ps1" }
    return [regex]::Replace($text, $pattern, $replacement)
}

try {
    # --- 1. derive the two rehearsal installers ---------------------------------
    $iss = [System.IO.File]::ReadAllText((Join-Path $repoRoot 'packaging\terminal-manager.iss'))
    $iss = Replace-Required $iss ([regex]::Escape('{{B3E1B6B2-7C44-4E2E-9C1A-0A1D2E3F4A5B}')) "{{$($rehearsalGuid.Trim('{','}'))}" 'the AppId'
    $iss = Replace-Required $iss ([regex]::Escape('#define MyAppName "Terminal Manager"')) '#define MyAppName "TM Rehearsal"' 'MyAppName'
    $iss = Replace-Required $iss ([regex]::Escape('#define MyStartupValueName "Unshit Terminal Manager"')) '#define MyStartupValueName "TM Rehearsal"' 'MyStartupValueName'
    $iss = Replace-Required $iss ([regex]::Escape('OutputBaseFilename=terminal-manager-{#MyAppVersion}-setup')) 'OutputBaseFilename=tm-rehearsal-{#MyAppVersion}-setup' 'OutputBaseFilename'
    # The real uninstaller kills unshit-ptyd.exe; the rehearsal must not touch the user's daemon.
    $iss = Replace-Required $iss ([regex]::Escape('/F /IM {#MyDaemonExeName}')) '/F /IM tm-rehearsal-none.exe' 'the [UninstallRun] taskkill'
    $iss = Replace-Required $iss '#define ReleaseDir "[^"]+"' ('#define ReleaseDir "' + $ReleaseDir.Replace('\', '\\') + '"') 'ReleaseDir'
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($issBase, $iss, $utf8)
    $issTargetText = Replace-Required $iss '#define MyAppVersion "[^"]+"' '#define MyAppVersion "99.0.0"' 'MyAppVersion'
    [System.IO.File]::WriteAllText($issTarget, $issTargetText, $utf8)
    $baseVersion = [regex]::Match($iss, '#define MyAppVersion "([^"]+)"').Groups[1].Value

    foreach ($script in $issBase, $issTarget) {
        & $Iscc /Qp "/O$work" $script | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE) on $script" }
    }
    $baseSetup = Join-Path $work "tm-rehearsal-$baseVersion-setup.exe"
    $targetSetup = Join-Path $work 'tm-rehearsal-99.0.0-setup.exe'
    foreach ($f in $baseSetup, $targetSetup) { if (-not (Test-Path -LiteralPath $f)) { throw "installer not produced: $f" } }
    Write-Host "compiled $baseSetup and $targetSetup"

    # --- 2. base install ----------------------------------------------------------
    $baseLog = Join-Path $work 'base-install.log'
    # Start-Process does not quote elements with spaces; quote the path values.
    $p = Start-Process -FilePath $baseSetup -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$installDir`"", "/LOG=`"$baseLog`"") -PassThru -Wait
    if ($p.ExitCode -ne 0) { throw "base install failed with exit $($p.ExitCode); see $baseLog" }
    $reg = Get-ItemProperty $regKey
    Write-Host "installed TM Rehearsal $($reg.DisplayVersion) at $($reg.InstallLocation)"

    # --- 3. throwaway profile, fake feed, update.install --------------------------
    $isolation = Enter-TmIsolation -Tag 'rehearsal'
    $feedDir = Join-Path $work 'feed'
    New-Item -ItemType Directory -Force -Path $feedDir | Out-Null
    $asset = Join-Path $feedDir 'terminal-manager-99.0.0-setup.exe'
    Copy-Item -LiteralPath $targetSetup -Destination $asset -Force
    $digest = (Get-FileHash -Algorithm SHA256 -LiteralPath $asset).Hash.ToLower()
    $feed = @{
        tag_name = 'v99.0.0'; name = '99.0.0 (rehearsal)'; html_url = 'https://example.invalid/release'
        draft = $false; prerelease = $false
        assets = @(@{ name = 'terminal-manager-99.0.0-setup.exe'; browser_download_url = ('file:///' + ($asset -replace '\\', '/')); size = (Get-Item -LiteralPath $asset).Length; digest = "sha256:$digest" })
    }
    $feedPath = Join-Path $feedDir 'latest.json'
    [System.IO.File]::WriteAllText($feedPath, ($feed | ConvertTo-Json -Depth 4), $utf8)
    $env:TM_UPDATE_FEED_URL = 'file:///' + ($feedPath -replace '\\', '/')
    $env:TM_UPDATE_STARTUP_DELAY_MS = '600000'
    $env:TM_UPDATE_INSTALL_SCOPE = 'user'   # the rehearsal AppId is not the one install.rs looks up
    $env:TM_STARTUP_DISPATCH = 'settings.section:updates;update.install'
    $errLog = Join-Path $work 'app.err.txt'
    try {
        $app = Start-Process -FilePath (Join-Path $installDir 'terminal-manager.exe') -WorkingDirectory $installDir -PassThru -RedirectStandardError $errLog
        $null = $app.Handle
    } finally {
        Remove-Item Env:TM_STARTUP_DISPATCH -ErrorAction SilentlyContinue
    }
    Write-Host "launched pid=$($app.Id) with update.install"
    if (-not $app.WaitForExit(90000)) { throw 'the app did not exit within 90 s after update.install' }
    Write-Host "app exited on its own (code $($app.ExitCode))"

    # --- 4. the installer must relaunch the app from the install dir ---------------
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 120) {
        $relaunched = Get-Process -Name terminal-manager -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and $_.Path.StartsWith($installDir, [System.StringComparison]::OrdinalIgnoreCase) } |
            Select-Object -First 1
        if ($relaunched) { break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $relaunched) { throw 'no relaunched app from the install dir within 120 s' }
    Write-Host "relaunched pid=$($relaunched.Id) after $([int]$sw.Elapsed.TotalSeconds)s"
    Start-Sleep -Seconds 5   # let it initialise and write update.init

    # --- 5. evidence ---------------------------------------------------------------
    $after = (Get-ItemProperty $regKey).DisplayVersion
    if ($after -ne '99.0.0') { throw "DisplayVersion is $after, expected 99.0.0" }
    $updatesDir = Join-Path $env:LOCALAPPDATA ("com.godly.terminal.{0}\updates" -f $isolation.Token)
    $installerLog = Get-ChildItem -LiteralPath $updatesDir -Filter '*.log' | Select-Object -First 1
    if (-not $installerLog) { throw "no installer log under $updatesDir" }
    $logText = Get-Content -LiteralPath $installerLog.FullName -Raw
    foreach ($needle in 'Self-update: waiting for parent pid', 'unshit-ptyd.exe free after', 'terminal-manager.exe free after', 'Installation process succeeded.', 'Self-update: relaunching') {
        if ($logText -notlike "*$needle*") { throw "installer log lacks '$needle'; see $($installerLog.FullName)" }
    }
    Write-Host '--- installer log (Self-update lines) ---'
    Get-Content -LiteralPath $installerLog.FullName | Select-String -Pattern 'Self-update|Installation process succeeded' | ForEach-Object { $_.Line }
    $events = @(Get-Content -LiteralPath (Join-Path $isolation.ConfigDir 'update-events.jsonl'))
    foreach ($needle in '"event":"update.download_completed"', '"event":"update.install_launched"', '"event":"update.daemon_shutdown"', '"event":"update.exiting"') {
        if (-not ($events | Where-Object { $_ -like "*$needle*" })) { throw "update-events.jsonl lacks $needle" }
    }
    $inits = @($events | Where-Object { $_ -like '*"event":"update.init"*' })
    if ($inits.Count -lt 2) { throw "expected update.init from both the old and the relaunched app, got $($inits.Count)" }
    if (Test-Path -LiteralPath $errLog) {
        $panics = Get-Content -LiteralPath $errLog | Where-Object { $_ -match 'panicked|ERROR' }
        if ($panics) { throw "app stderr has errors: $($panics -join '; ')" }
    }
    Write-Host "rehearsal OK: $baseVersion -> 99.0.0 installed in place, app relaunched (pid $($relaunched.Id)), $($events.Count) telemetry lines"
} finally {
    $ErrorActionPreference = 'Continue'
    if ($relaunched) { try { Stop-Process -Id $relaunched.Id -Force } catch {} }
    Get-Process -Name terminal-manager -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($installDir, [System.StringComparison]::OrdinalIgnoreCase) } |
        ForEach-Object { try { Stop-Process -Id $_.Id -Force } catch {} }
    if ($isolation) {
        Start-Sleep -Seconds 1
        Exit-TmIsolation -Isolation $isolation -PtydExe (Join-Path $installDir 'unshit-ptyd.exe')
    }
    $unins = Join-Path $installDir 'unins000.exe'
    if (Test-Path -LiteralPath $unins) {
        $u = Start-Process -FilePath $unins -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -PassThru -Wait
        Start-Sleep -Seconds 2
        Write-Host "uninstalled (exit $($u.ExitCode)); install dir left: $(Test-Path -LiteralPath $installDir); key left: $(Test-Path $regKey)"
    }
    if ($isolation) {
        foreach ($root in $env:LOCALAPPDATA, $env:APPDATA) {
            $d = Join-Path $root ("com.godly.terminal.{0}" -f $isolation.Token)
            if (Test-Path -LiteralPath $d) { Remove-Item -Recurse -Force -LiteralPath $d -ErrorAction SilentlyContinue }
        }
    }
    if (-not $KeepArtifacts) {
        Remove-Item -Force -LiteralPath $issBase, $issTarget -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force -LiteralPath $work -ErrorAction SilentlyContinue
    } else {
        Write-Host "artifacts kept under $work (and packaging\_rehearsal-*.iss)"
    }
    Remove-Item Env:TM_UPDATE_FEED_URL, Env:TM_UPDATE_STARTUP_DELAY_MS, Env:TM_UPDATE_INSTALL_SCOPE -ErrorAction SilentlyContinue
    $realVersionAfter = (Get-ItemProperty $realRegKey -ErrorAction SilentlyContinue).DisplayVersion
    if ($realVersionAfter -ne $realVersionBefore) { Write-Error "the real install's DisplayVersion changed ($realVersionBefore -> $realVersionAfter)" }
}
