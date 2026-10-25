#Requires -Version 5.1
<#
  Stand-in for an agent CLI pane, used by scripts/idle-cpu-bench.ps1.

  Reads its mode from -ModeFile (default: bench-mode.txt in the working
  directory, so one shell spec can drive panes in different workspaces)
  every half second and animates what a busy Claude Code pane animates:
    title    cycles the OSC 0 window title through half-circle frames, the
             way the sidebar and tab labels show a working agent
    spinner  rewrites one status line with a spinner glyph and timer
    notify   sends one OSC 9 notification on entering the mode, which puts
             a background pane into the pulsing attention state and raises
             a real desktop notification for it
    idle     sets a static title once and prints nothing
  A mode string can combine words ("title+spinner").

  ASCII only on purpose: Windows PowerShell 5.1 reads a BOM-less script as
  the ANSI code page, so every non-ASCII glyph is built from its code point.
  The daemon appends `-NoExit -Command "Set-Location -LiteralPath '<cwd>'"`
  to every PowerShell pane with a working directory; with -File those
  arrive here as trailing arguments, and the quoted path is the workspace.
#>
param(
    [string]$ModeFile = '',
    [string]$Name = '',
    [int]$IntervalMs = 100,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$DaemonArgs = @()
)

$workspace = (Get-Location).Path
$setLocation = ($DaemonArgs -join ' ') -replace '^.*-LiteralPath\s+', ''
if ($setLocation -match "^'(.*)'$") { $workspace = $Matches[1] -replace "''", "'" }
if (-not $ModeFile) { $ModeFile = Join-Path $workspace 'bench-mode.txt' }
if (-not $Name) { $Name = Split-Path -Leaf $workspace }

[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$esc = [char]27
$bel = [char]7
$idleMark = [char]0x2733
$ellipsis = [char]0x2026
$titleFrames = @([char]0x25D0, [char]0x25D3, [char]0x25D1, [char]0x25D2)
$spinFrames = @([char]0x00B7, [char]0x2722, [char]0x2733, [char]0x2736, [char]0x273B, [char]0x273D)

function Set-Title([string]$text) { [Console]::Write("$esc]0;$text$bel") }

Write-Host "fake agent $Name"
Set-Title "$idleMark $Name"
$mode = 'idle'
$titleAnimated = $false
$notified = $false
$clock = [Diagnostics.Stopwatch]::StartNew()
$tick = 0
while ($true) {
    if ($tick % [Math]::Max(1, [int](500 / $IntervalMs)) -eq 0) {
        try { $mode = ([IO.File]::ReadAllText($ModeFile)).Trim() } catch {}
    }
    if ($mode -match 'title') {
        Set-Title "$($titleFrames[$tick % $titleFrames.Count]) $Name"
        $titleAnimated = $true
    } elseif ($titleAnimated) {
        Set-Title "$idleMark $Name"
        $titleAnimated = $false
    }
    if ($mode -match 'spinner') {
        $seconds = [int]$clock.Elapsed.TotalSeconds
        [Console]::Write("`r$($spinFrames[$tick % $spinFrames.Count]) Germinating$ellipsis (${seconds}s)   ")
    }
    if ($mode -match 'notify') {
        if (-not $notified) { [Console]::Write("$esc]9;$Name needs input$bel") }
        $notified = $true
    } else {
        $notified = $false
    }
    $tick++
    [Threading.Thread]::Sleep($IntervalMs)
}
