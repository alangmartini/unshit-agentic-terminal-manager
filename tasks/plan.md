# Plan: Quick Prompt overlay

Derived from `specs/quick-prompt.md`. Each task is a vertical slice that leaves the system in a working state. Tasks are sequential per the project's "merge agents one at a time" rule.

## Architecture decisions

* **One new app module `src/quick_prompt/`** owns state, UI, autocomplete, images, and spawn glue. It is self contained; the only outward touchpoints are `KeybindAction::QuickPromptOpen`, an `AppState.quick_prompt: Option<QuickPromptState>` field, dispatch arms, and a render hook in `main.rs`. This keeps the blast radius small.
* **Framework first for clipboard images.** `ClipboardContent::Image { width, height, bytes }`, `ClipboardFormat::Image`, and `ClipboardContext::read_image()` land in `crates/unshit-framework/crates/unshit-app/src/clipboard.rs` before any app code consumes them. Reuses the existing process wide arboard mutex (Windows heap corruption guard).
* **No protocol change.** `Request::SpawnSession` already carries `cwd`, `shell`, `shell_args`, and `name`. Quick Prompt sets `shell = "claude"` (or `"codex"`) with `shell_args` carrying the command tail. The daemon executes that as if it were a user shell.
* **Worktree creation shells out git.** Reuses the pattern in `src/git.rs`: `Command::new("git").args(["worktree", "add", <path>, "HEAD"]).current_dir(repo_root)`. No `git2` dependency. Empty repo fallback creates the same path as a plain directory and skips git.
* **Submit is fire and forget.** Spawn errors surface as an inline error chip on the overlay (overlay stays open); successful spawn closes the overlay.
* **Persistence is a new file.** `~/.config/com.godly.terminal/quick_prompt.json` mirrors the `OnceLock<PathBuf>` install pattern in `persist.rs`. Not bundled into `workspaces.json` so it stays decoupled.
* **Autocomplete sources are filesystem driven.** Cached in a `Mutex<Option<Vec<Entry>>>` per source root, refreshed lazily when the overlay opens after >5s of being closed. Filter is case insensitive substring; no fuzzy matcher dependency unless A8.6 fails the bench.
* **Image references inline as `@.quick-prompt/<hash>.png`.** Claude tolerates this convention; Codex does too in practice (verify in Slice 6). Per spec OQ4 we treat missing source dirs as empty, not an error.

## Dependency graph

```
Slice 0: framework clipboard image API
   |
   |   (independent of agent autocomplete + spawn paths;
   |    Slice 4 is the consumer)
   |
   v
Slice 1: KeybindAction + dispatch open/close + bare overlay
   |
   v
Slice 2: prompt input + agent toggle + persistence
   |
   +-> Slice 3: submit happy path (worktree + Claude spawn, no images, no autocomplete)
   |       |
   |       +-> Slice 4: image paste (consumes Slice 0)
   |       |       |
   |       |       +-> (image references inlined into prompt at submit)
   |       |
   |       +-> Slice 5: autocomplete (Claude only)
   |               |
   |               +-> Slice 6: Codex parity for spawn + autocomplete
   |                       |
   |                       +-> Slice 7: polish, perf bench, error chip, daemon name
```

Slices 4 and 5 are independent of each other once Slice 3 lands; they could parallelize across two agents, but per project rule we merge sequentially.

## Phase 1: Foundation

### Slice 0: Framework clipboard image API

**Description:** Add image support to the framework's `ClipboardContent`, `ClipboardFormat`, and `ClipboardContext` so the app can read pasted screenshots without each consumer reaching into `arboard` directly.

**Acceptance criteria:**
* `ClipboardContent::Image { width: usize, height: usize, bytes: Vec<u8> }` variant exists.
* `ClipboardFormat::Image` variant exists.
* `ClipboardContext::read_image() -> Result<Option<ClipboardContent>, ClipboardError>` returns `Some(Image { .. })` when the system clipboard holds an image, `Ok(None)` when it does not, and `Err(..)` only if the clipboard handle itself is unavailable.
* `available_formats()` returns a `Vec<ClipboardFormat>` that includes `Image` when one is present.
* Honors the existing process wide arboard mutex (no separate handle on Windows).

**Verification:**
* `cargo test -p unshit-app clipboard::` green, including a new `read_image_round_trips` test that writes synthetic `arboard::ImageData` then reads it back.
* `cargo test -p unshit-app` whole suite green.
* `cargo clippy --all-targets` clean.
* `cargo fmt --check` clean.

**Dependencies:** None.

**Files likely touched:**
* `crates/unshit-framework/crates/unshit-app/src/clipboard.rs`
* `crates/unshit-framework/crates/unshit-app/Cargo.toml` (only if a feature flag needs adjustment; expected: no change).

**Estimated scope:** S (1 file + tests).

### Slice 1: Empty overlay (open + close)

**Description:** Add the keybind, the empty `QuickPromptState`, dispatch arms, and a bare modal overlay so Ctrl+Shift+Q opens an empty card and Esc / backdrop / hotkey closes it. No prompt input yet.

**Acceptance criteria:**
* `KeybindAction::QuickPromptOpen` exists with id `quick_prompt_open`, default combo `Ctrl+Shift+Q`, label `Quick prompt`, dispatch command `quick_prompt.open`.
* `AppState.quick_prompt: Option<QuickPromptState>` exists. The `QuickPromptState` struct is just `{}` (or an empty marker) at this slice; richer fields land in Slice 2.
* Dispatch arms `quick_prompt.open` (sets `Some(default)`) and `quick_prompt.close` (sets `None`) work.
* `modal.close` arm clears `quick_prompt` alongside its existing fields.
* `main.rs` renders an empty card with class `quick-prompt-overlay` when `snap.quick_prompt.is_some()`, with backdrop click + Esc dispatching `quick_prompt.close`.
* `KeybindAction::ALL` count test updates from 17 to 18; `dispatch_commands_match_state_rs` adds an entry for `QuickPromptOpen`.

**Verification:**
* `cargo test --bin terminal-manager keybinds::` green.
* `cargo test --bin terminal-manager state::dispatch::quick_prompt_` green (3 new tests: open, close, modal close coexists).
* `cargo run` and verify visually: Ctrl+Shift+Q opens the empty card; Esc closes; backdrop click closes; hotkey toggles.

**Dependencies:** None (does not need Slice 0).

**Files likely touched:**
* `src/keybinds/mod.rs`
* `src/state.rs` (UiSnapshot mirror + dispatch arms + AppState field)
* `src/quick_prompt/mod.rs`, `src/quick_prompt/state.rs` (new module declared in `main.rs`)
* `src/quick_prompt/ui.rs` (new, returns the empty card)
* `src/main.rs` (mod declaration + render hook)
* `assets/styles.css` (`.quick-prompt-overlay` rules; copy from `.modal-overlay` shape)

**Estimated scope:** M (5 files + new module).

### Checkpoint: Foundation
- [ ] `cargo test`, `cargo clippy --all-targets`, `cargo fmt --check` clean.
- [ ] Manual verification: Ctrl+Shift+Q opens empty card; Esc + backdrop + hotkey close it; no other behavior regressed.
- [ ] Review with human before proceeding to Phase 2.

## Phase 2: Core feature

### Slice 2: Prompt input + agent toggle + persistence

**Description:** Wire the prompt input field, the Claude / Codex chips, the Tab toggle, and persistence of the agent choice. No submit yet, no images, no autocomplete.

**Acceptance criteria:**
* `QuickPromptState` gains `prompt: String`, `agent: Agent` (`enum Agent { Claude, Codex }`), and an `error: Option<String>` field for the future inline error chip.
* Dispatch arms: `quick_prompt.input <text>` (replaces buffer), `quick_prompt.toggle_agent`.
* Tab key inside the input dispatches `quick_prompt.toggle_agent` (does not move focus).
* On overlay open, `Agent` is loaded from `~/.config/com.godly.terminal/quick_prompt.json`; on toggle, the new value is written back.
* Missing or unparseable JSON falls back to `Agent::Claude` without panic.
* `quick_prompt::state::QuickPromptStore` provides `install(path)`, `load() -> PersistedAgent`, `save(state: &QuickPromptState)` mirroring `persist.rs`.

**Verification:**
* `cargo test --bin terminal-manager quick_prompt::state::` green (5 new tests: round trip, missing file, malformed JSON, toggle, prompt input replaces buffer).
* `cargo run`: Ctrl+Shift+Q, type, Tab toggles agent visibly, restart app, reopen overlay, agent retained.

**Dependencies:** Slice 1.

**Files likely touched:**
* `src/quick_prompt/state.rs`
* `src/quick_prompt/ui.rs` (input + chips)
* `src/state.rs` (dispatch arms, snapshot mirror)
* `src/main.rs` (install QuickPromptStore path on startup, mirroring `persist::install`)
* `assets/styles.css` (input + chip rules)

**Estimated scope:** M (5 files).

### Slice 3: Submit happy path (Claude, no images, no autocomplete)

**Description:** Pressing Ctrl+Enter creates a worktree, spawns a tab running `claude <prompt>` in it, and closes the overlay. Empty repo fallback creates a plain directory at the same path. No image attachments yet, no autocomplete.

**Acceptance criteria:**
* New `quick_prompt::spawn` module exposes:
  * `prepare_target(workspace_cwd: Option<&Path>) -> io::Result<TargetDir>` where `TargetDir { path: PathBuf, kind: TargetKind }` and `enum TargetKind { Worktree, PlainDir }`.
  * `claude_shell_spec(prompt: &str) -> ShellSpec` returning `ShellSpec { program: "claude", args: vec![prompt.to_string()] }`.
* `prepare_target` rule:
  * If `workspace_cwd` is `Some(path)` and `path` is inside a git repo (detected via `git rev-parse --is-inside-work-tree`), shell out `git worktree add <appdata-path> HEAD` and return `TargetKind::Worktree`.
  * Otherwise create the same path as a plain dir and return `TargetKind::PlainDir`.
* `mutate_add_quick_prompt_tab(state, prompt, agent)` creates the target dir, builds the agent ShellSpec, calls `state.pty_manager.spawn_in(id, workspace_id, cols, rows, Some(&path), Some(&shell_spec))`, registers reader, and pushes a tab named `qp: <truncate(prompt, 30)>`.
* Dispatch arm `quick_prompt.submit`:
  * Empty prompt sets `state.quick_prompt.as_mut().unwrap().error = Some("Type a prompt to continue.".into())` and returns `true` (rebuild).
  * Worktree creation failure sets `error` to the underlying message and keeps the overlay open. No partial state left behind.
  * Success calls `mutate_add_quick_prompt_tab`, sets `quick_prompt = None`, returns `true`.
* Ctrl+Enter inside the prompt input dispatches `quick_prompt.submit`.
* Slice 3 only wires Claude. Codex submits show an inline error chip "Codex coming soon" until Slice 6.

**Verification:**
* `cargo test --bin terminal-manager quick_prompt::spawn::` green (worktree creates a fresh dir, fallback works, claude_shell_spec round trip).
* `cargo test --bin terminal-manager state::dispatch::quick_prompt_submit_` green (3 tests: empty prompt errors, success closes overlay, failure keeps overlay).
* `cargo run` from inside the project (a git repo): Ctrl+Shift+Q, type "say hi", Ctrl+Enter. A new tab opens, runs `claude "say hi"`, working directory is `%APPDATA%\com.godly.terminal\worktrees\godly-qp-<8-hex>`.
* `cargo run` from a non git temp dir: same flow creates a plain directory and runs `claude` there.

**Dependencies:** Slice 2.

**Files likely touched:**
* `src/quick_prompt/spawn.rs`
* `src/quick_prompt/state.rs` (error field already added in Slice 2)
* `src/state.rs` (dispatch arm, mutate_add_quick_prompt_tab)
* `src/quick_prompt/ui.rs` (Ctrl+Enter on_submit; error chip render)
* `assets/styles.css` (error chip style)

**Estimated scope:** M (5 files).

### Checkpoint: Core feature
- [ ] All tests green; clippy + fmt clean.
- [ ] End to end: open overlay, type prompt, submit, verify a new tab is running Claude in the expected worktree path.
- [ ] Empty repo fallback verified manually.
- [ ] Review with human before proceeding to Phase 3.

## Phase 3: Polish

### Slice 4: Image paste

**Description:** Consume Slice 0's `read_image()` so Ctrl+V on the overlay attaches images as chips. On submit images are moved into `<worktree>\.quick-prompt\<hash>.png` and `@.quick-prompt/<hash>.png` references are appended to the prompt.

**Acceptance criteria:**
* `QuickPromptState` gains `images: Vec<QuickPromptImage>` where `QuickPromptImage { hash: String, temp_path: PathBuf, thumb_bytes: Vec<u8>, width: u32, height: u32 }`.
* `quick_prompt::images` module exposes `capture_clipboard_image(clipboard: &ClipboardContext) -> Result<Option<QuickPromptImage>, ClipboardError>`. Hashes the bytes (sha256), writes to `temp_dir().join("godly-qp").join(<session-hex>).join(<hash>.png)`, generates a 64x64 thumbnail using `image` crate (already a transitive dep; verify).
* Dispatch arms: `quick_prompt.image_paste`, `quick_prompt.image_remove <hash>`.
* Ctrl+V over the input dispatches `quick_prompt.image_paste`. If the clipboard has no image, no op (text paste is the input's default).
* Pasting the same image twice (same hash) does not duplicate the chip; the second paste is a no op.
* On submit: images are moved (not copied) from temp into `<target_path>\.quick-prompt\<hash>.png`. The prompt the agent receives is the user's text, then a blank line, then `Attached images:` header, then one `@.quick-prompt/<hash>.png` per line.
* On overlay close (cancel or hotkey toggle): all temp files under the session hex dir are deleted.
* Thumbnail chip renders inline below the input; hovering shows a remove "x" that dispatches `quick_prompt.image_remove`.

**Verification:**
* `cargo test --bin terminal-manager quick_prompt::images::` green (hash dedup, thumbnail size cap, cleanup on session drop, move on submit).
* `cargo test --bin terminal-manager quick_prompt::spawn::` updated for the image references appended to the prompt.
* `cargo run`: PrintScreen, open overlay, Ctrl+V, see chip; Ctrl+Enter; verify the worktree contains `.quick-prompt/<hash>.png` and the agent received the inline reference (visible in the new tab as Claude's first-line echo).
* Manual cancel test: paste image, Esc, verify temp dir is empty.

**Dependencies:** Slice 0, Slice 3.

**Files likely touched:**
* `src/quick_prompt/images.rs`
* `src/quick_prompt/state.rs`
* `src/quick_prompt/ui.rs`
* `src/quick_prompt/spawn.rs` (move + reference inlining)
* `src/state.rs` (dispatch arms)
* `assets/styles.css` (chip strip + thumbnail)

**Estimated scope:** M (6 files).

### Slice 5: Autocomplete (Claude only)

**Description:** `/` after whitespace opens a popup of skills + slash commands sourced from `~/.claude/skills/` and `~/.claude/commands/`. Up/Down navigates, Enter or Tab confirms, Esc closes the popup.

**Acceptance criteria:**
* `quick_prompt::autocomplete` module exposes:
  * `load_claude_sources() -> Vec<Entry>` (scans `~/.claude/skills/` for dirs, `~/.claude/commands/` for `*.md` files, returns the union with kind tags).
  * `filter(entries: &[Entry], query: &str) -> Vec<usize>` (case insensitive substring match returning indices).
  * `Popup { entries: Vec<Entry>, query: String, selected: usize, anchor_offset: usize }`.
* Trigger detection in dispatch handler `quick_prompt.input`: when the typed character is `/` and the previous char is whitespace or the buffer is empty, open the popup.
* Popup state machine arms: `quick_prompt.autocomplete_select_next`, `quick_prompt.autocomplete_select_prev`, `quick_prompt.autocomplete_confirm`, `quick_prompt.autocomplete_dismiss`.
* Confirming inserts the literal `/<entry-name>` at the cursor and closes the popup.
* Source list cached in a `Mutex<Option<(Instant, Vec<Entry>)>>`; cache reused if loaded <5 seconds ago, otherwise refreshed when the overlay opens.
* Missing source directory yields zero entries with no error (per spec OQ4).
* New criterion bench `benches/quick_prompt_filter.rs` measures `filter` over 200 synthetic entries; gate is <1ms p99 (per A8.6).

**Verification:**
* `cargo test --bin terminal-manager quick_prompt::autocomplete::` green (loader, filter, popup state, trigger detection, missing dir).
* `cargo bench --bench quick_prompt_filter`: median <500us, p99 <1ms over 200 entries.
* `cargo run`: open overlay, type `/`, see popup with real claude skills; arrow keys move; Tab inserts; Esc closes popup but keeps overlay.

**Dependencies:** Slice 3.

**Files likely touched:**
* `src/quick_prompt/autocomplete.rs`
* `src/quick_prompt/state.rs`
* `src/quick_prompt/ui.rs`
* `src/state.rs` (dispatch arms)
* `assets/styles.css` (popup + selection styles)
* `benches/quick_prompt_filter.rs` (new)
* `Cargo.toml` (add the bench entry)

**Estimated scope:** M (7 files).

### Slice 6: Codex parity

**Description:** Add Codex spawn (`codex exec <prompt>`), Codex `/` for prompts (`~/.codex/prompts/`), and Codex `

` for skills (`~/.codex/skills/` excluding `.system/`).

**Acceptance criteria:**
* `quick_prompt::spawn::codex_shell_spec(prompt) -> ShellSpec` returns `ShellSpec { program: "codex", args: vec!["exec".into(), prompt.into()] }`.
* `quick_prompt.submit` arm uses `agent` to dispatch to either `claude_shell_spec` or `codex_shell_spec`. Slice 3's "Codex coming soon" stub is removed.
* `quick_prompt::autocomplete::load_codex_command_sources()` and `load_codex_skill_sources()` exist; the popup uses the right source list based on `agent` and trigger character.
* Trigger char `

 (single backtick) opens the skill popup when `agent == Codex`.
* `.system/` under `~/.codex/skills/` is excluded from results.
* OQ1 verified: a manual run of `codex exec "say hi"` (no Quick Prompt involved) confirms the CLI accepts a prompt as the trailing positional argument. If the syntax differs, this slice updates `codex_shell_spec` accordingly and the spec note is corrected.

**Verification:**
* `cargo test --bin terminal-manager quick_prompt::autocomplete::codex_` green.
* `cargo test --bin terminal-manager quick_prompt::spawn::codex_` green.
* `cargo run` from a git repo: Ctrl+Shift+Q, Tab to Codex, type "say hi", submit; new tab runs `codex exec "say hi"`.
* Backtick autocomplete works; `.system/` does not appear in skill results.

**Dependencies:** Slice 5 (autocomplete plumbing), Slice 3 (spawn plumbing).

**Files likely touched:**
* `src/quick_prompt/spawn.rs`
* `src/quick_prompt/autocomplete.rs`
* `src/quick_prompt/ui.rs` (trigger char dispatch)
* `src/state.rs` (dispatch arm input handler)

**Estimated scope:** M (4 files).

### Slice 7: Polish + perf gates + daemon name

**Description:** Tab display name set on the daemon side, perf gate for the autocomplete bench wired into CI thinking, error chip styling, accessibility audit on the overlay (focus management, aria roles), and a sweep for any `RequestRebuild` opportunities to coalesce.

**Acceptance criteria:**
* `pty.rs::DaemonPty::spawn_in_named(pane_id, workspace_id, cols, rows, cwd, shell, name: Option<&str>)` added; existing `spawn_in` delegates with `None`. Quick Prompt uses the new signature so the daemon stores `qp: <prompt prefix>` as the session name.
* Error chip CSS finalized; matches the existing toast color palette.
* Overlay traps focus inside the card; Tab cycles between agent chips, input, and submit-disabled-when-empty button (visual only; the input still uses Tab to toggle agent in spec, so the chips are mouse only and the keyboard contract is unchanged).
* `cargo bench --bench quick_prompt_filter` is added to a `scripts/bench.ps1` invocation (or noted in `CLAUDE.md` for CI follow up).
* `cargo run` shows zero `RequestRebuild` storms when typing rapidly into the prompt input (visual perf check, optional perf overlay if available).

**Verification:**
* `cargo test`, `cargo clippy --all-targets`, `cargo fmt --check` clean.
* Manual: tab title shows "qp: <prompt prefix>" in the tab bar after submit.
* Manual: rapid typing into the prompt does not stutter the rest of the app.
* `cargo bench --bench quick_prompt_filter` p99 <1ms.

**Dependencies:** Slices 0 through 6.

**Files likely touched:**
* `src/pty.rs`
* `src/state.rs` (use `spawn_in_named` from Quick Prompt only)
* `src/quick_prompt/ui.rs`
* `assets/styles.css`
* Possibly `crates/unshit-ptyd/src/protocol/message.rs` if `name` plumbing needs an extension (existing `Request::SpawnSession.name` already exists; expected: no protocol change).

**Estimated scope:** S to M (4 to 5 files).

### Checkpoint: Complete
- [ ] All acceptance criteria F1 to F8 met.
- [ ] All tests green; clippy + fmt clean.
- [ ] Bench gate met.
- [ ] Manual verification of every user story U1 to U6.
- [ ] Ready for review.

## Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| arboard `read_image` fails or hangs on Windows | High (Slice 4 blocked) | Slice 0 lands first and is tested in isolation; production wide arboard mutex already in place. If `read_image` is not in arboard's stable API, fall back to `arboard::Clipboard::get_image()`. |
| `git worktree add` fails on weird repo states (locked, no commits, sparse) | Medium (submit blocked) | Surface error inline in the card; do not delete temp images on failure; spec already covers A5.7. |
| Codex CLI prompt syntax differs from `codex exec <prompt>` | Medium (Slice 6 broken) | Verify manually before Slice 6 lands (OQ1 in spec). If syntax differs, only `codex_shell_spec` changes. |
| Filesystem walk for autocomplete is too slow on large `~/.claude/skills` | Medium (typing latency) | Cache with 5s TTL; bench gate <1ms p99 (A8.6). If exceeded, switch to a `walkdir` crate scan with depth=1. |
| Quick Prompt overlay clashes with existing modal logic in `modal.close` arm | Low | Slice 1 explicitly tests that `modal.close` clears `quick_prompt` alongside settings_open and ctx_menu. |
| Image temp dir leaks when the app crashes mid session | Low | Best effort: clean up on overlay open if the dir already exists (delete + recreate). Process wide cleanup on next overlay open. |

## Open questions

* **OQ1** Does `codex exec <prompt>` accept a prompt as its trailing positional argument? Verify before Slice 6.
* **OQ2** Should the prompt draft persist across opens, not just the agent? Spec says no; confirm with user during Slice 2 review.
* **OQ4** What if `~/.claude/` or `~/.codex/` does not exist on the user's machine? Treat as zero entries, no error chip. Spec assumes this.
