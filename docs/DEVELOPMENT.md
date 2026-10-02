# Development and releases

## One shared application

`main` is the integration branch for both Windows and macOS. Start short-lived
feature and fix branches from `main`, and target pull requests at `main`.
Platform support is part of the same application, not a separate product branch.

Keep shared terminal, daemon protocol, state and UI behavior in shared code.
Use platform modules and Rust `cfg` gates for native APIs, transports, shortcuts
and packaging. General UI toolkit fixes belong in `crates/unshit-framework/`.
The framework's upstream branch remains independent of this repository's branch
names; the subtree commands in `AGENTS.md` still apply.

Before merging, pass the PR Quality Gate's Windows Rust Checks and macOS Checks.
The macOS job also builds the application bundle. For local changes, follow the
verification requirements in `AGENTS.md`; CI complements local UI checks.

## Migrating existing checkouts

After fetching the repository, switch to the shared branch:

```bash
git fetch origin
git switch main
git pull --ff-only origin main
git remote set-head origin -a
```

If `main` does not exist locally, use `git switch --track origin/main` instead.
Commit any pending changes before switching branches. Existing work branches can
merge `origin/main` and continue as normal; they do not need to be recreated.
Existing pull requests should target `main`.

`feat/rust-terminal-manager`, `feat/macos-terminal-manager` and `master` are
historical references after this transition. Do not use them for new feature
work or maintain a separate stream of platform fixes there. They are retained
so existing checkouts and links remain recoverable.

## Releases

A release tag identifies one source revision, with separate Windows and macOS
artifacts built from that revision. Use the same application version on both
platforms. The existing `v0.5.0` tag remains the original release; integrating
macOS support does not move or overwrite it.

Build a macOS bundle with `scripts/package-macos.sh`, then create the drag-to-Applications DMG with `scripts/create-macos-dmg.sh`. Use the Windows build and installer instructions in `README.md` for Windows
artifacts. Development CI artifacts are not a signed or notarized
public macOS release. Publish a new release only after its platform checks and
packaging checks pass.

Write the GitHub release body with `scripts/release-notes.ps1 -Version X.Y.Z
-Verify` and pass the generated file to `gh release create --notes-file`.
GitHub renders release bodies like issue comments, where every newline is a
hard line break, so the hard-wrapped `CHANGELOG.md` section must be unwrapped
first; the script does that and `-Verify` renders the result through GitHub's
markdown API and fails on any `<br>`.

Attach the macOS bundle from the release commit's own quality-gate run instead
of building it by hand. The `macOS Checks` job uploads a
`terminal-manager-macos` artifact holding
`terminal-manager-<version>-macos-<arch>.zip`, named from the version
`scripts/package-macos.sh` stamps into the bundle's `Info.plist`. Find the run
with `gh run list --branch main --json databaseId,headSha` (match the release
commit's SHA), download it with
`gh run download <run-id> -n terminal-manager-macos -D dist`, and pass the zip
to `gh release create` next to the Windows installer. Never re-zip the bundle
on another platform: the archive `ditto` produced keeps the executable bits
and the ad-hoc signature.

Use a maintenance branch only when an older released version actually needs
support while `main` moves forward; platform alone is not a reason for one.
