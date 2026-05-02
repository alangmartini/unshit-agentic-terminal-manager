# Spec: Quick Prompt overlay

A user invokes a global hotkey, gets a centered overlay where they can type a prompt, attach screenshot images from the clipboard, pick between Claude Code and Codex CLI, and dispatch the prompt into a brand new isolated worktree where the chosen agent runs unattended.

## Objective

Reduce the friction of "I have a quick task for an agent" from "open a terminal, navigate, decide a worktree path, paste prompt, copy image references" to "press Ctrl+Shift+Q, type, paste, submit." The agent must run in an isolated git worktree so the user's current checkout never moves.

## User stories

* **U1** As a user editing in any tab, I press Ctrl+Shift+Q and a centered overlay appears in the foreground without disturbing the active tab.
* **U2** I type a prompt; if I trigger autocomplete I get a list of skills and slash commands relevant to the agent I picked.
* **U3** I press Tab to switch between Claude and Codex; the autocomplete sources change to match.
* **U4** I press Ctrl+V or Win+Shift+S then Ctrl+V; a thumbnail chip appears for each pasted image.
* **U5** I press Ctrl+Enter; the overlay closes, a new tab opens running the chosen agent inside a fresh worktree, and the agent receives my prompt with image references baked in.
* **U6** I press Esc or click the backdrop; nothing happens to my current work, no worktree is created, pasted images are cleaned from temp storage.

## Acceptance criteria

### F1. Hotkey and overlay lifecycle
* **A1.1** Pressing Ctrl+Shift+Q from anywhere in the app opens the Quick Prompt overlay.
* **A1.2** Pressing Esc, clicking the modal backdrop, or pressing Ctrl+Shift+Q again closes it.
* **A1.3** Closing without submit discards the in flight prompt and removes any temp files holding pasted images.
* **A1.4** Opening from inside any workspace or tab keeps the active tab alive; the overlay does not change focus to a different tab.

### F2. Prompt input
* **A2.1** A multi line text input with a placeholder of "What should the agent do?" receives focus on open.
* **A2.2** Plain text editing supports the same kbd shortcuts the rest of the app uses for inputs (arrow keys, Home/End, Ctrl+A, Ctrl+Backspace word delete).
* **A2.3** Submitting an empty prompt is a no op; the input gains an inline error chip "Type a prompt to continue."

### F3. Agent picker
* **A3.1** Two chips at the top of the card show "Claude" and "Codex"; the selected one is highlighted.
* **A3.2** Pressing Tab toggles between them while the input has focus; the input keeps focus.
* **A3.3** The selection persists across overlay opens via `quick_prompt.json`.

### F4. Image paste
* **A4.1** With image data on the system clipboard (PrintScreen, Snipping Tool, image copied from a browser), pressing Ctrl+V inserts the image as a chip below the input, not as a base64 blob in the text.
* **A4.2** A thumbnail (max 64x64 logical px) renders inside the chip; the chip shows a remove "x" on hover.
* **A4.3** Multiple images stack horizontally; clicking the remove "x" deletes the image and its temp file.
* **A4.4** Each chip has a stable filename of `<sha256-prefix>.png`; pasting the same image twice deduplicates to one chip.
* **A4.5** Image bytes live in a per session temp dir (`std::env::temp_dir().join("godly-qp").join(<8-hex>)`) until submit or cancel. On cancel they are deleted. On submit they are moved into the worktree.

### F5. Submit
* **A5.1** Pressing Ctrl+Enter while the input has focus triggers submit.
* **A5.2** A new git worktree is created under `%APPDATA%\com.godly.terminal\worktrees\godly-qp-<8-hex>` if and only if the current workspace cwd is inside a git repo. Worktree base ref is whatever HEAD points at (no detached HEAD; we use `git worktree add <path> HEAD` so the worktree starts on a fresh anonymous branch).
* **A5.3** If the current workspace cwd is NOT inside a git repo, the same path is created as a plain directory and the agent is launched there with no git context (empty repo fallback).
* **A5.4** Pasted images are moved from temp into `<worktree>\.quick-prompt\<hash>.png`. The prompt the agent receives gets `@.quick-prompt/<hash>.png` references appended (one per image, on their own line under a "Attached images:" header).
* **A5.5** A new tab opens running the chosen agent with the prompt as the first argument:
  * Claude: `claude.cmd <prompt>` (Windows) / `claude <prompt>` (Unix), shell launched as the daemon's `default_shell()` with `shell_args = ["claude", <prompt>]`. (Resolved during planning: spawn `claude` directly as the shell; the daemon is fine with that since it just execs.)
  * Codex: `codex exec <prompt>` resolved the same way (`shell = "codex"`, `shell_args = ["exec", <prompt>]`). To verify in Slice 6.
* **A5.6** The new tab's display name is "qp: <first 30 chars of prompt>" so it is recognizable in the tab bar.
* **A5.7** If worktree creation fails (git not installed, repo locked, disk full), the overlay stays open and shows an inline error chip with the underlying message; no temp files are deleted.
* **A5.8** On success the overlay closes, the prompt input clears, the agent picker keeps its current value.

### F6. Cancel cleanup
* **A6.1** Closing without submit removes every temp image file under `std::env::temp_dir().join("godly-qp").join(<session-hex>)`.
* **A6.2** No worktree is created on cancel.

### F7. Persistence
* **A7.1** A new file `~/.config/com.godly.terminal/quick_prompt.json` records `{ agent: "claude" | "codex" }` plus optional future settings.
* **A7.2** Missing file or unparseable JSON falls back to default agent = Claude. No panic.

### F8. Autocomplete
* **A8.1** Trigger characters
  * Claude agent: `/` after whitespace or at start of input opens the popup with both skills and slash commands.
  * Codex agent: `/` after whitespace or at start opens commands; `

 opens skills.
* **A8.2** Sources scanned on overlay open (cached in a `OnceLock` per source root, invalidated when the overlay opens after >5s of being closed):
  * Claude skills: every directory under `~/.claude/skills/`. Display name = directory name.
  * Claude commands: every `*.md` file under `~/.claude/commands/`. Display name = filename without `.md`.
  * Codex skills: every directory under `~/.codex/skills/` excluding `.system/`.
  * Codex prompts: every `*.md` file under `~/.codex/prompts/`. Display name = filename without `.md`.
* **A8.3** Popup renders inline below the input. Up/Down moves selection; Enter or Tab confirms; Esc closes the popup without closing the overlay.
* **A8.4** Confirming inserts the literal token (`/<name>` or `

<name>`) at the cursor and closes the popup.
* **A8.5** Filter is case insensitive substring match on display name; no fuzzy yet (perf is enough; revisit only if user reports otherwise).
* **A8.6** Filter pass over the whole source list runs in <1 ms p99. Source scan runs in <5 ms p99 for ~100 entries.

## Project structure

New module: `src/quick_prompt/`

```
src/quick_prompt/
  mod.rs           // re-exports + public surface
  state.rs         // QuickPromptState, Agent enum, image entry
  ui.rs            // build_quick_prompt_overlay(snap, shared)
  autocomplete.rs  // source loaders, filter, popup state
  images.rs        // hash, thumbnail, temp dir lifecycle
  spawn.rs         // worktree creation, fallback, agent ShellSpec
```

Touched in app:
* `src/keybinds/mod.rs`: add `KeybindAction::QuickPromptOpen` (and bump variant count test to 18).
* `src/state.rs`: `AppState.quick_prompt: Option<QuickPromptState>`, dispatch arms.
* `src/main.rs`: render block.
* `src/persist.rs`: leave alone; Quick Prompt has its own file.

Touched in framework subtree:
* `crates/unshit-framework/crates/unshit-app/src/clipboard.rs`: add `ClipboardContent::Image { width, height, bytes }`, `ClipboardFormat::Image`, `read_image()`. Honor the existing process wide arboard mutex.

## Code style

* No em dashes, no double dashes, no " - " punctuation in source files (matches `CLAUDE.md` rule).
* Inline `#[cfg(test)] mod tests` blocks following the project pattern.
* No new top level dependency unless Slice 5 needs a fuzzy matcher (open question).

## Testing strategy

* Unit tests inline per module:
  * `quick_prompt::state`: agent toggle, image dedup, persistence round trip.
  * `quick_prompt::autocomplete`: source loader using a temp dir, filter case folding, popup state machine.
  * `quick_prompt::images`: hash, thumbnail size cap, cleanup on drop.
  * `quick_prompt::spawn`: worktree creation in a temp git repo, empty repo fallback.
* Framework unit tests in `clipboard.rs` for the new `Image` variant (use a synthetic `arboard::ImageData`).
* Criterion bench: `benches/quick_prompt_filter.rs` measuring autocomplete filter latency over 200 entries (gate per A8.6).
* No new `tests/` integration files; this app keeps tests inline.

## Boundaries (what is out of scope)

* Streaming the agent output back into a result panel; we just spawn a tab and walk away.
* Editing or saving recent prompts as templates.
* Drag and drop image input (Ctrl+V only for now).
* Changing the agent of an already running tab.
* Worktree pruning / cleanup of stale `godly-qp-*` dirs (separate maintenance feature).
* Touching the existing context palette (`Ctrl+K`) or settings shells.

## Open questions

* **OQ1** Codex `exec` flag confirmed? We pass `codex exec <prompt>` and assume it accepts a free form prompt as the trailing positional. Verify before Slice 6 is merged.
* **OQ2** Should the prompt draft itself persist across opens, not just the agent? Current spec says no (clean draft each open). Confirm during review.
* **OQ3** Is a worktree pruning cron needed? Out of scope for this spec; track separately.
* **OQ4** What is the right behavior when the user's home `.claude/` or `.codex/` dirs do not exist? Treat as zero entries, no error chip. Spec assumes this; confirm.

## Decisions resolved during planning

* Hotkey is Ctrl+Shift+Q (no remap UI in this scope; keybind override system already covers it).
* Submit is Ctrl+Enter.
* Agent toggle is Tab.
* Worktree path lives under `%APPDATA%\com.godly.terminal\worktrees\godly-qp-<8-hex>` so it never lands inside the user's repo.
* Empty repo fallback is a plain dir at the same path; agent runs there.
* Autocomplete trigger for Codex skills is `

` (single backtick). For Codex commands and Claude both, it is `/`.
* Persistence file is a new `quick_prompt.json` next to `workspaces.json`, not a section inside `workspaces.json`.
* Image references use Claude's `@<path>` convention since both agents tolerate it; Codex specific syntax can be added later if needed.
