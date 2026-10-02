#Requires -Version 5.1
$ErrorActionPreference = 'Stop'
$tag = $env:RELEASE_REF
$version = $tag.Substring(1)
$headers = @{ Authorization = "Bearer $env:GH_TOKEN"; Accept = 'application/vnd.github+json' }
$existing = $null
try {
    $existing = Invoke-RestMethod "https://api.github.com/repos/$env:GITHUB_REPOSITORY/releases/tags/$tag" -Headers $headers
} catch {
    if ([int]$_.Exception.Response.StatusCode -ne 404) { throw }
}
if ($existing -and -not $existing.draft) {
    # A tag push and its merge may both enqueue this job. Never rewrite a release.
    if (-not ($existing.assets | Where-Object name -eq "terminal-manager-$version-setup.exe")) {
        throw 'Published release is missing its installer; refusing to overwrite it'
    }
    Write-Host "Release $tag already published; nothing to change."
    exit 0
}
if (-not $existing) {
    & gh release create $tag --verify-tag --draft --title $tag --notes-file "dist/release-notes-$version.md"
    if ($LASTEXITCODE -ne 0) { throw 'Could not create draft release' }
}
& gh release upload $tag "dist/terminal-manager-$version-setup.exe" dist/SHA256SUMS.txt --clobber
if ($LASTEXITCODE -ne 0) { throw 'Could not upload release assets; release remains a draft' }
& gh release edit $tag --notes-file "dist/release-notes-$version.md" --draft=false
if ($LASTEXITCODE -ne 0) { throw 'Could not publish release' }
"Published $tag with Windows installer and changelog." >> $env:GITHUB_STEP_SUMMARY
