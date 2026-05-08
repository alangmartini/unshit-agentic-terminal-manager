#Requires -Version 5.1
<#
  Compatibility wrapper for the Desktop Interaction Regression harness.

  Prefer:
    powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1

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

$runner = Join-Path $PSScriptRoot "desktop-regression\run.ps1"
$suite = if ($OnlySnapTest) {
    @("post-resize-glitches")
} elseif ($SkipSnapTest) {
    @("edge-resize-stability")
} else {
    @("edge-resize-stability", "post-resize-glitches")
}

& $runner `
    -Suite $suite `
    -SkipBuild `
    -DragDelta $DragDelta `
    -Tolerance $Tolerance `
    -SnapLitRatioThreshold $SnapLitRatioThreshold `
    -SnapMidLitRatioThreshold $SnapMidLitRatioThreshold `
    -SnapShell $SnapShell `
    -SnapFillLines $SnapFillLines `
    -SnapTabbarPx $SnapTabbarPx `
    -SnapStatusbarPx $SnapStatusbarPx `
    -SnapSidebarPx $SnapSidebarPx `
    -SnapStripeHeightPx $SnapStripeHeightPx

exit $LASTEXITCODE
