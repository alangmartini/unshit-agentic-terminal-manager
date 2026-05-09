#Requires -Version 5.1
<#
  Compatibility wrapper for the Desktop Interaction Regression framework.

  The canonical Windows framework now lives under
  tests\windows\desktop-regression so it is grouped with the rest of the
  project's test assets. This wrapper is kept for existing local commands and
  agent notes.
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

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$runner = Join-Path $repoRoot "tests\windows\desktop-regression\run.ps1"
if (-not (Test-Path -LiteralPath $runner)) {
    throw "Canonical desktop regression runner not found: $runner"
}

$forward = @{}
foreach ($entry in $PSBoundParameters.GetEnumerator()) {
    $forward[$entry.Key] = $entry.Value
}

& $runner @forward
exit $LASTEXITCODE
