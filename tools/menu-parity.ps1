<#
.SYNOPSIS
  Score visual parity of the implemented sidebar context menu against the
  design reference, for both the root-menu and flyout states. Produces a
  side-by-side composite and a downsampled color-grid similarity score that
  is robust to font anti-aliasing / sub-pixel drift.
#>
param(
    [string]$AppDir = 'artifacts/sidebar-menu/latest',
    [string]$RefDir = "$env:LOCALAPPDATA/Temp/sidebar_design/screenshots",
    [string]$OutDir = 'artifacts/sidebar-menu/parity'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.Drawing

function New-Dir($p){ if(-not(Test-Path $p)){ New-Item -ItemType Directory -Path $p -Force | Out-Null } }
New-Dir $OutDir

function Load($p){ [System.Drawing.Bitmap]::FromFile((Resolve-Path $p)) }

function CropTo([System.Drawing.Bitmap]$img,[int]$x,[int]$y,[int]$w,[int]$h){
    $x=[Math]::Max(0,$x); $y=[Math]::Max(0,$y)
    $w=[Math]::Min($w,$img.Width-$x); $h=[Math]::Min($h,$img.Height-$y)
    $r=New-Object System.Drawing.Rectangle($x,$y,$w,$h)
    $img.Clone($r,$img.PixelFormat)
}

function Resample([System.Drawing.Bitmap]$src,[int]$w,[int]$h){
    $b=New-Object System.Drawing.Bitmap $w,$h
    $g=[System.Drawing.Graphics]::FromImage($b)
    $g.InterpolationMode=[System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode=[System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($src,0,0,$w,$h); $g.Dispose(); $b
}

# Structural color similarity. Resample both crops to a coarse grid so font
# anti-aliasing and sub-pixel/DPI drift blur into block averages (the standard
# basis for perceptual/structural image comparison), then score each cell on
# RGB distance with a graceful falloff. $gw/$gh set the perceptual scale;
# $cut sets the distance at which a cell scores zero (out of a 441 max).
function GridParity([System.Drawing.Bitmap]$a,[System.Drawing.Bitmap]$b,[int]$gw=26,[int]$gh=34,[double]$cut=170.0){
    $ra=Resample $a $gw $gh; $rb=Resample $b $gw $gh
    try{
        $sum=0.0; $n=$gw*$gh
        for($y=0;$y -lt $gh;$y++){ for($x=0;$x -lt $gw;$x++){
            $ca=$ra.GetPixel($x,$y); $cb=$rb.GetPixel($x,$y)
            $d=[Math]::Sqrt(
                [Math]::Pow($ca.R-$cb.R,2)+[Math]::Pow($ca.G-$cb.G,2)+[Math]::Pow($ca.B-$cb.B,2))
            $score=[Math]::Max(0.0, 1.0 - ($d/$cut))
            $sum+=$score
        }}
        return [Math]::Round(100.0*$sum/$n,1)
    } finally { $ra.Dispose(); $rb.Dispose() }
}

function Composite([System.Drawing.Bitmap]$a,[System.Drawing.Bitmap]$b,[string]$path){
    $h=[Math]::Max($a.Height,$b.Height); $gap=24
    $w=$a.Width+$b.Width+$gap
    $c=New-Object System.Drawing.Bitmap $w,$h
    $g=[System.Drawing.Graphics]::FromImage($c)
    $g.Clear([System.Drawing.Color]::FromArgb(20,17,12))
    $g.DrawImage($a,0,0,$a.Width,$a.Height)
    $g.DrawImage($b,$a.Width+$gap,0,$b.Width,$b.Height)
    $g.Dispose(); $c.Save((Join-Path (Get-Location) $path),[System.Drawing.Imaging.ImageFormat]::Png); $c.Dispose()
}

# --- Reference crops (1200-wide design render) ---
$refMenuImg  = Load (Join-Path $RefDir '02-menu-open.png')
$refFlyImg   = Load (Join-Path $RefDir '02-flyout.png')
# --- App crops (1920x1200 capture, 150% DPI) ---
$appMenuImg  = Load (Join-Path $AppDir 'menu-open.png')
$appFlyImg   = Load (Join-Path $AppDir 'flyout.png')

try {
    # Reference menu panel and flyout bounds (measured from the design render).
    $refMenu = CropTo $refMenuImg 94 157 226 206
    $refFly  = CropTo $refFlyImg  94 157 446 320
    # App menu panel and flyout bounds (physical px, 150% DPI capture).
    $appMenu = CropTo $appMenuImg 205 137 333 283
    $appFly  = CropTo $appFlyImg  205 137 627 405

    try {
        $appMenu.Save((Join-Path (Get-Location) "$OutDir/app-menu.png"))
        $refMenu.Save((Join-Path (Get-Location) "$OutDir/ref-menu.png"))
        $appFly.Save((Join-Path (Get-Location) "$OutDir/app-flyout.png"))
        $refFly.Save((Join-Path (Get-Location) "$OutDir/ref-flyout.png"))

        Composite $refMenu $appMenu "$OutDir/compare-menu.png"
        Composite $refFly  $appFly  "$OutDir/compare-flyout.png"

        $menuScore = GridParity $refMenu $appMenu
        $flyScore  = GridParity $refFly  $appFly
        $overall   = [Math]::Round(($menuScore+$flyScore)/2.0,1)

        [pscustomobject]@{
            menu_parity_pct   = $menuScore
            flyout_parity_pct = $flyScore
            overall_pct       = $overall
            composites        = @("$OutDir/compare-menu.png","$OutDir/compare-flyout.png")
        } | ConvertTo-Json
    } finally { $refMenu.Dispose(); $refFly.Dispose(); $appMenu.Dispose(); $appFly.Dispose() }
} finally { $refMenuImg.Dispose(); $refFlyImg.Dispose(); $appMenuImg.Dispose(); $appFlyImg.Dispose() }
