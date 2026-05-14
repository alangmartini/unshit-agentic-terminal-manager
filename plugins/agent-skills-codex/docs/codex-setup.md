# Codex Setup

This repo is a Codex-only skill bundle.

## Manual Install

Codex loads user skills from `$CODEX_HOME/skills`. If `CODEX_HOME` is not set, the default is `~/.codex/skills`.

Windows PowerShell:

```powershell
$dest = if ($env:CODEX_HOME) { Join-Path $env:CODEX_HOME "skills" } else { Join-Path $env:USERPROFILE ".codex\skills" }
New-Item -ItemType Directory -Force $dest
Get-ChildItem .\skills -Directory | Copy-Item -Destination $dest -Recurse -Force
```

macOS/Linux:

```bash
dest="${CODEX_HOME:-$HOME/.codex}/skills"
mkdir -p "$dest"
cp -R skills/* "$dest/"
```

Restart Codex after installing or updating skills.

## Installing One Skill

Windows PowerShell:

```powershell
$dest = if ($env:CODEX_HOME) { Join-Path $env:CODEX_HOME "skills" } else { Join-Path $env:USERPROFILE ".codex\skills" }
New-Item -ItemType Directory -Force $dest
Copy-Item -Recurse .\skills\code-review-and-quality $dest
```

macOS/Linux:

```bash
dest="${CODEX_HOME:-$HOME/.codex}/skills"
mkdir -p "$dest"
cp -R skills/code-review-and-quality "$dest/"
```

## Plugin Layout

The plugin manifest lives at:

```text
.codex-plugin/plugin.json
```

It declares:

```json
{
  "skills": "./skills/"
}
```

Use the plugin manifest in Codex environments that support plugin installation. For direct local use, copying the skill folders is enough.

## Verify the Bundle

From the repository root:

```bash
python scripts/validate-skills.py
```

The validator checks skill frontmatter, the Codex manifest, and accidental restoration of non-Codex artifacts.
