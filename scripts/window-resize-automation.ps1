#Requires -Version 5.1
<#
  Compatibility wrapper for the Desktop Interaction Regression framework.

  Prefer:
    powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1

  The historical command still works:
    powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest
#>

[CmdletBinding()]
param(
    [int]$DragDelta = 220,
    [int]$ScreenshotScale = 1,
    [double]$Tolerance = 2.0,
    [switch]$SkipSnapTest,
    [switch]$OnlySnapTest,
    [double]$SnapLitRatioThreshold = 0.01,
    [string]$SnapShell = "bash",
    [int]$SnapFillLines = 200,
    [int]$SnapTabbarPx = 88,
    [int]$SnapStatusbarPx = 32,
    [int]$SnapSidebarPx = 252,
    [int]$SnapStripeHeightPx = 12,
    [double]$SnapMidLitRatioThreshold = 0.005
)

$ErrorActionPreference = "Stop"

[void]$ScreenshotScale

$runner = Join-Path $PSScriptRoot "..\tests\windows\desktop-regression\run.ps1"
$suite = if ($OnlySnapTest) {
    @("post-resize-glitches")
} elseif ($SkipSnapTest) {
    @("edge-resize-stability")
} else {
    @("edge-resize-stability", "post-resize-glitches")
}

$forward = @{
    Suite = $suite
    SkipBuild = $true
}

foreach ($name in @(
    "DragDelta",
    "Tolerance",
    "SnapLitRatioThreshold",
    "SnapMidLitRatioThreshold",
    "SnapShell",
    "SnapFillLines",
    "SnapTabbarPx",
    "SnapStatusbarPx",
    "SnapSidebarPx",
    "SnapStripeHeightPx"
)) {
    if ($PSBoundParameters.ContainsKey($name)) {
        $forward[$name] = $PSBoundParameters[$name]
    }
}

& $runner @forward

exit $LASTEXITCODE
