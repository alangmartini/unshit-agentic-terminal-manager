#Requires -Version 5.1
<#
  Compatibility wrapper for the sequential Desktop Interaction Regression
  runner. The canonical script lives under tests\windows\desktop-regression.
#>

[CmdletBinding()]
param(
    [string[]]$Suite,
    [switch]$List,
    [switch]$SkipBuild,
    [string]$ExePath = "target\debug\terminal-manager.exe",
    [string]$ArtifactRoot = "artifacts\windows\desktop-regression-sequential",
    [ValidateSet("off", "basic", "full")]
    [string]$Observe = "basic",
    [switch]$Interactive,
    [switch]$KeepOpenOnFailure,
    [switch]$Record
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$runner = Join-Path $repoRoot "tests\windows\desktop-regression\run-all-sequential.ps1"
if (-not (Test-Path -LiteralPath $runner)) {
    throw "Canonical sequential desktop regression runner not found: $runner"
}

$forward = @{}
foreach ($entry in $PSBoundParameters.GetEnumerator()) {
    $forward[$entry.Key] = $entry.Value
}

& $runner @forward
exit $LASTEXITCODE
