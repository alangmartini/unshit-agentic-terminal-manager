#Requires -Version 5.1
<#
  Desktop Interaction Regression framework runner.

  These suites drive a real Windows desktop: native windows, global input,
  compositor snap behavior, screenshots, and pixel assertions. They are not
  unit tests, browser-style e2e tests, or CI jobs.

  Run all suites:
    powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1

  Run one suite:
    powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches
#>

[CmdletBinding()]
param(
    [string[]]$Suite,
    [switch]$List,
    [switch]$SkipBuild,
    [string]$ExePath,
    [string]$ArtifactsDir = "artifacts",
    [double]$Tolerance = 2.0,
    [int]$DragDelta = 220,
    [double]$SnapLitRatioThreshold = 0.01,
    [double]$SnapMidLitRatioThreshold = 0.005,
    [string]$SnapShell = "bash",
    [int]$SnapFillLines = 200,
    [int]$SnapTabbarPx = 88,
    [int]$SnapStatusbarPx = 32,
    [int]$SnapSidebarPx = 252,
    [int]$SnapStripeHeightPx = 12
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..\..\..")
$libPath = Join-Path $scriptDir "lib\DesktopRegression.ps1"
. $libPath

$suiteDir = Join-Path $scriptDir "suites"
Get-ChildItem -Path $suiteDir -Filter "*.ps1" |
    Sort-Object Name |
    ForEach-Object { . $_.FullName }

$registered = Get-DesktopRegressionSuites

if ($List) {
    foreach ($item in $registered) {
        Write-Output ("{0} - {1}" -f $item.Name, $item.Title)
        Write-Output ("  covers: {0}" -f $item.Covers)
        if ($item.Tags.Count -gt 0) {
            Write-Output ("  tags: {0}" -f ($item.Tags -join ", "))
        }
    }
    exit 0
}

Initialize-DesktopRegressionWin32

if (-not $ExePath) {
    $ExePath = Join-Path $repoRoot "target\debug\terminal-manager.exe"
} elseif (-not [System.IO.Path]::IsPathRooted($ExePath)) {
    $ExePath = Join-Path $repoRoot $ExePath
}

if (-not [System.IO.Path]::IsPathRooted($ArtifactsDir)) {
    $ArtifactsDir = Join-Path $repoRoot $ArtifactsDir
}
if (-not (Test-Path $ArtifactsDir)) {
    New-Item -ItemType Directory -Path $ArtifactsDir | Out-Null
}

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        Write-Output "building target: cargo build"
        & cargo build
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path $ExePath)) {
    throw "Missing binary: $ExePath"
}

$selectedSuites = @()
if ($Suite -and $Suite.Count -gt 0) {
    foreach ($name in $Suite) {
        $match = $registered | Where-Object { $_.Name -eq $name }
        if (-not $match) {
            $known = ($registered | ForEach-Object { $_.Name }) -join ", "
            throw "Unknown desktop regression suite '$name'. Known suites: $known"
        }
        $selectedSuites += $match
    }
} else {
    $selectedSuites = $registered
}

$context = New-DesktopRegressionContext `
    -RepoRoot $repoRoot `
    -ExePath $ExePath `
    -ArtifactsRoot $ArtifactsDir `
    -Tolerance $Tolerance `
    -DragDelta $DragDelta `
    -SnapLitRatioThreshold $SnapLitRatioThreshold `
    -SnapMidLitRatioThreshold $SnapMidLitRatioThreshold `
    -SnapShell $SnapShell `
    -SnapFillLines $SnapFillLines `
    -SnapTabbarPx $SnapTabbarPx `
    -SnapStatusbarPx $SnapStatusbarPx `
    -SnapSidebarPx $SnapSidebarPx `
    -SnapStripeHeightPx $SnapStripeHeightPx

Write-Output "Desktop Interaction Regression suites"
Write-Output ("run_id={0}" -f $context.RunId)
Write-Output ("artifacts={0}" -f $context.RunArtifactsDir)
Write-Output ("selected={0}" -f (($selectedSuites | ForEach-Object { $_.Name }) -join ", "))

$results = @()
$failed = $false

foreach ($item in $selectedSuites) {
    Write-Output ""
    Write-Output ("[RUN] {0} - {1}" -f $item.Name, $item.Title)
    Write-Output ("covers: {0}" -f $item.Covers)

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        & $item.ScriptBlock $context
        $sw.Stop()
        Write-Output ("[PASS] {0} ({1:N2}s)" -f $item.Name, $sw.Elapsed.TotalSeconds)
        $results += [pscustomobject]@{
            name = $item.Name
            title = $item.Title
            status = "passed"
            duration_seconds = [Math]::Round($sw.Elapsed.TotalSeconds, 3)
        }
    } catch {
        $sw.Stop()
        $failed = $true
        Write-Output ("[FAIL] {0} ({1:N2}s): {2}" -f $item.Name, $sw.Elapsed.TotalSeconds, $_.Exception.Message)
        $results += [pscustomobject]@{
            name = $item.Name
            title = $item.Title
            status = "failed"
            duration_seconds = [Math]::Round($sw.Elapsed.TotalSeconds, 3)
            error = $_.Exception.Message
        }
    }
}

$resultsPath = Join-Path $context.RunArtifactsDir "results.json"
$results | ConvertTo-Json -Depth 5 | Set-Content -Path $resultsPath -Encoding UTF8
Write-Output ""
Write-Output ("results={0}" -f $resultsPath)

if ($failed) {
    Write-Output "FAIL Desktop Interaction Regression"
    exit 1
}

Write-Output "PASS Desktop Interaction Regression"
