#Requires -Version 5.1
<#
.SYNOPSIS
  Build the GitHub release body for a version from its CHANGELOG.md section.

.DESCRIPTION
  GitHub renders release bodies the way it renders issue comments: every
  single newline becomes a hard <br>. CHANGELOG.md is hard-wrapped at about
  76 columns (Keep a Changelog convention), so a section pasted verbatim shows
  up as a narrow column of short lines. v0.6.0 shipped that way.

  This script extracts the `## [<Version>]` section, joins each hard-wrapped
  paragraph and list item back into one line, promotes `###` headings to `##`
  (the release title already names the version) and prepends an `## Install`
  section. The notes are written as UTF-8 without BOM (PowerShell 5.1's
  Out-File would add one) to -OutFile, ready for

    gh release create v<Version> dist\terminal-manager-<Version>-setup.exe `
      --latest --notes-file dist\release-notes-<Version>.md

  -Verify renders the file through GitHub's markdown API in `gfm` mode, the
  mode release pages use, and fails when the rendered HTML contains a <br>.
  Run it before publishing; it is the check that would have caught v0.6.0.

.PARAMETER Version
  The version whose section to extract, for example 0.6.1 (no leading v).

.PARAMETER ChangelogPath
  Path to CHANGELOG.md. Default: the repository root's.

.PARAMETER IntroFile
  A markdown file whose content replaces the default Install paragraph. Prose
  goes through a file because backticks and `**` in a string parameter would
  fight PowerShell's own escaping.

.PARAMETER OutFile
  Where to write the notes. Default: dist\release-notes-<Version>.md.

.PARAMETER Verify
  Render the notes with `gh api markdown` in gfm mode and fail on any <br>.

.EXAMPLE
  powershell -File scripts\release-notes.ps1 -Version 0.6.1 -Verify
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$ChangelogPath,
  [string]$IntroFile,
  [string]$OutFile,
  [switch]$Verify
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $ChangelogPath) { $ChangelogPath = Join-Path $repoRoot 'CHANGELOG.md' }
if (-not $OutFile) { $OutFile = Join-Path $repoRoot ("dist\release-notes-{0}.md" -f $Version) }

$utf8NoBom = New-Object System.Text.UTF8Encoding $false

function Read-Utf8Text([string]$path) {
  return [System.IO.File]::ReadAllText($path, $utf8NoBom)
}

# ---- 1. extract the version's section --------------------------------------
$lines = (Read-Utf8Text $ChangelogPath) -replace "`r`n", "`n"
$lines = $lines -split "`n"
$headerPattern = '^## \[' + [regex]::Escape($Version) + '\]'
$start = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
  if ($lines[$i] -match $headerPattern) { $start = $i; break }
}
if ($start -lt 0) { throw "No '## [$Version]' section in $ChangelogPath" }
$end = $lines.Count
for ($i = $start + 1; $i -lt $lines.Count; $i++) {
  if ($lines[$i] -match '^## ') { $end = $i; break }
}
if ($end -le $start + 1) { throw "The '## [$Version]' section is empty" }
$section = $lines[($start + 1)..($end - 1)]

# ---- 2. unwrap paragraphs and list items -----------------------------------
# A line starts a new block when it is blank, a heading, a list item (`- `,
# `* `, `+ `, `1. `), a block quote, a table row, a code fence, a link
# reference definition or an HTML tag. Anything else continues the previous
# text line and is joined to it with one space. Fenced code is copied as is,
# and a line ending in two spaces or a backslash keeps its hard break.
$blockStart = '^\s*(#{1,6}\s|[-*+]\s|\d+[.)]\s|>|\||```|~~~|\[[^\]]+\]:\s|<)'
$out = New-Object System.Collections.Generic.List[string]
$inFence = $false
$joinable = $false
$joined = 0
foreach ($line in $section) {
  if ($line -match '^\s*(```|~~~)') {
    $inFence = -not $inFence
    $out.Add($line); $joinable = $false; continue
  }
  if ($inFence) { $out.Add($line); continue }
  if ($line.Trim() -eq '') { $out.Add(''); $joinable = $false; continue }
  if ($line -match '^\s*#{2,6}\s') {
    # `### Added` -> `## Added`: the release title already carries the version.
    $out.Add(($line -replace '^(\s*)#(#+\s)', '$1$2')); $joinable = $false; continue
  }
  # Two trailing spaces or a trailing backslash (\x5c) is a deliberate hard break.
  $hardBreak = $line -match '(  |\x5c)$'
  if ($line -match $blockStart) {
    $out.Add($line)
    $joinable = ($line -match '^\s*([-*+]\s|\d+[.)]\s)') -and -not $hardBreak
    continue
  }
  if ($joinable) {
    $out[$out.Count - 1] = $out[$out.Count - 1] + ' ' + $line.Trim()
    $joined++
  } else {
    $out.Add($line.TrimEnd())
  }
  $joinable = -not $hardBreak
}
while ($out.Count -gt 0 -and $out[0] -eq '') { $out.RemoveAt(0) }
while ($out.Count -gt 0 -and $out[$out.Count - 1] -eq '') { $out.RemoveAt($out.Count - 1) }

# ---- 3. prepend the Install section ----------------------------------------
if ($IntroFile) {
  $intro = (Read-Utf8Text $IntroFile).Trim()
} else {
  # U+203A (the Settings breadcrumb chevron) is built from its code point: PowerShell 5.1
  # reads a BOM-less script as ANSI and would turn a literal one into mojibake.
  $chevron = [string][char]0x203A
  $intro = ('Download `terminal-manager-{0}-setup.exe` below and run it (per-user install, no admin prompt). ' +
            'An installed 0.6.0 or newer offers this release itself at startup and under **Settings {1} Updates**; ' +
            'older installs run the installer over the current copy.') -f $Version, $chevron
}
$body = "## Install`n`n$intro`n`n" + ($out -join "`n") + "`n"

$outDir = Split-Path -Parent $OutFile
if ($outDir -and -not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
[System.IO.File]::WriteAllText($OutFile, $body, $utf8NoBom)
$longest = ($body -split "`n" | ForEach-Object { $_.Length } | Measure-Object -Maximum).Maximum
Write-Host ("release-notes: version={0} section_lines={1} joined_lines={2} longest_line={3} out={4}" -f `
  $Version, $section.Count, $joined, $longest, $OutFile)

# ---- 4. optional render check against GitHub's own renderer ----------------
if ($Verify) {
  $html = & gh api markdown -f mode=gfm -F "text=@$OutFile"
  if ($LASTEXITCODE -ne 0) { throw "gh api markdown failed with exit code $LASTEXITCODE" }
  $breaks = [regex]::Matches(($html -join "`n"), '<br\s*/?>').Count
  if ($breaks -gt 0) {
    throw "$breaks hard line break(s) would render in the release body; unwrap them in $OutFile"
  }
  Write-Host "release-notes: verified gfm render, hard_breaks=0"
}
