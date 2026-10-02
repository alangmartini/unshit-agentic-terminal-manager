#Requires -Version 5.1
$ErrorActionPreference = 'Stop'

function Invoke-Native([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Command failed with exit code $LASTEXITCODE" }
}

$metadata = & cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$version = ($metadata.packages | Where-Object name -eq 'terminal-manager').version
if ($env:RELEASE_REF -match '^v' -and $env:RELEASE_REF -cne "v$version") {
    throw "Tag $env:RELEASE_REF does not match Cargo.toml version $version"
}
# The checkout is the exact release revision, not the moving default branch.
$repo = Get-Content $env:GITHUB_EVENT_PATH -Raw | ConvertFrom-Json
Invoke-Native git @('fetch', 'origin', $repo.repository.default_branch)
Invoke-Native git @('merge-base', '--is-ancestor', 'HEAD', "origin/$($repo.repository.default_branch)")

# Prevent stale files on the persistent self-hosted runner from being uploaded.
if (Test-Path dist) { Remove-Item dist -Recurse -Force }
New-Item -ItemType Directory dist | Out-Null
$intro = 'Download `terminal-manager-' + $version + '-setup.exe` and run it for a per-user Windows x64 installation. Existing installations can update through **Settings > Updates**. This release contains the Windows installer.'
[IO.File]::WriteAllText((Join-Path $PWD 'dist/install.md'), $intro)
& "$PSScriptRoot/../release-notes.ps1" -Version $version -IntroFile dist/install.md -Verify

Invoke-Native cargo @('fmt', '--check')
Invoke-Native cargo @('clippy', '--locked', '--', '-D', 'warnings')
Invoke-Native cargo @('test', '--locked', '--all', '--no-fail-fast', '--', '--test-threads=1')
Invoke-Native cargo @('build', '--locked', '--release', '-p', 'terminal-manager', '--bin', 'terminal-manager', '-p', 'unshit-ptyd', '--bin', 'unshit-ptyd')

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { $iscc = $command.Source }
}
if (-not $iscc) { throw 'Install Inno Setup 6 on the Beelink runner before releasing' }
Invoke-Native $iscc @("/DMyAppVersion=$version", 'packaging/terminal-manager.iss')
$installer = "dist/terminal-manager-$version-setup.exe"
if (-not (Test-Path $installer)) { throw "Missing installer: $installer" }
$hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $PWD 'dist/SHA256SUMS.txt'), "$hash  terminal-manager-$version-setup.exe`n")
