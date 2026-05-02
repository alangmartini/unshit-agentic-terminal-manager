//! Autocomplete for the Quick Prompt overlay.
//!
//! Slice 5 wires up the Claude flavored sources only. The popup opens
//! when the user types `/` after whitespace (or at the start of the
//! buffer) and offers skills from `~/.claude/skills/` (one entry per
//! directory) and slash commands from `~/.claude/commands/` (one entry
//! per `*.md` file). Codex parity lands in Slice 6.
//!
//! The design keeps three things separable:
//!
//! * Pure data (`Entry`, `EntryKind`, `Popup`) so the dispatch arms
//!   and UI render path can match on shapes without owning IO logic.
//! * IO loaders (`load_claude_sources`, `load_claude_sources_from`)
//!   that walk the filesystem. Only the `_from(home)` variant is used
//!   in tests so we never reach into the real `$HOME`.
//! * A process global cache with a 5 second TTL so repeated opens of
//!   the overlay do not re-walk the filesystem.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One row in the popup. `name` is what the user sees and what gets
/// inserted (without the leading `/`). `kind` drives the icon / label
/// in the popup row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

/// Where the entry was sourced from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A Claude skill: a directory under `~/.claude/skills/`.
    Skill,
    /// A Claude slash command: a `*.md` file under `~/.claude/commands/`.
    Command,
}

impl EntryKind {
    /// Short label rendered next to the entry name.
    pub fn label(self) -> &'static str {
        match self {
            EntryKind::Skill => "skill",
            EntryKind::Command => "command",
        }
    }
}

/// Popup state. The popup belongs to a `QuickPromptState`; cancel paths
/// drop it without further cleanup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    /// Full source list for the active agent. The popup keeps its own
    /// copy so the cache TTL can refresh in the background without
    /// invalidating an open popup.
    pub entries: Vec<Entry>,
    /// Live query, derived from the prompt buffer between
    /// `anchor_offset + 1` and the end of the buffer (or the next
    /// whitespace).
    pub query: String,
    /// Filtered indices into `entries` for the current `query`. Kept
    /// alongside `entries` so the UI does not recompute on every
    /// render.
    pub matches: Vec<usize>,
    /// Selected position inside `matches`. Always `< matches.len()`
    /// when `matches` is non-empty; clamped to 0 when empty.
    pub selected: usize,
    /// Byte offset of the trigger character (`/`) in the prompt
    /// buffer. The query is the slice immediately after it.
    pub anchor_offset: usize,
}

impl Popup {
    /// Open a fresh popup at the given anchor with all entries visible.
    pub fn open(entries: Vec<Entry>, anchor_offset: usize) -> Self {
        let matches = (0..entries.len()).collect();
        Self {
            entries,
            query: String::new(),
            matches,
            selected: 0,
            anchor_offset,
        }
    }

    /// Recompute `matches` from `query` and clamp `selected`.
    pub fn refilter(&mut self) {
        self.matches = filter(&self.entries, &self.query);
        if self.selected >= self.matches.len() {
            self.selected = self.matches.len().saturating_sub(1);
        }
    }

    /// Move selection one row down, wrapping to the top.
    pub fn select_next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.matches.len();
    }

    /// Move selection one row up, wrapping to the bottom.
    pub fn select_prev(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.matches.len() - 1;
        } else {
            self.selected -= 1;
        }
    }

    /// The currently highlighted entry, if any.
    pub fn current(&self) -> Option<&Entry> {
        self.matches
            .get(self.selected)
            .and_then(|&idx| self.entries.get(idx))
    }
}

/// Case insensitive substring filter. Returns indices into `entries`,
/// preserving source order so the popup is stable across keystrokes.
/// An empty query keeps every row.
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let needle = query.to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if e.name.to_ascii_lowercase().contains(&needle) {
                Some(i)
            } else {
                None
            }
        })
        .collect()
}

/// Trigger detection. Returns `Some(anchor_offset)` (byte index of the
/// `/`) when the user just typed a `/` in a position that should open
/// the popup: at the very start of the buffer, or right after an ASCII
/// whitespace char.
///
/// `prev_prompt` is the previous buffer state and `next_prompt` is the
/// new buffer state. We only fire on additions of a single trigger char
/// at the end of the buffer; mid buffer insertions and replacements do
/// not open the popup. The simple "ends with /" heuristic handles the
/// common typing flow without needing real cursor tracking from the
/// framework input.
pub fn detect_claude_trigger(prev_prompt: &str, next_prompt: &str) -> Option<usize> {
    if next_prompt.len() <= prev_prompt.len() {
        return None;
    }
    if !next_prompt.ends_with('/') {
        return None;
    }
    if prev_prompt.ends_with('/') {
        // The user did not just type the trigger; they were already
        // sitting on one (e.g. backspaced something else).
        return None;
    }
    // The byte just before the trailing '/' must be whitespace or
    // missing (start of buffer).
    let trigger_pos = next_prompt.len() - 1;
    if trigger_pos == 0 {
        return Some(0);
    }
    let prefix = &next_prompt[..trigger_pos];
    match prefix.chars().next_back() {
        Some(c) if c.is_whitespace() => Some(trigger_pos),
        _ => None,
    }
}

/// Recompute `query` (and matches) from the current prompt buffer for
/// an open popup. Returns `false` when the popup should be dismissed
/// because the trigger char was deleted, the cursor moved before it,
/// or whitespace appeared inside the query window.
pub fn rederive_query(popup: &mut Popup, prompt: &str) -> bool {
    if popup.anchor_offset >= prompt.len() {
        return false;
    }
    let bytes = prompt.as_bytes();
    if bytes[popup.anchor_offset] != b'/' {
        return false;
    }
    let after = &prompt[popup.anchor_offset + 1..];
    if after.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    popup.query = after.to_string();
    popup.refilter();
    true
}

/// Insert `/<entry_name>` at `anchor_offset` (replacing whatever the
/// user had typed as the query so far). Returns the new prompt buffer.
pub fn confirm_into_prompt(prompt: &str, anchor_offset: usize, entry_name: &str) -> String {
    if anchor_offset > prompt.len() {
        return prompt.to_string();
    }
    let head = &prompt[..anchor_offset];
    // Drop everything from `/` to the next whitespace (or end). That is
    // the literal the user was building when the popup was open.
    let after_slash = &prompt[anchor_offset..];
    let tail_start_in_after = after_slash
        .char_indices()
        .skip(1)
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(after_slash.len());
    let tail = &after_slash[tail_start_in_after..];
    format!("{head}/{entry_name}{tail}")
}

// ---------------------------------------------------------------------------
// Source loaders
// ---------------------------------------------------------------------------

/// Load every Claude skill + command entry. Resolves `$HOME` via
/// `dirs::home_dir`. A missing `~/.claude` directory yields an empty
/// list with no error (per spec OQ4).
pub fn load_claude_sources() -> Vec<Entry> {
    match dirs::home_dir() {
        Some(home) => load_claude_sources_from(&home),
        None => Vec::new(),
    }
}

/// Testable variant: load skills + commands rooted at the given home
/// directory. Tests construct a temp dir mimicking `~/.claude/...` and
/// pass it here.
pub fn load_claude_sources_from(home: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    out.extend(load_skill_dirs(&home.join(".claude").join("skills")));
    out.extend(load_command_files(&home.join(".claude").join("commands")));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn load_skill_dirs(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Exclude dot prefixed dirs (e.g. `.system`); not
            // strictly needed for Claude but cheap parity with Codex
            // and keeps the popup tidy.
            if name.starts_with('.') {
                continue;
            }
            out.push(Entry {
                name: name.to_string(),
                kind: EntryKind::Skill,
            });
        }
    }
    out
}

fn load_command_files(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|n| n.to_str()) {
            if stem.starts_with('.') {
                continue;
            }
            out.push(Entry {
                name: stem.to_string(),
                kind: EntryKind::Command,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

const CACHE_TTL: Duration = Duration::from_secs(5);

static CLAUDE_CACHE: Mutex<Option<(Instant, Vec<Entry>)>> = Mutex::new(None);

/// Return the cached Claude source list when the most recent load is
/// fresh; otherwise reload from disk and update the cache. The 5s TTL
/// is the same window the spec calls out (A8.2).
pub fn cached_claude_sources() -> Vec<Entry> {
    let mut guard = CLAUDE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, entries)) = guard.as_ref() {
        if at.elapsed() < CACHE_TTL {
            return entries.clone();
        }
    }
    let fresh = load_claude_sources();
    *guard = Some((Instant::now(), fresh.clone()));
    fresh
}

/// Test only: clear the cache so a follow up call re-walks the
/// filesystem. Production code never needs this; the TTL handles
/// invalidation.
#[cfg(test)]
pub fn reset_cache_for_tests() {
    if let Ok(mut guard) = CLAUDE_CACHE.lock() {
        *guard = None;
    }
}

// Keep a path alias around for callers that want to inspect the
// resolved Claude root (e.g. for diagnostics in future slices).
#[allow(dead_code)]
pub(crate) fn claude_root_for(home: &Path) -> PathBuf {
    home.join(".claude")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_home(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("godly-qp-ac-{}-{}-{}", tag, pid, n))
    }

    fn write_at(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write");
    }

    fn make_dir(path: &Path) {
        std::fs::create_dir_all(path).expect("create dir");
    }

    fn entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            name: name.to_string(),
            kind,
        }
    }

    // --- filter ---------------------------------------------------------

    #[test]
    fn filter_empty_query_returns_all_indices() {
        let entries = vec![
            entry("apple", EntryKind::Skill),
            entry("banana", EntryKind::Command),
            entry("cherry", EntryKind::Skill),
        ];
        let got = filter(&entries, "");
        assert_eq!(got, vec![0, 1, 2]);
    }

    #[test]
    fn filter_case_insensitive_substring() {
        let entries = vec![
            entry("AppleSauce", EntryKind::Skill),
            entry("banana", EntryKind::Command),
            entry("CrabApple", EntryKind::Skill),
        ];
        let got = filter(&entries, "APPLE");
        assert_eq!(got, vec![0, 2]);
    }

    #[test]
    fn filter_misses_yields_empty() {
        let entries = vec![entry("foo", EntryKind::Skill)];
        let got = filter(&entries, "bar");
        assert!(got.is_empty());
    }

    #[test]
    fn filter_preserves_source_order() {
        let entries = vec![
            entry("zeta", EntryKind::Skill),
            entry("alpha", EntryKind::Skill),
        ];
        let got = filter(&entries, "a");
        assert_eq!(got, vec![0, 1]);
    }

    // --- trigger detection ---------------------------------------------

    #[test]
    fn detect_trigger_at_start_of_buffer() {
        assert_eq!(detect_claude_trigger("", "/"), Some(0));
    }

    #[test]
    fn detect_trigger_after_space() {
        assert_eq!(detect_claude_trigger("hello ", "hello /"), Some(6));
    }

    #[test]
    fn detect_trigger_after_newline() {
        assert_eq!(detect_claude_trigger("hi\n", "hi\n/"), Some(3));
    }

    #[test]
    fn detect_trigger_no_fire_inside_word() {
        assert_eq!(detect_claude_trigger("path", "path/"), None);
    }

    #[test]
    fn detect_trigger_no_fire_when_buffer_did_not_grow() {
        // User pasted same length text; not a single char insert.
        assert_eq!(detect_claude_trigger("abc/", "xyz/"), None);
    }

    #[test]
    fn detect_trigger_no_fire_when_already_on_trigger() {
        // Previous buffer already ended with `/`; another typed char
        // earlier in the buffer should not retrigger.
        assert_eq!(detect_claude_trigger("a /", "a /"), None);
    }

    // --- popup state machine -------------------------------------------

    #[test]
    fn popup_open_starts_with_all_matches_and_zero_selected() {
        let entries = vec![entry("a", EntryKind::Skill), entry("b", EntryKind::Command)];
        let p = Popup::open(entries.clone(), 0);
        assert_eq!(p.entries, entries);
        assert_eq!(p.matches, vec![0, 1]);
        assert_eq!(p.selected, 0);
        assert!(p.query.is_empty());
        assert_eq!(p.anchor_offset, 0);
    }

    #[test]
    fn popup_select_next_wraps() {
        let entries = vec![entry("a", EntryKind::Skill), entry("b", EntryKind::Skill)];
        let mut p = Popup::open(entries, 0);
        p.select_next();
        assert_eq!(p.selected, 1);
        p.select_next();
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn popup_select_prev_wraps() {
        let entries = vec![entry("a", EntryKind::Skill), entry("b", EntryKind::Skill)];
        let mut p = Popup::open(entries, 0);
        p.select_prev();
        assert_eq!(p.selected, 1);
        p.select_prev();
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn popup_select_no_op_when_empty_matches() {
        let mut p = Popup::open(Vec::new(), 0);
        p.select_next();
        p.select_prev();
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn popup_refilter_clamps_selected_when_matches_shrink() {
        let entries = vec![
            entry("apple", EntryKind::Skill),
            entry("banana", EntryKind::Skill),
            entry("cherry", EntryKind::Skill),
        ];
        let mut p = Popup::open(entries, 0);
        p.selected = 2;
        p.query = "apple".into();
        p.refilter();
        assert_eq!(p.matches, vec![0]);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn popup_refilter_keeps_selected_when_in_range() {
        let entries = vec![
            entry("apple", EntryKind::Skill),
            entry("apricot", EntryKind::Skill),
            entry("banana", EntryKind::Skill),
        ];
        let mut p = Popup::open(entries, 0);
        p.selected = 1;
        p.query = "ap".into();
        p.refilter();
        assert_eq!(p.matches, vec![0, 1]);
        assert_eq!(p.selected, 1);
    }

    #[test]
    fn popup_current_returns_selected_entry() {
        let entries = vec![
            entry("first", EntryKind::Skill),
            entry("second", EntryKind::Command),
        ];
        let mut p = Popup::open(entries, 0);
        p.select_next();
        assert_eq!(p.current().map(|e| e.name.as_str()), Some("second"));
    }

    // --- rederive_query ------------------------------------------------

    #[test]
    fn rederive_query_picks_up_typed_chars_after_slash() {
        let entries = vec![entry("plan", EntryKind::Command)];
        let mut p = Popup::open(entries, 6);
        let prompt = "hello /pla";
        assert!(rederive_query(&mut p, prompt));
        assert_eq!(p.query, "pla");
        assert_eq!(p.matches, vec![0]);
    }

    #[test]
    fn rederive_query_dismisses_when_anchor_lost() {
        let entries = vec![entry("plan", EntryKind::Command)];
        let mut p = Popup::open(entries, 6);
        // User backspaced past the slash.
        let prompt = "hello";
        assert!(!rederive_query(&mut p, prompt));
    }

    #[test]
    fn rederive_query_dismisses_when_whitespace_inside_query_window() {
        let entries = vec![entry("plan", EntryKind::Command)];
        let mut p = Popup::open(entries, 6);
        let prompt = "hello /pla now";
        assert!(!rederive_query(&mut p, prompt));
    }

    #[test]
    fn rederive_query_dismisses_when_anchor_no_longer_a_slash() {
        let entries = vec![entry("plan", EntryKind::Command)];
        let mut p = Popup::open(entries, 6);
        // Anchor offset 6 is now an `x`.
        let prompt = "hello x";
        assert!(!rederive_query(&mut p, prompt));
    }

    // --- confirm_into_prompt -------------------------------------------

    #[test]
    fn confirm_replaces_query_with_full_token() {
        let out = confirm_into_prompt("hello /pl", 6, "plan");
        assert_eq!(out, "hello /plan");
    }

    #[test]
    fn confirm_preserves_trailing_text_after_first_whitespace() {
        // Anchor at byte 0; the first whitespace ends the token. The
        // trailing portion (including the leading space) is preserved.
        let out = confirm_into_prompt("/pl rest", 0, "plan");
        assert_eq!(out, "/plan rest");
    }

    #[test]
    fn confirm_at_start_of_buffer() {
        let out = confirm_into_prompt("/p", 0, "plan");
        assert_eq!(out, "/plan");
    }

    #[test]
    fn confirm_returns_input_unchanged_when_anchor_out_of_bounds() {
        let out = confirm_into_prompt("/p", 99, "plan");
        assert_eq!(out, "/p");
    }

    // --- source loaders -------------------------------------------------

    #[test]
    fn load_claude_sources_from_walks_skills_and_commands() {
        let home = unique_temp_home("walk");
        // Create a synthetic ~/.claude tree.
        make_dir(&home.join(".claude").join("skills").join("git-flow"));
        make_dir(&home.join(".claude").join("skills").join("review"));
        write_at(
            &home.join(".claude").join("commands").join("plan.md"),
            "# plan",
        );
        write_at(
            &home.join(".claude").join("commands").join("commit.md"),
            "# commit",
        );

        let entries = load_claude_sources_from(&home);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        // Sort is stable on name; we expect alphabetical merge.
        assert_eq!(names, vec!["commit", "git-flow", "plan", "review"]);

        let kinds: Vec<_> = entries.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EntryKind::Command,
                EntryKind::Skill,
                EntryKind::Command,
                EntryKind::Skill,
            ]
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn load_claude_sources_from_missing_root_yields_empty() {
        let home = unique_temp_home("missing");
        // Do NOT create any directories; the load path should swallow
        // the IO error and return an empty list (per spec OQ4).
        let entries = load_claude_sources_from(&home);
        assert!(entries.is_empty());
    }

    #[test]
    fn load_claude_sources_from_skips_non_md_command_files() {
        let home = unique_temp_home("non-md");
        write_at(
            &home.join(".claude").join("commands").join("plan.md"),
            "# plan",
        );
        write_at(
            &home.join(".claude").join("commands").join("notes.txt"),
            "ignored",
        );
        let entries = load_claude_sources_from(&home);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "plan");
        assert_eq!(entries[0].kind, EntryKind::Command);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn load_claude_sources_from_skips_dot_dirs_in_skills() {
        let home = unique_temp_home("dot-dir");
        make_dir(&home.join(".claude").join("skills").join(".system"));
        make_dir(&home.join(".claude").join("skills").join("real"));
        let entries = load_claude_sources_from(&home);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["real"]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn entry_kind_label_is_human_readable() {
        assert_eq!(EntryKind::Skill.label(), "skill");
        assert_eq!(EntryKind::Command.label(), "command");
    }
}
