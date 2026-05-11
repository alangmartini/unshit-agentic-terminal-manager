#Requires -Version 5.1
<#
  Compatibility wrapper for the Rust Desktop Interaction Regression runner.

  Prefer:
    cargo xtask desktop-regression --list
    cargo xtask desktop-regression --suite edge-resize-stability --observe off

  Existing PowerShell entry points remain available and forward to:
    cargo xtask desktop-regression
#>

[CmdletBinding()]
param(
    [string[]]$Suite,
    [switch]$List,
    [switch]$SkipBuild,
    [string]$ExePath,
    [string]$ArtifactsDir = "artifacts",
    [ValidateSet("off", "basic", "full")]
    [string]$Observe,
    [switch]$Interactive,
    [switch]$KeepOpenOnFailure,
    [switch]$Record,
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
$providedParameters = @{}
foreach ($entry in $PSBoundParameters.GetEnumerator()) {
    $providedParameters[$entry.Key] = $entry.Value
}

function Resolve-DesktopRegressionCargo {
    $stableBin = Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-msvc\bin"
    $stableCargo = Join-Path $stableBin "cargo.exe"

    if (Test-Path -LiteralPath $stableCargo) {
        $env:RUSTC = Join-Path $stableBin "rustc.exe"
        $env:RUSTDOC = Join-Path $stableBin "rustdoc.exe"
        $env:PATH = "$stableBin;$env:PATH"
        return $stableCargo
    }

    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargo) {
        return $cargo.Source
    }

    throw "Could not find cargo. Install Rust or add cargo.exe to PATH."
}

function Convert-LegacyArtifactRoot {
    param([Parameter(Mandatory = $true)][string]$Path)

    return Join-Path $Path "windows\desktop-regression"
}

function Add-ObsoleteFlagWarning {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Explanation
    )

    if ($providedParameters.ContainsKey($Name)) {
        Write-Warning ("-{0} is ignored by the Rust desktop-regression runner; {1}" -f $Name, $Explanation)
    }
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..\..\..")

$argsList = New-Object System.Collections.Generic.List[string]
$argsList.Add("xtask")
$argsList.Add("desktop-regression")

if ($List) {
    $argsList.Add("--list")
} else {
    foreach ($suiteId in @($Suite)) {
        if ([string]::IsNullOrWhiteSpace($suiteId)) {
            continue
        }
        $argsList.Add("--suite")
        $argsList.Add($suiteId)
    }

    if ($SkipBuild) {
        $argsList.Add("--skip-build")
    }

    if ($ExePath) {
        $argsList.Add("--exe-path")
        $argsList.Add($ExePath)
    } elseif ($SkipBuild) {
        $argsList.Add("--exe-path")
        $argsList.Add("target\debug\terminal-manager.exe")
    }

    if ($providedParameters.ContainsKey("ArtifactsDir")) {
        $argsList.Add("--artifact-root")
        $argsList.Add((Convert-LegacyArtifactRoot -Path $ArtifactsDir))
    }

    if ($providedParameters.ContainsKey("Observe")) {
        $argsList.Add("--observe")
        $argsList.Add($Observe)
    }

    if ($Interactive) {
        $argsList.Add("--interactive")
    }

    if ($KeepOpenOnFailure) {
        $argsList.Add("--keep-open-on-failure")
    }

    if ($Record) {
        $argsList.Add("--record")
    }

    Add-ObsoleteFlagWarning -Name "Tolerance" -Explanation "pixel thresholds are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "DragDelta" -Explanation "desktop input distances are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapLitRatioThreshold" -Explanation "snap visual thresholds are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapMidLitRatioThreshold" -Explanation "snap visual thresholds are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapShell" -Explanation "terminal fixture commands are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapFillLines" -Explanation "terminal fixture sizes are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapTabbarPx" -Explanation "layout constants are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapStatusbarPx" -Explanation "layout constants are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapSidebarPx" -Explanation "layout constants are owned by the migrated Rust suites."
    Add-ObsoleteFlagWarning -Name "SnapStripeHeightPx" -Explanation "visual sampling constants are owned by the migrated Rust suites."
}

$cargo = Resolve-DesktopRegressionCargo

Push-Location $repoRoot
try {
    & $cargo @argsList
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
