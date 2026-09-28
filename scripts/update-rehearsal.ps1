<#
.SYNOPSIS
  Rehearses the self-update installer hand-off end to end without touching the
  installed app.

.DESCRIPTION
  Builds two separately registered rehearsal installers from the current binaries.
  Starts an isolated daemon and a continuously running counter command, performs
  the UI self-update, then verifies the same daemon PID and shell PID survive and
  the counter continues. No installed user sessions are touched.

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

$rehearsalGuid = '{' + [guid]::NewGuid().ToString() + '}'
$installDir = Join-Path $env:LOCALAPPDATA ('tm-rehearsal-' + $PID)
$regKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${rehearsalGuid}_is1"
$realRegKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{B3E1B6B2-7C44-4E2E-9C1A-0A1D2E3F4A5B}_is1'
$realVersionBefore = (Get-ItemProperty $realRegKey -ErrorAction SilentlyContinue).DisplayVersion
$work = Join-Path $env:TEMP ("tm-update-rehearsal-{0}" -f $PID)
if (Test-Path -LiteralPath $installDir) { throw "Rehearsal destination exists: $installDir" }
New-Item -ItemType Directory -Force -Path $work | Out-Null
$issBase = Join-Path $repoRoot 'packaging\_rehearsal-base.iss'
$issTarget = Join-Path $repoRoot 'packaging\_rehearsal-target.iss'
$isolation = $null
$relaunched = $null
$feedDir = $null
$daemon = $null

function Replace-Required([string]$text, [string]$pattern, [string]$replacement, [string]$what) {
    if ($text -notmatch $pattern) { throw "packaging\terminal-manager.iss no longer contains $what; update scripts\update-rehearsal.ps1" }
    return [regex]::Replace($text, $pattern, $replacement)
}

function Assert-ChildPath([string]$path, [string]$root) {
    $resolved = [IO.Path]::GetFullPath($path)
    $prefix = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing cleanup outside $root : $resolved"
    }
}

function Read-Counter {
    for ($i = 0; $i -lt 20; $i++) {
        $value = 0
        try {
            if ([int]::TryParse([IO.File]::ReadAllText($counterPath), [ref]$value) -and $value -gt 0) { return $value }
        } catch {}
        Start-Sleep -Milliseconds 20
    }
    throw 'Counter file did not contain a positive number'
}

function Invoke-DaemonRequest([hashtable]$request) {
    $pipeName = $isolation.PipePath.Substring('\\.\pipe\'.Length)
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', $pipeName, [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::Asynchronous)
    try {
        $pipe.Connect(5000)
        $request.id = 1
        $json = [Text.Encoding]::UTF8.GetBytes(($request | ConvertTo-Json -Compress -Depth 8))
        $frame = [BitConverter]::GetBytes([uint32]($json.Length + 1)) + [byte]0 + $json
        $pipe.Write($frame, 0, $frame.Length)
        while ($true) {
            $prefix = Read-PipeBytes $pipe 4
            $length = [BitConverter]::ToUInt32($prefix, 0)
            if ($length -lt 1 -or $length -gt 1048576) { throw 'Invalid daemon frame' }
            $body = Read-PipeBytes $pipe $length
            if ($body[0] -ne 0) { continue }
            $response = [Text.Encoding]::UTF8.GetString($body, 1, $body.Length - 1) | ConvertFrom-Json
            if ($response.id -eq 1) {
                if ($response.kind -eq 'error') { throw $response.message }
                return $response
            }
        }
    } finally { $pipe.Dispose() }
}

function Read-PipeBytes($pipe, [int]$count) {
    $buffer = New-Object byte[] $count
    $offset = 0
    while ($offset -lt $count) {
        $read = $pipe.ReadAsync($buffer, $offset, $count - $offset)
        if (-not $read.Wait(5000)) { throw 'Daemon response timed out' }
        if ($read.Result -eq 0) { throw 'Daemon disconnected' }
        $offset += $read.Result
    }
    return ,$buffer
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
    # A deliberately older directory lets the real UI exercise deferred
    # activation after the continuity assertions, using the same built daemon.
    $iss = $iss.Replace('[Files]', ('[Files]' + [Environment]::NewLine + 'Source: "{#ReleaseDir}\{#MyDaemonExeName}"; DestDir: "{app}\daemons\0.0.0"; Flags: onlyifdoesntexist'))
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
    $p = Start-Process -FilePath $baseSetup -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$installDir`"", "/LOG=`"$baseLog`"") -WindowStyle Hidden -PassThru -Wait
    if ($p.ExitCode -ne 0) { throw "base install failed with exit $($p.ExitCode); see $baseLog" }
    $reg = Get-ItemProperty $regKey
    Write-Host "installed TM Rehearsal $($reg.DisplayVersion) at $($reg.InstallLocation)"

    # --- 3. throwaway profile, fake feed, update.install --------------------------
    $isolation = Enter-TmIsolation -Tag 'rehearsal'
    $daemonExe = Join-Path $installDir 'daemons\0.0.0\unshit-ptyd.exe'
    $daemon = Start-Process -FilePath $daemonExe -ArgumentList @('--socket', $isolation.PipePath) -WindowStyle Hidden -PassThru
    $null = $daemon.Handle
    $counterPath = Join-Path $work 'counter.txt'
    $counterScript = Join-Path $work 'counter.ps1'
    $escapedCounter = $counterPath.Replace("'", "''")
    $counterSource = '$n=0; while ($true) { $n++; [IO.File]::WriteAllText(''{0}'', [string]$n); Start-Sleep -Milliseconds 100 }'
    $counterSource = $counterSource.Replace('{0}', $escapedCounter)
    [IO.File]::WriteAllText($counterScript, $counterSource, $utf8)
    $session = Invoke-DaemonRequest @{ kind='spawn_session'; cols=80; rows=24; shell='powershell.exe'; shell_args=@('-NoProfile', '-File', $counterScript); workspace_id=60000; pane_id=60000 }
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not (Test-Path -LiteralPath $counterPath)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Counter command did not start' }
        Start-Sleep -Milliseconds 100
    }
    $before = Invoke-DaemonRequest @{ kind='list_sessions' }
    $beforeSession = $before.sessions | Where-Object { $_.id -eq $session.session_id }
    if (-not $beforeSession.alive) { throw 'Counter session is not alive' }
    $counterBefore = Read-Counter
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
        $app = Start-Process -FilePath (Join-Path $installDir 'terminal-manager.exe') -WorkingDirectory $installDir -WindowStyle Hidden -PassThru -RedirectStandardError $errLog
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
    foreach ($needle in 'Self-update: waiting for parent pid', 'daemon compatibility check passed; preserving sessions', 'terminal-manager.exe free after', 'Installation process succeeded.', 'Self-update: relaunching') {
        if ($logText -notlike "*$needle*") { throw "installer log lacks '$needle'; see $($installerLog.FullName)" }
    }
    $afterSessions = Invoke-DaemonRequest @{ kind='list_sessions' }
    $afterSession = $afterSessions.sessions | Where-Object { $_.id -eq $session.session_id }
    if ($daemon.HasExited -or $afterSessions.daemon_pid -ne $before.daemon_pid) { throw 'Daemon was replaced during update' }
    if (-not $afterSession.alive -or $afterSession.pid -ne $beforeSession.pid) { throw 'Running command did not survive update' }
    $counterAfter = Read-Counter
    Start-Sleep -Seconds 1
    $counterContinued = Read-Counter
    if ($counterAfter -le $counterBefore -or $counterContinued -le $counterAfter) { throw 'Command stopped executing across update' }
    Write-Host "preserved daemon=$($before.daemon_pid), shell=$($beforeSession.pid), counter=$counterBefore -> $counterAfter -> $counterContinued"
    Write-Host '--- installer log (Self-update lines) ---'
    Get-Content -LiteralPath $installerLog.FullName | Select-String -Pattern 'Self-update|Installation process succeeded' | ForEach-Object { $_.Line }
    $events = @(Get-Content -LiteralPath (Join-Path $isolation.ConfigDir 'update-events.jsonl'))
    foreach ($needle in '"event":"update.download_completed"', '"event":"update.install_launched"', '"event":"update.daemon_preserved"', '"event":"update.exiting"') {
        if (-not ($events | Where-Object { $_ -like "*$needle*" })) { throw "update-events.jsonl lacks $needle" }
    }
    $inits = @($events | Where-Object { $_ -like '*"event":"update.init"*' })
    if ($inits.Count -lt 2) { throw "expected update.init from both the old and the relaunched app, got $($inits.Count)" }
    if (Test-Path -LiteralPath $errLog) {
        $panics = Get-Content -LiteralPath $errLog | Where-Object { $_ -match 'panicked|ERROR' }
        if ($panics) { throw "app stderr has errors: $($panics -join '; ')" }
    }
    Write-Host "rehearsal OK: $baseVersion -> 99.0.0 installed in place, app relaunched (pid $($relaunched.Id)), $($events.Count) telemetry lines"

    # Close the UI and explicitly end only this rehearsal's sessions. The
    # next UI startup must retire the now-idle old daemon and use its bundle.
    Stop-Process -Id $relaunched.Id -Force
    $relaunched.WaitForExit()
    foreach ($live in (Invoke-DaemonRequest @{ kind='list_sessions' }).sessions) {
        $null = Invoke-DaemonRequest @{ kind='kill_session'; session_id=$live.id }
    }
    $relaunched = Start-Process -FilePath (Join-Path $installDir 'terminal-manager.exe') -WorkingDirectory $installDir -WindowStyle Hidden -PassThru
    $null = $relaunched.Handle
    if (-not $daemon.WaitForExit(15000)) { throw 'The next UI launch did not retire the idle old daemon' }
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    $current = $null
    do {
        try { $current = Invoke-DaemonRequest @{ kind='hello'; client_version='rehearsal' } } catch { Start-Sleep -Milliseconds 100 }
    } until ($current -or [DateTime]::UtcNow -gt $deadline)
    if (-not $current -or $current.executable -ne (Join-Path $installDir "daemons\$baseVersion\unshit-ptyd.exe")) {
        throw "The new UI did not select its bundled daemon: $($current.executable)"
    }
    Write-Host "idle rollover OK: old daemon $($daemon.Id) exited; UI selected $($current.executable)"
} finally {
    $ErrorActionPreference = 'Continue'
    if ($relaunched) { try { Stop-Process -Id $relaunched.Id -Force } catch {} }
    Get-Process -Name terminal-manager -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($installDir, [System.StringComparison]::OrdinalIgnoreCase) } |
        ForEach-Object { try { Stop-Process -Id $_.Id -Force } catch {} }
    if ($isolation) {
        Start-Sleep -Seconds 1
        Assert-ChildPath $isolation.ConfigDir (Join-Path $env:TEMP 'tm-isolated')
        Exit-TmIsolation -Isolation $isolation -PtydExe $daemonExe
    }
    if ($daemon -and -not $daemon.HasExited) { $daemon.Kill(); $daemon.WaitForExit() }
    $unins = Join-Path $installDir 'unins000.exe'
    if (Test-Path -LiteralPath $unins) {
        $u = Start-Process -FilePath $unins -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -WindowStyle Hidden -PassThru -Wait
        Start-Sleep -Seconds 2
        Write-Host "uninstalled (exit $($u.ExitCode)); install dir left: $(Test-Path -LiteralPath $installDir); key left: $(Test-Path $regKey)"
    }
    if ($isolation) {
        foreach ($root in $env:LOCALAPPDATA, $env:APPDATA) {
            $d = Join-Path $root ("com.godly.terminal.{0}" -f $isolation.Token)
            Assert-ChildPath $d $root
            if (Test-Path -LiteralPath $d) { Remove-Item -Recurse -Force -LiteralPath $d -ErrorAction SilentlyContinue }
        }
    }
    if (-not $KeepArtifacts) {
        Remove-Item -Force -LiteralPath $issBase, $issTarget -ErrorAction SilentlyContinue
        Assert-ChildPath $work $env:TEMP
        Remove-Item -Recurse -Force -LiteralPath $work -ErrorAction SilentlyContinue
    } else {
        Write-Host "artifacts kept under $work (and packaging\_rehearsal-*.iss)"
    }
    Remove-Item Env:TM_UPDATE_FEED_URL, Env:TM_UPDATE_STARTUP_DELAY_MS, Env:TM_UPDATE_INSTALL_SCOPE -ErrorAction SilentlyContinue
    $realVersionAfter = (Get-ItemProperty $realRegKey -ErrorAction SilentlyContinue).DisplayVersion
    if ($realVersionAfter -ne $realVersionBefore) { Write-Error "the real install's DisplayVersion changed ($realVersionBefore -> $realVersionAfter)" }
}
