#!/usr/bin/env python3
"""Validate the Codex skill bundle."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


REMOVED_ARTIFACTS = [
    ".claude",
    ".claude-plugin",
    ".gemini",
    ".opencode",
    "CLAUDE.md",
    "hooks",
    "agents",
    "docs/copilot-setup.md",
    "docs/cursor-setup.md",
    "docs/gemini-cli-setup.md",
    "docs/opencode-setup.md",
    "docs/windsurf-setup.md",
    "references/orchestration-patterns.md",
]


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def parse_frontmatter(path: Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8")
    match = re.match(r"^---\r?\n(.*?)\r?\n---\r?\n", text, re.DOTALL)
    if not match:
        fail(f"{path.relative_to(ROOT)} is missing YAML frontmatter")

    data: dict[str, str] = {}
    for line in match.group(1).splitlines():
        if not line.strip() or line.startswith(" "):
            continue
        key, sep, value = line.partition(":")
        if sep:
            data[key.strip()] = value.strip().strip('"').strip("'")
    return data


def validate_plugin() -> None:
    manifest_path = ROOT / ".codex-plugin" / "plugin.json"
    if not manifest_path.is_file():
        fail(".codex-plugin/plugin.json is missing")

    with manifest_path.open(encoding="utf-8") as handle:
        manifest = json.load(handle)

    if manifest.get("name") != "agent-skills-codex":
        fail(".codex-plugin/plugin.json name must be agent-skills-codex")
    if manifest.get("skills") != "./skills/":
        fail(".codex-plugin/plugin.json must point skills at ./skills/")


def validate_removed_artifacts() -> None:
    for relative in REMOVED_ARTIFACTS:
        if (ROOT / relative).exists():
            fail(f"non-Codex artifact should not exist: {relative}")


def validate_skills() -> None:
    skills_dir = ROOT / "skills"
    if not skills_dir.is_dir():
        fail("skills/ directory is missing")

    skill_dirs = sorted(path for path in skills_dir.iterdir() if path.is_dir())
    if not skill_dirs:
        fail("skills/ contains no skill directories")

    for skill_dir in skill_dirs:
        skill_path = skill_dir / "SKILL.md"
        if not skill_path.is_file():
            fail(f"{skill_dir.relative_to(ROOT)} is missing SKILL.md")

        metadata = parse_frontmatter(skill_path)
        expected_name = skill_dir.name
        actual_name = metadata.get("name")
        description = metadata.get("description")

        if actual_name != expected_name:
            fail(f"{skill_path.relative_to(ROOT)} name is {actual_name!r}, expected {expected_name!r}")
        if not description:
            fail(f"{skill_path.relative_to(ROOT)} is missing description")
        if len(description) > 1024:
            fail(f"{skill_path.relative_to(ROOT)} description is longer than 1024 characters")


def main() -> None:
    validate_plugin()
    validate_removed_artifacts()
    validate_skills()
    print("Codex skill bundle validation passed.")


if __name__ == "__main__":
    main()
