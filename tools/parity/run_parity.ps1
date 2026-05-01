param(
    [string]$ArtifactDir = 'artifacts/parity/latest',
    [string]$TerminalManagerExe = '',
    [string]$DaemonExe = '',
    [string]$WtExe = 'wt.exe',
    [string]$PowerShellExe = 'pwsh.exe',
    [string]$WtProfile = '',
    [string]$WtColorScheme = '',
    [string]$TerminalManagerCrop = '',
    [string]$WindowsTerminalCrop = '',
    [int]$Cols = 100,
    [int]$Rows = 30,
    [int]$WindowX = 32,
    [int]$WindowY = 32,
    [int]$WindowWidth = 1280,
    [int]$WindowHeight = 800,
    [int]$TerminalManagerWindowWidth = 0,
    [int]$TerminalManagerWindowHeight = 0,
    [int]$WindowsTerminalWindowWidth = 0,
    [int]$WindowsTerminalWindowHeight = 0,
    [int]$CaptureDelayMs = 9000,
    [int]$SceneHoldSeconds = 20,
    [int]$SceneInitialDelayMs = 5000,
    [int]$SceneStableSizeMs = 750,
    [int]$SceneMaxSizeWaitMs = 3000,
    [int]$WindowTimeoutSeconds = 60,
    [int]$Tolerance = 8,
    [ValidateSet('Window', 'Screen')][string]$CaptureMode = 'Window',
    [string]$TerminalManagerCaptureMode = '',
    [string]$WindowsTerminalCaptureMode = '',
    [switch]$SkipBuild,
    [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $PSCommandPath
. (Join-Path $scriptDir 'ParityLib.ps1')

Add-ParityWin32Types
$isWindowsHost = [System.IO.Path]::DirectorySeparatorChar -eq '\'

$repoRoot = (Resolve-Path (Join-Path $scriptDir '..\..')).Path
$absoluteArtifactDir = if ([System.IO.Path]::IsPathRooted($ArtifactDir)) {
    $ArtifactDir
} else {
    Join-Path $repoRoot $ArtifactDir
}
New-ParityDirectory -Path $absoluteArtifactDir

if ($SelfTest) {
    $selfTestDir = Join-Path $absoluteArtifactDir 'selftest'
    $result = Invoke-ParitySelfTest -OutDir $selfTestDir
    $result | ConvertTo-Json -Depth 5
    exit 0
}

$runId = [Guid]::NewGuid().ToString('N').Substring(0, 12)
$smokeScript = (Resolve-Path (Join-Path $scriptDir 'smoke-scene.ps1')).Path
$terminalManagerFullPath = Join-Path $absoluteArtifactDir 'terminal-manager-full.png'
$terminalManagerCropPath = Join-Path $absoluteArtifactDir 'terminal-manager.png'
$windowsTerminalFullPath = Join-Path $absoluteArtifactDir 'windows-terminal-full.png'
$windowsTerminalCropPath = Join-Path $absoluteArtifactDir 'windows-terminal.png'
$diffPath = Join-Path $absoluteArtifactDir 'diff.png'
$reportPath = Join-Path $absoluteArtifactDir 'report.json'
$terminalManagerSceneMetaPath = Join-Path $absoluteArtifactDir 'terminal-manager-scene.json'
$windowsTerminalSceneMetaPath = Join-Path $absoluteArtifactDir 'windows-terminal-scene.json'

function Invoke-Checked {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath $($Arguments -join ' ') failed with exit code $LASTEXITCODE."
    }
}

function Resolve-TerminalManagerPath {
    param([string]$ExplicitPath)

    if ($ExplicitPath -ne '') {
        return (Resolve-Path $ExplicitPath).Path
    }

    $name = if ($isWindowsHost) { 'terminal-manager.exe' } else { 'terminal-manager' }
    return (Join-Path $repoRoot "target\debug\$name")
}

function Resolve-DaemonPath {
    param(
        [string]$ExplicitPath,
        [string]$TerminalManagerExePath
    )

    $name = if ($isWindowsHost) { 'unshit-ptyd.exe' } else { 'unshit-ptyd' }

    if ($ExplicitPath -ne '') {
        return (Resolve-Path $ExplicitPath).Path
    }

    if ($TerminalManagerExePath -ne '') {
        $siblingPath = Join-Path (Split-Path -Parent $TerminalManagerExePath) $name
        if (Test-Path -LiteralPath $siblingPath) {
            return (Resolve-Path $siblingPath).Path
        }
    }

    return (Join-Path $repoRoot "target\debug\$name")
}

function Resolve-PositiveIntOverride {
    param(
        [int]$Value,
        [int]$DefaultValue
    )

    if ($Value -gt 0) {
        return $Value
    }
    $DefaultValue
}

function Resolve-CaptureModeOverride {
    param(
        [string]$Value,
        [string]$DefaultValue
    )

    if ($Value -eq '') {
        return $DefaultValue
    }
    if ($Value -ne 'Window' -and $Value -ne 'Screen') {
        throw "Capture mode '$Value' must be Window or Screen."
    }
    $Value
}

function New-SmokeSceneArgs {
    param([Parameter(Mandatory)][string]$MetaPath)

    @(
        '-NoLogo',
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        $smokeScript,
        '-HoldSeconds',
        [string]$SceneHoldSeconds,
        '-InitialDelayMs',
        [string]$SceneInitialDelayMs,
        '-StableSizeMs',
        [string]$SceneStableSizeMs,
        '-MaxSizeWaitMs',
        [string]$SceneMaxSizeWaitMs,
        '-MetaPath',
        $MetaPath
    )
}

function Read-ParityJsonFile {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Get-ParitySceneSize {
    param($Scene)

    if ($null -eq $Scene -or $null -eq $Scene.size_before_output) {
        return $null
    }
    $Scene.size_before_output
}

function Stop-ParityProcess {
    param($Process)

    if ($null -eq $Process) {
        return
    }
    try {
        $Process.Refresh()
        if (-not $Process.HasExited) {
            Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
        }
    } catch {
        Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
    }
}

function Stop-ParityDaemon {
    param(
        [string]$DaemonExePath,
        [string]$SocketPath,
        [int]$TimeoutSeconds
    )

    if (-not (Test-Path -LiteralPath $DaemonExePath)) {
        return
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastOutput = ''
    do {
        $output = & $DaemonExePath --shutdown --socket $SocketPath 2>&1
        $lastOutput = ($output | Out-String).Trim()
        if ($LASTEXITCODE -eq 0) {
            return $true
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)

    return $false
}

function Get-ParityDaemonIds {
    param([string]$DaemonExePath)

    $resolved = (Resolve-Path $DaemonExePath).Path
    @(
        Get-Process unshit-ptyd -ErrorAction SilentlyContinue |
            Where-Object {
                try {
                    $_.Path -eq $resolved
                } catch {
                    $false
                }
            } |
            Select-Object -ExpandProperty Id
    )
}

function Get-ParityDescendantProcessIds {
    param([int[]]$RootIds)

    $all = New-Object System.Collections.Generic.List[int]
    $frontier = @($RootIds)
    while ($frontier.Count -gt 0) {
        $children = @(
            Get-CimInstance Win32_Process |
                Where-Object { $frontier -contains [int]$_.ParentProcessId }
        )
        $frontier = @()
        foreach ($child in $children) {
            $childPid = [int]$child.ProcessId
            if (-not $all.Contains($childPid)) {
                $all.Add($childPid)
                $frontier += $childPid
            }
        }
    }

    return @($all)
}

function Stop-NewParityDaemons {
    param(
        [string]$DaemonExePath,
        [int[]]$ExistingIds
    )

    $currentIds = Get-ParityDaemonIds -DaemonExePath $DaemonExePath
    $newIds = @($currentIds | Where-Object { $ExistingIds -notcontains $_ })
    if ($newIds.Count -eq 0) {
        return
    }

    $childIds = Get-ParityDescendantProcessIds -RootIds $newIds
    foreach ($processId in @($childIds | Sort-Object -Descending)) {
        Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
    }
    foreach ($processId in $newIds) {
        Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
    }
}

function Start-TerminalManagerParity {
    param(
        [string]$ExePath,
        [string]$SocketPath,
        [string[]]$SmokeArgs
    )

    $envKeys = @(
        'TM_PTYD_SOCKET',
        'TM_PARITY_SHELL_PROGRAM',
        'TM_PARITY_SHELL_ARGS_JSON',
        'TM_PARITY_WINDOWS_TERMINAL_COLORS'
    )
    $previous = @{}
    foreach ($key in $envKeys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
    }

    try {
        [Environment]::SetEnvironmentVariable('TM_PTYD_SOCKET', $SocketPath, 'Process')
        [Environment]::SetEnvironmentVariable('TM_PARITY_SHELL_PROGRAM', $PowerShellExe, 'Process')
        [Environment]::SetEnvironmentVariable(
            'TM_PARITY_SHELL_ARGS_JSON',
            ($SmokeArgs | ConvertTo-Json -Compress),
            'Process'
        )
        [Environment]::SetEnvironmentVariable('TM_PARITY_WINDOWS_TERMINAL_COLORS', '1', 'Process')
        return Start-Process -FilePath $ExePath -WorkingDirectory $repoRoot -PassThru -WindowStyle Normal
    } finally {
        foreach ($key in $envKeys) {
            [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process')
        }
    }
}

function Start-WindowsTerminalParity {
    param(
        [string]$Title,
        [string[]]$SmokeArgs
    )

    $args = @(
        '--window', 'new',
        '--pos', "$WindowX,$WindowY",
        '--size', "$Cols,$Rows",
        'new-tab'
    )
    if ($WtProfile -ne '') {
        $args += @('--profile', $WtProfile)
    }
    if ($WtColorScheme -ne '') {
        $args += @('--colorScheme', $WtColorScheme)
    }
    $args += @('--title', $Title, '--suppressApplicationTitle', $PowerShellExe)
    $args += $SmokeArgs

    return Start-Process -FilePath $WtExe -ArgumentList $args -WorkingDirectory $repoRoot -PassThru -WindowStyle Normal
}

if (-not $SkipBuild -and $TerminalManagerExe -eq '') {
    Push-Location $repoRoot
    try {
        Invoke-Checked -FilePath 'cargo' -Arguments @('build', '--bin', 'terminal-manager')
        Invoke-Checked -FilePath 'cargo' -Arguments @('build', '-p', 'unshit-ptyd')
    } finally {
        Pop-Location
    }
}

$terminalManagerExePath = Resolve-TerminalManagerPath -ExplicitPath $TerminalManagerExe
$daemonExePath = Resolve-DaemonPath `
    -ExplicitPath $DaemonExe `
    -TerminalManagerExePath $terminalManagerExePath
if (-not (Test-Path -LiteralPath $terminalManagerExePath)) {
    throw "terminal-manager binary not found at $terminalManagerExePath. Run without -SkipBuild or pass -TerminalManagerExe."
}
if (-not (Test-Path -LiteralPath $daemonExePath)) {
    throw "unshit-ptyd binary not found at $daemonExePath. Run without -SkipBuild first."
}

$terminalManagerSmokeArgs = New-SmokeSceneArgs -MetaPath $terminalManagerSceneMetaPath
$windowsTerminalSmokeArgs = New-SmokeSceneArgs -MetaPath $windowsTerminalSceneMetaPath

$existingDaemonIds = Get-ParityDaemonIds -DaemonExePath $daemonExePath

$socketPath = if ($isWindowsHost) {
    "\\.\pipe\unshit-ptyd-parity-$PID-$runId"
} else {
    Join-Path ([System.IO.Path]::GetTempPath()) "unshit-ptyd-parity-$PID-$runId.sock"
}

$terminalManagerBounds = [pscustomobject]@{
    x = $WindowX
    y = $WindowY
    width = Resolve-PositiveIntOverride -Value $TerminalManagerWindowWidth -DefaultValue $WindowWidth
    height = Resolve-PositiveIntOverride -Value $TerminalManagerWindowHeight -DefaultValue $WindowHeight
}
$windowsTerminalBounds = [pscustomobject]@{
    x = $WindowX
    y = $WindowY
    width = Resolve-PositiveIntOverride -Value $WindowsTerminalWindowWidth -DefaultValue $WindowWidth
    height = Resolve-PositiveIntOverride -Value $WindowsTerminalWindowHeight -DefaultValue $WindowHeight
}
$terminalManagerResolvedCaptureMode = Resolve-CaptureModeOverride `
    -Value $TerminalManagerCaptureMode `
    -DefaultValue $(if ($PSBoundParameters.ContainsKey('CaptureMode')) { $CaptureMode } else { 'Screen' })
$windowsTerminalResolvedCaptureMode = Resolve-CaptureModeOverride `
    -Value $WindowsTerminalCaptureMode `
    -DefaultValue $(if ($PSBoundParameters.ContainsKey('CaptureMode')) { $CaptureMode } else { 'Window' })

$terminalManagerProcess = $null
$terminalManagerWindow = $null
$windowsTerminalProcess = $null
$windowsTerminalWindow = $null

try {
    $terminalManagerProcess = Start-TerminalManagerParity `
        -ExePath $terminalManagerExePath `
        -SocketPath $socketPath `
        -SmokeArgs $terminalManagerSmokeArgs
    $terminalManagerWindow = Wait-ParityProcessWindow `
        -Process $terminalManagerProcess `
        -MinWidth 600 `
        -MinHeight 400 `
        -TimeoutSeconds $WindowTimeoutSeconds
    Set-ParityWindowBounds `
        -WindowProcess $terminalManagerWindow `
        -X $terminalManagerBounds.x `
        -Y $terminalManagerBounds.y `
        -Width $terminalManagerBounds.width `
        -Height $terminalManagerBounds.height
    Start-Sleep -Milliseconds $CaptureDelayMs
    $terminalManagerRect = Capture-ParityWindow `
        -WindowProcess $terminalManagerWindow `
        -Path $terminalManagerFullPath `
        -Mode $terminalManagerResolvedCaptureMode

    $terminalManagerCropRect = if ($TerminalManagerCrop -ne '') {
        ConvertTo-ParityRect -Spec $TerminalManagerCrop
    } else {
        Get-DefaultTerminalManagerCropRect `
            -Width $terminalManagerRect.Width `
            -Height $terminalManagerRect.Height
    }
    Save-ParityImageCrop `
        -SourcePath $terminalManagerFullPath `
        -DestPath $terminalManagerCropPath `
        -Rect $terminalManagerCropRect
} finally {
    Stop-ParityProcess -Process $terminalManagerProcess
    $daemonStopped = Stop-ParityDaemon `
        -DaemonExePath $daemonExePath `
        -SocketPath $socketPath `
        -TimeoutSeconds ([Math]::Max(5, $SceneHoldSeconds + 5))
    if (-not $daemonStopped) {
        Stop-NewParityDaemons -DaemonExePath $daemonExePath -ExistingIds $existingDaemonIds
    }
}

$wtTitle = "godly-parity-wt-$runId"
try {
    $windowsTerminalProcess = Start-WindowsTerminalParity -Title $wtTitle -SmokeArgs $windowsTerminalSmokeArgs
    $windowsTerminalWindow = Wait-ParityTopLevelWindow `
        -TitleLike "*$wtTitle*" `
        -MinWidth 600 `
        -MinHeight 300 `
        -TimeoutSeconds $WindowTimeoutSeconds
    Set-ParityWindowBounds `
        -WindowProcess $windowsTerminalWindow `
        -X $windowsTerminalBounds.x `
        -Y $windowsTerminalBounds.y `
        -Width $windowsTerminalBounds.width `
        -Height $windowsTerminalBounds.height
    Start-Sleep -Milliseconds $CaptureDelayMs
    $windowsTerminalRect = Capture-ParityWindow `
        -WindowProcess $windowsTerminalWindow `
        -Path $windowsTerminalFullPath `
        -Mode $windowsTerminalResolvedCaptureMode

    $windowsTerminalCropRect = if ($WindowsTerminalCrop -ne '') {
        ConvertTo-ParityRect -Spec $WindowsTerminalCrop
    } else {
        Get-DefaultWindowsTerminalCropRect `
            -Width $windowsTerminalRect.Width `
            -Height $windowsTerminalRect.Height `
            -CropWidth $terminalManagerCropRect.Width `
            -CropHeight $terminalManagerCropRect.Height
    }
    Save-ParityImageCrop `
        -SourcePath $windowsTerminalFullPath `
        -DestPath $windowsTerminalCropPath `
        -Rect $windowsTerminalCropRect
} finally {
    Stop-ParityProcess -Process $windowsTerminalWindow
    Stop-ParityProcess -Process $windowsTerminalProcess
}

$comparison = Compare-ParityImages `
    -ReferencePath $windowsTerminalCropPath `
    -ActualPath $terminalManagerCropPath `
    -DiffPath $diffPath `
    -Tolerance $Tolerance

$windowsTerminalScene = Read-ParityJsonFile -Path $windowsTerminalSceneMetaPath
$terminalManagerScene = Read-ParityJsonFile -Path $terminalManagerSceneMetaPath
$windowsTerminalSize = Get-ParitySceneSize -Scene $windowsTerminalScene
$terminalManagerSize = Get-ParitySceneSize -Scene $terminalManagerScene
$sceneSizeComparison = if ($null -ne $windowsTerminalSize -and $null -ne $terminalManagerSize) {
    [pscustomobject]@{
        match = (
            $windowsTerminalSize.cols -eq $terminalManagerSize.cols -and
            $windowsTerminalSize.rows -eq $terminalManagerSize.rows
        )
        cols_delta = [int]$terminalManagerSize.cols - [int]$windowsTerminalSize.cols
        rows_delta = [int]$terminalManagerSize.rows - [int]$windowsTerminalSize.rows
        windows_terminal = $windowsTerminalSize
        terminal_manager = $terminalManagerSize
    }
} else {
    $null
}

$report = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString('o')
    repo_root = $repoRoot
    smoke_scene = $smokeScript
    capture = [pscustomobject]@{
        cols = $Cols
        rows = $Rows
        window = [pscustomobject]@{
            x = $WindowX
            y = $WindowY
            width = $WindowWidth
            height = $WindowHeight
        }
        terminal_manager_window = $terminalManagerBounds
        windows_terminal_window = $windowsTerminalBounds
        capture_delay_ms = $CaptureDelayMs
        capture_mode = $CaptureMode
        terminal_manager_capture_mode = $terminalManagerResolvedCaptureMode
        windows_terminal_capture_mode = $windowsTerminalResolvedCaptureMode
        terminal_manager_visual_profile = 'WindowsTerminal'
        scene_hold_seconds = $SceneHoldSeconds
        scene_initial_delay_ms = $SceneInitialDelayMs
        scene_stable_size_ms = $SceneStableSizeMs
        scene_max_size_wait_ms = $SceneMaxSizeWaitMs
        tolerance = $Tolerance
    }
    binaries = [pscustomobject]@{
        terminal_manager = $terminalManagerExePath
        unshit_ptyd = $daemonExePath
        wt = $WtExe
        powershell = $PowerShellExe
    }
    windows_terminal = [pscustomobject]@{
        full = $windowsTerminalFullPath
        crop = $windowsTerminalCropPath
        crop_rect = $windowsTerminalCropRect
        title = $wtTitle
        profile = $WtProfile
        color_scheme = $WtColorScheme
        scene = $windowsTerminalScene
    }
    terminal_manager = [pscustomobject]@{
        full = $terminalManagerFullPath
        crop = $terminalManagerCropPath
        crop_rect = $terminalManagerCropRect
        socket = $socketPath
        scene = $terminalManagerScene
    }
    comparison = $comparison
    scene_size_comparison = $sceneSizeComparison
    artifacts = [pscustomobject]@{
        windows_terminal = $windowsTerminalCropPath
        terminal_manager = $terminalManagerCropPath
        diff = $diffPath
        report = $reportPath
        windows_terminal_scene = $windowsTerminalSceneMetaPath
        terminal_manager_scene = $terminalManagerSceneMetaPath
    }
    notes = @(
        'Windows Terminal font and color settings come from the selected/default Windows Terminal profile.',
        'Use -TerminalManagerCrop and -WindowsTerminalCrop as x,y,width,height to calibrate crops after inspecting the full captures.'
    )
}

$report | ConvertTo-Json -Depth 8 | Set-Content -Path $reportPath -Encoding UTF8
$report | ConvertTo-Json -Depth 8
