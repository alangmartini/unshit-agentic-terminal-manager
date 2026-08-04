# Unshit Terminal Manager Backlog

## Renderer follow-ups (from the 2026-08 dropping-artifacts audit)

The persistent missing-glyph bug was fixed by never caching runs/rows with
failed glyphs (see `changelog.d/unreleased/glyph-dropping-artifacts.md`).
The audit surfaced adjacent hazards that are not yet fixed:

- [ ] **`emit_select_overlays` runs outside atlas recovery** — it emits after
  `build_render_batch_with_atlas_recovery`, so an atlas-full latched there is
  only consumed on the NEXT frame (spurious rebuild), and its instances are
  appended after `draw_spans` were built — the interleaved span path never
  draws instances beyond the last span. Move it inside the recovery window or
  give overlay instances their own span.
- [ ] **Grid splice trusts damage ranges over content** — `splice_inputs`
  deliberately skips the `content_sig` check; an under-reported damage range
  copies a stale cached column forward silently. Consider a debug-assert
  comparing spliced-row content hash against the stored payload.
- [ ] **`GlyphAtlasSet` generation trap** — mono and color atlases keep
  independent `generation` counters while every renderer cache stores a single
  u64. If the set is ever adopted (color emoji atlas), the caches need a
  combined generation or per-kind stamps.
- [ ] **Atlas shelf-height slack** — a freed shelf keeps its original height
  forever, so short glyphs revived onto tall shelves waste vertical space and
  accelerate exhaustion. Consider shelf splitting or best-fit selection.

## File editor follow-ups (post-MVP; from the 2026-07 feature review)

- [ ] **Wide-glyph cell mapping** — CJK/emoji glyphs render wide but occupy one
  editor cell, so cursor/selection drift on such lines. Tab stops are handled;
  extend `char_width_at` with unicode-width and `wide_continuation` cells.
- [ ] **Duplicate-open focuses the existing pane** — opening an already-open
  path today creates a second editor pane; last save silently wins. Focus the
  existing pane instead, and consider an mtime staleness check before overwrite.
- [ ] **CloseEditor dialog: store tab id, not index** — `tab: Some(idx)` can
  close the wrong tab if tab order changes while the dialog is up (consistent
  with the existing `KillWorkspace { workspace_idx }` pattern, so low risk).
- [ ] **Terminal AltGr audit** — `terminal/keys.rs` `encode_key` treats CTRL as
  winning before composed text; if AltGr chars misbehave in terminals too, the
  editor's fix (composed-text-only insert under CTRL+ALT) is the model.
- [ ] **CloseApp dialog: name unsaved files** — the dialog is now forced when a
  dirty editor exists and rows show the ● marker, but a "N files have unsaved
  changes" blurb + save action would make the data-loss stakes explicit. Also
  fix the copy: "ptyd will keep them alive" and the per-row keep checkboxes
  don't apply to editor panes, which always die with the process.
- [ ] **Save temp-file hardening** — `save()` writes `.{name}.tm-save-{pid}.tmp`
  via truncating `File::create`, which follows a pre-planted file/symlink at
  that predictable name. Use `create_new(true)` plus a random suffix component
  (retry on AlreadyExists). Extend the symlink doc comment to also cover
  rename-over dropping explicit NTFS ACLs, alternate data streams (incl.
  Mark-of-the-Web), and hard-link identity; `ReplaceFileW` preserves those if
  it ever matters. Consider sweeping stale `*.tm-save-*.tmp` siblings on open.
- [ ] **Bound editor memory** — the undo stack stores full old/new text and is
  never truncated, and Ctrl+V accepts clipboard text of any size; select-all/
  paste cycles on a big buffer accumulate without limit and can push the file
  past the 16 MiB cap the editor will then refuse to reopen. Cap undo by total
  bytes (evict oldest groups) and warn/cap on oversized paste. Also close the
  open-path TOCTOU: re-check `bytes.len()` after `fs::read` (or read via
  `take(MAX + 1)`) so a growing file can't blow past the cap.
- [ ] **Clipboard test seam** — `AppState.clipboard` is the concrete OS
  clipboard, so the Ctrl+C/X/V handler flow (cut deletes only after a
  successful write, empty-paste ignored) has zero tests. Introduce a trait or
  injectable clipboard and pin those flows; also emit structured events (not
  interpolated prose) on clipboard failures.
- [ ] **Coverage follow-ups from the ship review** — TooLarge (>16 MiB) refusal
  path incl. its telemetry (sparse file via `set_len`); tab-scope save-close
  with one failing save (whole tab must stay open after attempting all);
  deferred PTY-spawn guards in `main.rs`/`bridge.rs` (extract eligible-pane
  filter into `state.rs` to make it testable); workspace-scoped kill clearing
  editors; telemetry sink path injection so process-exit `editor.close` events
  are assertable and state tests stop appending to the developer's live
  `editor-events.jsonl`; touchpad sub-cell wheel fallback; Ctrl+Up/Down
  viewport scroll; Tab key insert; 512 KiB log rotation branch; pin current
  duplicate-open behavior as the baseline for the dedup fix above. Seed a real
  editor in `close_editor_dialog_discard_click_dispatches_discard_close` (it
  currently only proves the dialog clears).
- [ ] **Paint-only patch for edits** — every buffer-changing keystroke returns
  `RequestRebuild` (full tree rebuild); wheel scroll already has the
  `ScrollGridPatch` fast path. When title/dirty-marker didn't change, a
  grid-only patch would keep typing latency flat as tab count grows. Defer
  until profiling says otherwise (LineQuadCache replay absorbs most of it).
- [ ] **Max-line-length guard** — a valid 16 MiB single-line file makes
  cursor/paint math scan the whole line per keystroke (freeze, not crash).
  Refuse pathological line lengths on open or add a per-line lazy width index.

## Product ideas

- [ ] **Learning mode using agent skills**
  - **Source idea:** https://gist.github.com/ThariqS/1389dcdff9eba4789887a2211370f06b
  - **Goal:** Add an interactive teaching mode where an agent skill guides the user through code understanding instead of only executing commands.
  - **Core flow:**
    - Start from a task, file, commit, branch diff, or PR.
    - Ask the user to restate current understanding before explaining.
    - Open related files in Unshit as the explanation progresses.
    - Explain code paths, ownership boundaries, data flow, edge cases, and why the implementation exists.
    - Maintain a running Markdown checklist of concepts the user should understand.
    - Quiz with open-ended or multiple-choice questions before moving to the next stage.
    - Keep the session active until the user demonstrates understanding of the checklist.
  - **PR mode:**
    - Ingest a PR diff and identify changed files, entry points, and behavior changes.
    - Walk the user through problem, solution, design decisions, risks, tests, and likely impact.
    - Link each explanation step to the relevant file and code path.
    - End with a review-ready summary and remaining questions.
  - **Implementation notes:**
    - Model this as a reusable agent skill, not hard-coded teaching prompts.
    - Prefer existing terminal/session primitives; avoid blocking IPC in render paths.
    - Treat file opening and navigation as app-level behavior.
    - Store generated learning notes under a predictable workspace path.
  - **Open questions:**
    - Should learning mode run inside Quick Prompt, a dedicated command palette action, or a separate sidebar view?
    - Should mastery checks use agent-native question tooling when available, or an Unshit-native question UI?
    - How should PR mode fetch PR data: local branch diff only, GitHub CLI, or hosted provider API?
