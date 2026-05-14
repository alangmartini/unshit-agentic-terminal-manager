#Requires -Version 5.1
<#
  Compatibility wrapper for the Rust isolated sequential desktop-regression
  mode.

  Prefer:
    cargo xtask desktop-regression --sequential-isolated

  Existing PowerShell entry point:
    powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run-all-sequential.ps1
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
Set-StrictMode -Version Latest

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

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..\..\..")
$cargo = Resolve-DesktopRegressionCargo

$argsList = New-Object System.Collections.Generic.List[string]
$argsList.Add("xtask")
$argsList.Add("desktop-regression")

if ($List) {
    $argsList.Add("--list")
} else {
    $argsList.Add("--sequential-isolated")
    foreach ($suiteId in @($Suite)) {
        if ([string]::IsNullOrWhiteSpace($suiteId)) {
            continue
        }
        $argsList.Add("--suite")
        $argsList.Add($suiteId)
    }

    $argsList.Add("--observe")
    $argsList.Add($Observe)
    $argsList.Add("--artifact-root")
    $argsList.Add($ArtifactRoot)

    if ($SkipBuild) {
        $argsList.Add("--skip-build")
        $argsList.Add("--exe-path")
        $argsList.Add($ExePath)
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
}

Push-Location $repoRoot
try {
    & $cargo @argsList
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
