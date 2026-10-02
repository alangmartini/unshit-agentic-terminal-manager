"""Select stable release tags newly integrated into the default branch."""
import json
import os
import re
import subprocess
from pathlib import Path

TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z")


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def on_default(ref, branch):
    return subprocess.run(
        ["git", "merge-base", "--is-ancestor", ref, f"origin/{branch}"],
        check=False,
    ).returncode == 0


def select(event_name, event):
    branch = event["repository"]["default_branch"]
    if event_name == "workflow_dispatch":
        ref = event["inputs"]["ref"]
        dry_run = str(event["inputs"].get("dry_run", "true")).lower() == "true"
        if not TAG.fullmatch(ref) and not (dry_run and ref == branch):
            raise ValueError("Use vX.Y.Z, or the default branch for a dry run")
        revision = f"refs/tags/{ref}" if TAG.fullmatch(ref) else f"origin/{branch}"
        git("rev-parse", "--verify", f"{revision}^{{commit}}")
        if not on_default(revision, branch):
            raise ValueError("Release commit must belong to the default branch")
        return [{"ref": ref, "dry_run": dry_run}]
    if event.get("deleted"):
        return []
    ref = event["ref"]
    if ref.startswith("refs/tags/"):
        tag = ref.removeprefix("refs/tags/")
        # A tag pushed before its merge is picked up by the later branch push.
        return [{"ref": tag, "dry_run": False}] if TAG.fullmatch(tag) and on_default(ref, branch) else []
    if ref != f"refs/heads/{branch}":
        return []
    before = event["before"]
    after = event["after"]
    revision_range = after if set(before) == {"0"} else f"{before}..{after}"
    integrated = set(git("rev-list", revision_range).splitlines())
    return [
        {"ref": tag, "dry_run": False}
        for tag in git("tag", "--list", "v*").splitlines()
        if TAG.fullmatch(tag) and git("rev-parse", f"refs/tags/{tag}^{{commit}}") in integrated
    ]


if __name__ == "__main__":
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    refs = select(os.environ["GITHUB_EVENT_NAME"], event)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write("matrix=" + json.dumps({"include": refs}) + "\n")
        output.write(f"has_releases={str(bool(refs)).lower()}\n")
    print(json.dumps(refs))
