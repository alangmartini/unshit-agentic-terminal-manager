//! Quick-open file index (spec section 6, "Quick open").
//!
//! Enumerates the files of a workspace root so the command palette's `Files`
//! mode can fuzzy-match them. Two sources, in order of preference:
//!
//! 1. `git ls-files -z --cached --others --exclude-standard` — free
//!    `.gitignore` semantics, and git already skips `.git/`. It is also an
//!    order of magnitude faster than walking a large checkout ourselves.
//! 2. A bounded manual walk, for roots that are not a checkout (or when git
//!    is not installed at all).
//!
//! Design constraints that shaped this module:
//!
//! * **Never blocks the UI thread.** Building spawns a process and touches the
//!   filesystem, so the caller runs it on a background thread. [`FileIndex`]
//!   is plain data and therefore `Send`.
//! * **Never panics.** A directory that cannot be read is skipped; a missing
//!   git is a fall-through, not an error. A quick-open palette that cannot
//!   open is a much worse failure than a short list.
//! * **Bounded.** A runaway root (a home directory, a node monorepo, a
//!   symlink cycle) must not eat memory or time, hence the depth/entry caps
//!   and the "never follow a symlinked directory" rule.
//! * **Stable output.** Entries are sorted by path so identical queries give
//!   identical row order across rebuilds; a list that reshuffles under the
//!   cursor is unusable for keyboard-first selection.
//!
//! Privacy: the one telemetry record per build carries the root and a count,
//! never an individual file path and never the query text.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::editor::telemetry::{record_editor_event, EditorEventRecord};

/// Maximum directory depth the fallback walk descends, counted in path
/// components below the root (a file directly in the root is depth 1).
///
/// Deep trees are almost always generated output (build caches, virtualenvs);
/// 16 clears every hand-written source layout we have seen while keeping a
/// pathological tree bounded.
pub const MAX_INDEX_DEPTH: usize = 16;

/// Hard cap on indexed entries, for both the git and the walk path.
///
/// The palette shows at most 50 rows, so beyond this the extra entries only
/// cost memory and match time. 50 000 keeps a large monorepo usable while
/// bounding `rank` at a few milliseconds per keystroke.
pub const MAX_INDEX_ENTRIES: usize = 50_000;

/// How old an index may be before the palette rebuilds it on open.
///
/// Exposed so the caller and the tests agree on the number the spec names.
pub const DEFAULT_INDEX_MAX_AGE: Duration = Duration::from_secs(30);

/// Directory names the fallback walk never descends into.
///
/// These are dependency/output trees: huge, uninteresting to a human looking
/// for a source file, and the single biggest cost of an unbounded walk.
/// `.git` is listed explicitly even though the dot rule below would also
/// catch it, because it is the one entry whose exclusion is load-bearing
/// rather than a heuristic.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "__pycache__",
    ".next",
    "vendor",
];

/// Score added when a query matched the *basename* of an entry.
///
/// Humans quick-open by file name, not by path, so a basename hit must beat
/// a full-path hit of the same textual quality. The palette matcher awards
/// 100 per matched byte plus up to 40 for contiguity and 25 for a word
/// boundary, so the bonus has to clear a whole word-boundary bonus plus the
/// leading-offset penalty to be decisive; 60 does, without letting a weak
/// basename hit outrank a strong path hit.
const BASENAME_BONUS: i32 = 60;

/// Where the entries of a [`FileIndex`] came from.
///
/// Surfaced (rather than kept private) because the two sources have
/// different semantics: `Git` honours `.gitignore`, `Walk` only honours the
/// hardcoded skip list. Telemetry and the palette footer both want to say
/// which one produced the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexSource {
    /// `git ls-files` — tracked plus untracked-but-not-ignored files.
    Git,
    /// The bounded fallback walk.
    Walk,
}

impl IndexSource {
    /// Stable machine-readable name, used as the telemetry `reason`.
    fn as_reason(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Walk => "walk",
        }
    }
}

/// One indexable file.
///
/// `rel` is the path relative to the index root using **forward slashes** on
/// every platform, so match scoring, display and telemetry do not have to
/// care that Windows hands us backslashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Path relative to the index root, forward-slashed.
    pub rel: String,
    /// Byte offset of the basename inside `rel`.
    ///
    /// Precomputed so [`rank`] can score the basename separately without
    /// re-splitting the path on every keystroke for every entry — at 50 000
    /// entries that split would dominate the match cost.
    pub name_start: usize,
}

impl FileEntry {
    /// The basename slice of this entry.
    ///
    /// Always a valid slice: `name_start` is either 0 or one byte past a `/`,
    /// and `/` is a single-byte UTF-8 boundary.
    pub fn name(&self) -> &str {
        &self.rel[self.name_start..]
    }
}

/// A snapshot of the files under one workspace root.
///
/// Immutable once built: refreshing means building a new index and swapping
/// it in, so a background rebuild can never tear a list the UI is reading.
#[derive(Clone, Debug)]
pub struct FileIndex {
    /// The workspace root the entries are relative to.
    pub root: PathBuf,
    /// Entries sorted by `rel` ascending, deduplicated.
    pub entries: Vec<FileEntry>,
    /// Which enumeration strategy produced `entries`.
    pub source: IndexSource,
    /// True when a cap (depth or entry count) stopped enumeration, so the
    /// palette can tell the user the list is incomplete instead of letting
    /// them conclude a file does not exist.
    pub truncated: bool,
    /// When the build finished; drives [`is_stale`].
    ///
    /// `Instant` rather than a wall clock on purpose: staleness is about
    /// elapsed time, and a system clock jump must not make a fresh index look
    /// ancient (or a stale one look fresh).
    pub built_at: Instant,
}

impl FileIndex {
    /// Absolute path of entry `idx`, for dispatching `editor.open:<abs path>`.
    ///
    /// Joining a forward-slashed relative path onto a Windows root is fine —
    /// the OS accepts both separators — but going through here keeps the
    /// "rows only ever come from the index" invariant that lets the palette
    /// allowlist admit the `editor.open:` prefix.
    pub fn absolute_path(&self, idx: usize) -> Option<PathBuf> {
        self.entries
            .get(idx)
            .map(|entry| self.root.join(&entry.rel))
    }
}

/// A match produced by [`rank`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RankedFile {
    /// Index into [`FileIndex::entries`].
    pub idx: usize,
    /// Higher is better.
    pub score: i32,
}

/// True when `index` is older than `max_age` and should be rebuilt.
///
/// Separate from the struct so the caller can use a different age for a
/// user-triggered refresh than for the automatic on-open rebuild.
pub fn is_stale(index: &FileIndex, max_age: Duration) -> bool {
    index.built_at.elapsed() >= max_age
}

/// Build the quick-open index for `root` with the production caps.
///
/// Runs git first and falls back to a bounded walk. Emits exactly one
/// `quickopen.index` telemetry record per call.
pub fn build_index(root: &Path) -> FileIndex {
    build_index_with_limits(root, MAX_INDEX_DEPTH, MAX_INDEX_ENTRIES)
}

/// [`build_index`] with explicit caps.
///
/// Exists so the cap behaviour can be tested against a handful of fixture
/// files instead of a 50 000-file tree; production always goes through
/// [`build_index`].
pub(crate) fn build_index_with_limits(
    root: &Path,
    max_depth: usize,
    max_entries: usize,
) -> FileIndex {
    let (mut entries, source, truncated) = match git_entries(root, max_entries) {
        Some((entries, truncated)) => (entries, IndexSource::Git, truncated),
        None => {
            let (entries, truncated) = walk_entries(root, max_depth, max_entries);
            (entries, IndexSource::Walk, truncated)
        }
    };

    // Sorting is what makes repeated builds produce the same row order. The
    // dedup afterwards matters for the git path: during an unmerged merge,
    // `ls-files --cached` prints one line per conflict stage, so the same
    // path can appear up to three times.
    entries.sort_unstable_by(|a, b| a.rel.cmp(&b.rel));
    entries.dedup_by(|a, b| a.rel == b.rel);

    let index = FileIndex {
        root: root.to_path_buf(),
        entries,
        source,
        truncated,
        built_at: Instant::now(),
    };

    record_index_event(&index);
    index
}

/// Emit the single per-build telemetry record.
///
/// Reuses the editor sink (`editor-events.jsonl`) rather than opening a new
/// one: quick open is an editor navigation feature, and a third sink would be
/// plumbing for no diagnostic gain. Privacy invariant: the root and a count
/// only — never an entry, never the query.
fn record_index_event(index: &FileIndex) {
    let root = index.root.to_string_lossy();
    let correlation_id = generate_correlation_id();
    record_editor_event(&EditorEventRecord {
        timestamp_unix_ms: crate::editor::telemetry::now_unix_ms(),
        event: "quickopen.index",
        level: "info",
        correlation_id: &correlation_id,
        path: Some(root.as_ref()),
        file_bytes: None,
        line_count: Some(index.entries.len() as u64),
        line: None,
        reason: Some(index.source.as_reason()),
        os_error: None,
    });
}

/// Process-unique id for one index build, mirroring
/// `crate::editor::generate_correlation_id` so records from the two producers
/// in the same sink are visibly the same shape.
fn generate_correlation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "qo-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        crate::editor::telemetry::now_unix_ms()
    )
}

/// Enumerate via git. `None` means "not usable here" (git missing, `root` not
/// a checkout, or git failed) and the caller must fall back to the walk.
///
/// `--cached` gives tracked files, `--others --exclude-standard` adds
/// untracked files that are not ignored — together, exactly the set a human
/// thinks of as "the files of this project". `-z` makes the output
/// NUL-separated, which sidesteps `core.quotepath` escaping and paths
/// containing newlines.
fn git_entries(root: &Path, max_entries: usize) -> Option<(Vec<FileEntry>, bool)> {
    // Must go through `crate::git::git_command`: it sets the working
    // directory and CREATE_NO_WINDOW, without which the release build (which
    // owns no console) flashes a black window per spawn.
    let output = crate::git::git_command(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let mut entries = Vec::new();
    let mut truncated = false;
    for raw in output.stdout.split(|byte| *byte == 0) {
        // The output ends with a separator, so the last split is empty.
        if raw.is_empty() {
            continue;
        }
        // Paths are bytes on disk; git hands them back verbatim. Lossy
        // conversion keeps a weirdly encoded name in the list (as a name with
        // replacement chars) rather than dropping the file entirely.
        let rel = normalize_separators(&String::from_utf8_lossy(raw));
        // Submodule gitlinks are the only directory-shaped output; a trailing
        // slash cannot appear, but guard anyway so a future git flag cannot
        // put a directory in the quick-open list.
        if rel.is_empty() || rel.ends_with('/') {
            continue;
        }
        // Check the cap only when there is a real entry to store, so a run
        // that happens to end exactly at the cap is not reported as
        // truncated. Truncating here (before the sort) means the kept set is
        // git's arbitrary order rather than the alphabetically first N; at
        // 50 000 entries the palette is already unusable as a browse list, so
        // paying a full sort of a discarded remainder is not worth it.
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
        entries.push(new_entry(rel));
    }
    Some((entries, truncated))
}

/// Bounded, iterative directory walk.
///
/// Iterative rather than recursive on purpose: `max_depth` bounds the tree we
/// *intend* to walk, but a caller passing a large depth on a pathological tree
/// would otherwise overflow the stack, and a stack overflow is an immediate
/// process abort with no telemetry.
///
/// Returns the entries and whether a cap stopped enumeration.
fn walk_entries(root: &Path, max_depth: usize, max_entries: usize) -> (Vec<FileEntry>, bool) {
    struct Pending {
        path: PathBuf,
        /// `rel` prefix for the children of `path`: either empty (root) or
        /// ending in `/`. Carried down so building a child's `rel` is a push
        /// instead of a `strip_prefix` + re-encode per file.
        prefix: String,
        /// Component count below the root; the root itself is 0.
        depth: usize,
    }

    let mut entries = Vec::new();
    let mut truncated = false;
    let mut stack = vec![Pending {
        path: root.to_path_buf(),
        prefix: String::new(),
        depth: 0,
    }];

    'walk: while let Some(dir) = stack.pop() {
        // An unreadable directory (permissions, a vanished path, a dead
        // junction) is skipped, never fatal.
        let Ok(read_dir) = std::fs::read_dir(&dir.path) else {
            continue;
        };
        for entry in read_dir.flatten() {
            // `read_dir` already carries the file type on both Windows and
            // Linux, so this is free; `symlink_metadata` is the fallback for
            // the platforms where it is not, and like `file_type` it does
            // *not* follow the link.
            let Some(file_type) = entry.file_type().ok().or_else(|| {
                std::fs::symlink_metadata(entry.path())
                    .ok()
                    .map(|m| m.file_type())
            }) else {
                continue;
            };

            let raw_name = entry.file_name();
            let name = normalize_separators(&raw_name.to_string_lossy());
            if name.is_empty() {
                continue;
            }

            if file_type.is_symlink() {
                // Never descend through a link: a self-referential junction
                // (or a symlink back to the root) would otherwise walk until
                // the depth cap, producing thousands of duplicate entries.
                // A symlink *to a file* is still worth indexing, and resolving
                // one link is cheap because links are rare.
                if std::fs::metadata(entry.path()).is_ok_and(|m| m.is_file()) {
                    if entries.len() >= max_entries {
                        truncated = true;
                        break 'walk;
                    }
                    entries.push(new_entry(format!("{}{}", dir.prefix, name)));
                }
                continue;
            }

            if file_type.is_dir() {
                if is_skipped_dir(&name) {
                    continue;
                }
                let child_depth = dir.depth + 1;
                // Children of this directory would live at `child_depth + 1`,
                // so descending is only useful while `child_depth < max_depth`.
                if child_depth < max_depth {
                    stack.push(Pending {
                        path: entry.path(),
                        prefix: format!("{}{}/", dir.prefix, name),
                        depth: child_depth,
                    });
                } else {
                    // Declining to descend is a cap stopping the walk, which
                    // is exactly what `truncated` reports: files below this
                    // point exist but are not in the list.
                    truncated = true;
                }
                continue;
            }

            if file_type.is_file() {
                // The cap is checked at the push site, not at the top of the
                // loop, so a tree whose file count lands exactly on the cap is
                // not falsely reported as truncated just because a skipped
                // directory still followed it in `read_dir` order.
                if entries.len() >= max_entries {
                    truncated = true;
                    break 'walk;
                }
                entries.push(new_entry(format!("{}{}", dir.prefix, name)));
            }
            // Anything else (FIFOs, devices, sockets) is deliberately dropped:
            // opening one in the editor would block or fail.
        }
    }

    (entries, truncated)
}

/// True for directory names the walk never enters.
///
/// Dot-directories are skipped wholesale (`.git`, `.idea`, `.cache`, …)
/// because they hold tool state, never files a human quick-opens. Dot-*files*
/// such as `.gitignore` are explicitly kept — they are edited often, and
/// losing them was the obvious failure mode of a blanket "skip dotted names"
/// rule.
fn is_skipped_dir(name: &str) -> bool {
    name.starts_with('.') || SKIPPED_DIRS.contains(&name)
}

/// Build an entry, computing the basename offset once.
fn new_entry(rel: String) -> FileEntry {
    // `rfind('/')` is byte-safe: `/` never appears inside a multi-byte UTF-8
    // sequence, so the offset is always a char boundary.
    let name_start = rel.rfind('/').map(|idx| idx + 1).unwrap_or(0);
    FileEntry { rel, name_start }
}

/// Replace backslashes with forward slashes.
///
/// Both separators mean the same thing to the Windows API, but the matcher,
/// the palette label and any future persisted form all want one spelling.
/// Byte-length preserving, so precomputed offsets survive.
fn normalize_separators(input: &str) -> String {
    if input.contains('\\') {
        input.replace('\\', "/")
    } else {
        input.to_owned()
    }
}

/// Rank index entries against `query`, best first, at most `limit` results.
///
/// Reuses the palette's [`crate::command_palette::fuzzy_match`] so quick open
/// and the command palette feel identical, then layers the one file-specific
/// rule on top: a hit in the basename beats a hit anywhere in the path.
///
/// An empty (or whitespace-only) query returns the first `limit` entries in
/// index order, which is already sorted by path — a stable "browse the repo"
/// list rather than an arbitrary one.
pub fn rank(index: &FileIndex, query: &str, limit: usize) -> Vec<RankedFile> {
    rank_counted(index, query, limit).0
}

/// [`rank`], plus how many entries matched before `limit` cut the list.
///
/// The palette shows at most `limit` rows, and without the total a user
/// whose file ranked 51st concludes it is not in the index and retypes.
pub fn rank_counted(index: &FileIndex, query: &str, limit: usize) -> (Vec<RankedFile>, usize) {
    if limit == 0 {
        return (Vec::new(), 0);
    }

    let trimmed = query.trim();
    if trimmed.is_empty() {
        // No query: every entry "matches", so the total is the index.
        return (
            index
                .entries
                .iter()
                .take(limit)
                .enumerate()
                .map(|(idx, _)| RankedFile { idx, score: 0 })
                .collect(),
            index.entries.len(),
        );
    }

    // `fuzzy_match` allocates two lowercase copies per call; at 50 000
    // entries times two passes that is the dominant cost of a keystroke. The
    // subsequence prefilter is allocation-free and rejects the vast majority.
    // It must lowercase and trim exactly the way `fuzzy_match` does, or we
    // would drop results the matcher would have accepted.
    let needle = trimmed.to_ascii_lowercase();
    let needle = needle.as_bytes();

    let mut ranked: Vec<RankedFile> = Vec::new();
    for (idx, entry) in index.entries.iter().enumerate() {
        // The basename is a suffix of `rel`, so anything that fails the
        // subsequence test on `rel` cannot match the basename either.
        if !is_ascii_ci_subsequence(needle, entry.rel.as_bytes()) {
            continue;
        }

        let path_score = crate::command_palette::fuzzy_match(trimmed, &entry.rel).map(|s| s.score);
        // For a root-level file the basename *is* the whole path, so reuse
        // the score instead of matching twice — but still award the bonus, or
        // `src/main.rs` would outrank a top-level `main.rs`.
        let base_score = if entry.name_start == 0 {
            path_score
        } else {
            crate::command_palette::fuzzy_match(trimmed, entry.name()).map(|s| s.score)
        };

        let mut best = path_score;
        if let Some(base) = base_score {
            let boosted = base.saturating_add(BASENAME_BONUS);
            best = Some(best.map_or(boosted, |path| path.max(boosted)));
        }
        let Some(score) = best else {
            continue;
        };
        ranked.push(RankedFile { idx, score });
    }

    // Score descending, then path ascending. `entries` is already sorted by
    // `rel`, so a stable sort would suffice; comparing `rel` explicitly keeps
    // the ordering correct if that invariant ever changes.
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| index.entries[a.idx].rel.cmp(&index.entries[b.idx].rel))
    });
    let matched = ranked.len();
    ranked.truncate(limit);
    (ranked, matched)
}

/// True when every byte of `needle` appears in `haystack`, in order,
/// case-insensitively for ASCII.
///
/// Deliberately ASCII-only and allocation-free: it mirrors `fuzzy_match`'s own
/// `to_ascii_lowercase` comparison, so it can only ever reject candidates the
/// matcher would also reject.
fn is_ascii_ci_subsequence(needle: &[u8], haystack: &[u8]) -> bool {
    let mut needle = needle.iter();
    let mut next = needle.next();
    for byte in haystack {
        match next {
            Some(want) if byte.to_ascii_lowercase() == *want => next = needle.next(),
            Some(_) => {}
            None => return true,
        }
    }
    next.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A fixture tree that removes itself even when an assertion panics —
    /// a leaked tree would poison the *next* run of the same test, because
    /// the walk would find files a later fixture did not create.
    struct TempTree {
        root: PathBuf,
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn unique_temp_dir(tag: &str) -> TempTree {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut root = std::env::temp_dir();
        root.push(format!(
            "terminal-manager-file-index-{tag}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        // PID reuse can make this name reachable again in a later run.
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");
        TempTree { root }
    }

    fn write_file(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&path, contents).expect("write fixture file");
    }

    /// The canonical fixture: two root-level files (one a dotfile), a nested
    /// source tree, and one of every kind of directory the walk must refuse.
    fn build_fixture(tag: &str) -> TempTree {
        let tree = unique_temp_dir(tag);
        write_file(&tree.root, ".gitignore", "target\n");
        write_file(&tree.root, "README.md", "hello\n");
        write_file(&tree.root, "src/main.rs", "fn main() {}\n");
        write_file(&tree.root, "src/ui/panel.rs", "// panel\n");
        write_file(&tree.root, ".git/config", "[core]\n");
        write_file(&tree.root, ".hidden/secret.txt", "shh\n");
        write_file(&tree.root, "node_modules/pkg/index.js", "module\n");
        write_file(&tree.root, "target/debug/app.exe", "binary\n");
        tree
    }

    fn rels(index: &FileIndex) -> Vec<&str> {
        index.entries.iter().map(|e| e.rel.as_str()).collect()
    }

    fn index_of(entries: &[&str]) -> FileIndex {
        FileIndex {
            root: PathBuf::from("C:/repo"),
            entries: entries
                .iter()
                .map(|rel| new_entry((*rel).to_owned()))
                .collect(),
            source: IndexSource::Walk,
            truncated: false,
            built_at: Instant::now(),
        }
    }

    /// Pins the whole skip policy at once: dependency/output directories and
    /// dot-*directories* are dropped, dot-*files* are kept. The dotfile half
    /// is the regression that a blanket "skip anything starting with a dot"
    /// rule would reintroduce, and it would silently hide `.gitignore`,
    /// `.env` and friends from quick open.
    #[test]
    fn file_index_walk_skips_noise_directories_but_keeps_dotfiles() {
        let tree = build_fixture("skip");
        let index = build_index_with_limits(&tree.root, MAX_INDEX_DEPTH, MAX_INDEX_ENTRIES);

        assert_eq!(index.source, IndexSource::Walk);
        assert!(!index.truncated, "fixture is far below both caps");
        assert_eq!(
            rels(&index),
            vec![".gitignore", "README.md", "src/main.rs", "src/ui/panel.rs"],
            "skipped trees must not contribute, dotfiles must"
        );
    }

    /// Sort order and separator normalisation are what make the palette rows
    /// stable and platform-independent; `std::fs::read_dir` order is not
    /// guaranteed, so without the sort the same repo could produce different
    /// row order between builds.
    #[test]
    fn file_index_entries_are_sorted_and_forward_slashed() {
        let tree = build_fixture("sorted");
        let index = build_index_with_limits(&tree.root, MAX_INDEX_DEPTH, MAX_INDEX_ENTRIES);

        let mut sorted = rels(&index);
        sorted.sort_unstable();
        assert_eq!(rels(&index), sorted, "entries must come back sorted");
        assert!(
            index.entries.iter().all(|e| !e.rel.contains('\\')),
            "Windows separators must be normalised: {:?}",
            rels(&index)
        );
    }

    /// `name_start` is precomputed for the matcher; an off-by-one here would
    /// silently score the wrong slice (a leading `/`, or half a directory
    /// name) and quietly degrade every quick-open result.
    #[test]
    fn file_index_name_start_points_at_the_basename() {
        let tree = build_fixture("namestart");
        let index = build_index_with_limits(&tree.root, MAX_INDEX_DEPTH, MAX_INDEX_ENTRIES);

        for entry in &index.entries {
            assert_eq!(
                entry.name(),
                entry.rel.rsplit('/').next().unwrap(),
                "basename slice for {}",
                entry.rel
            );
        }
        let nested = index
            .entries
            .iter()
            .find(|e| e.rel == "src/ui/panel.rs")
            .expect("nested entry present");
        assert_eq!(nested.name_start, "src/ui/".len());
        let root_level = index
            .entries
            .iter()
            .find(|e| e.rel == "README.md")
            .expect("root entry present");
        assert_eq!(root_level.name_start, 0);
    }

    /// The depth cap must prune *and* report, because a silently short list
    /// tells the user "that file does not exist" instead of "look deeper".
    #[test]
    fn file_index_depth_cap_prunes_and_marks_truncated() {
        let tree = build_fixture("depth");

        // Depth 1: only files directly in the root survive; `src/` is
        // declined, which is a cap stopping the walk.
        let shallow = build_index_with_limits(&tree.root, 1, MAX_INDEX_ENTRIES);
        assert_eq!(rels(&shallow), vec![".gitignore", "README.md"]);
        assert!(
            shallow.truncated,
            "declining to descend counts as truncated"
        );

        // Depth 2 reaches `src/main.rs` but declines `src/ui/`.
        let deeper = build_index_with_limits(&tree.root, 2, MAX_INDEX_ENTRIES);
        assert_eq!(
            rels(&deeper),
            vec![".gitignore", "README.md", "src/main.rs"]
        );
        assert!(deeper.truncated);
    }

    /// The entry cap bounds memory and match time on a runaway root. It must
    /// stop the walk immediately rather than collecting everything and
    /// trimming afterwards, which is what the count assertion pins.
    #[test]
    fn file_index_entry_cap_stops_the_walk_and_marks_truncated() {
        let tree = build_fixture("entries");
        let capped = build_index_with_limits(&tree.root, MAX_INDEX_DEPTH, 2);

        assert_eq!(capped.entries.len(), 2);
        assert!(capped.truncated);

        // A cap equal to the fixture size is not truncation.
        let exact = build_index_with_limits(&tree.root, MAX_INDEX_DEPTH, 4);
        assert_eq!(exact.entries.len(), 4);
        assert!(!exact.truncated);
    }

    /// The git path is environment-dependent and deliberately untested
    /// against a real checkout; what matters for correctness here is that a
    /// non-checkout falls through instead of producing an empty index.
    #[test]
    fn file_index_falls_back_to_walk_outside_a_checkout() {
        let tree = unique_temp_dir("nonrepo");
        write_file(&tree.root, "only.txt", "x\n");
        let index = build_index(&tree.root);

        assert_eq!(index.source, IndexSource::Walk);
        assert_eq!(rels(&index), vec!["only.txt"]);
        assert_eq!(index.root, tree.root);
    }

    /// An unreadable or missing root must produce an empty index, never a
    /// panic: quick open is opened by a keystroke, and a panic there takes
    /// the whole app down.
    #[test]
    fn file_index_missing_root_yields_empty_index_without_panicking() {
        let tree = unique_temp_dir("missing");
        let missing = tree.root.join("does-not-exist");
        let index = build_index(&missing);

        assert!(index.entries.is_empty());
        assert!(!index.truncated);
        assert_eq!(index.source, IndexSource::Walk);
    }

    /// The basename bonus is the whole point of a file matcher: typing a file
    /// name must surface that file, not a directory that happens to contain
    /// the same letters earlier in its path.
    #[test]
    fn rank_prefers_a_basename_hit_over_a_path_hit() {
        let index = index_of(&["lib/core.rs", "src/main.rs", "src/mainframe/loader.rs"]);
        let ranked = rank(&index, "main", 10);

        let matched: Vec<&str> = ranked
            .iter()
            .map(|r| index.entries[r.idx].rel.as_str())
            .collect();
        assert_eq!(
            matched,
            vec!["src/main.rs", "src/mainframe/loader.rs"],
            "basename hit first; non-matching entries excluded entirely"
        );
        assert!(ranked[0].score > ranked[1].score);
    }

    /// A nested file must not outrank a root-level file with the same name
    /// just because the root-level one has no directory prefix to bonus.
    /// Without the `name_start == 0` branch awarding the bonus, `main.rs`
    /// loses to `src/main.rs` by exactly [`BASENAME_BONUS`]; the equal-score
    /// assertion is what pins that, the order assertion only follows from it
    /// via the alphabetical tie-break.
    #[test]
    fn rank_awards_the_basename_bonus_to_root_level_files_too() {
        let index = index_of(&["main.rs", "src/main.rs"]);
        let ranked = rank(&index, "main", 10);

        assert_eq!(
            ranked[0].score, ranked[1].score,
            "a root-level file must get the same basename bonus as a nested one"
        );
        assert_eq!(index.entries[ranked[0].idx].rel, "main.rs");
    }

    /// Ties must break deterministically or the palette reshuffles rows under
    /// the cursor between identical rebuilds.
    #[test]
    fn rank_breaks_score_ties_by_path_ascending() {
        let index = index_of(&["b/main.rs", "a/main.rs"]);
        let ranked = rank(&index, "main.rs", 10);

        let matched: Vec<&str> = ranked
            .iter()
            .map(|r| index.entries[r.idx].rel.as_str())
            .collect();
        assert_eq!(matched, vec!["a/main.rs", "b/main.rs"]);
        assert_eq!(ranked[0].score, ranked[1].score);
    }

    /// The palette has no virtualisation, so the caller's row cap has to be
    /// honoured by the matcher rather than by the renderer.
    #[test]
    fn rank_honours_the_limit_and_a_zero_limit() {
        let index = index_of(&["a/main.rs", "b/main.rs", "c/main.rs"]);

        assert_eq!(rank(&index, "main", 2).len(), 2);
        assert!(rank(&index, "main", 0).is_empty());
    }

    /// An empty query is the state the palette opens in, so it must show a
    /// useful, stable prefix of the repo rather than nothing (or a random
    /// subset).
    #[test]
    fn rank_with_an_empty_query_returns_the_sorted_prefix() {
        let index = index_of(&["a.rs", "b.rs", "c.rs"]);
        let ranked = rank(&index, "   ", 2);

        let matched: Vec<&str> = ranked
            .iter()
            .map(|r| index.entries[r.idx].rel.as_str())
            .collect();
        assert_eq!(matched, vec!["a.rs", "b.rs"]);
        assert!(ranked.iter().all(|r| r.score == 0));
    }

    /// Matching is case-insensitive like the command palette's; the prefilter
    /// is the part that could regress this independently of `fuzzy_match`.
    #[test]
    fn rank_matching_is_case_insensitive() {
        let index = index_of(&["src/MainWindow.rs"]);
        assert_eq!(rank(&index, "mainwin", 10).len(), 1);
        assert_eq!(rank(&index, "MAINWIN", 10).len(), 1);
        assert!(rank(&index, "zzz", 10).is_empty());
    }

    /// The prefilter must never reject something `fuzzy_match` accepts, or
    /// results vanish for reasons no one can see in the matcher.
    #[test]
    fn rank_prefilter_agrees_with_the_palette_matcher() {
        let candidates = ["src/main.rs", "README.md", "a/b/c/deep_file.txt"];
        let index = index_of(&candidates);
        for query in ["m", "src", "dft", "readme", "c/deep", "  main  "] {
            let matched_by_matcher = candidates
                .iter()
                .filter(|rel| crate::command_palette::fuzzy_match(query.trim(), rel).is_some())
                .count();
            assert_eq!(
                rank(&index, query, 100).len(),
                matched_by_matcher,
                "prefilter dropped a candidate for query {query:?}"
            );
        }
    }

    /// Staleness drives the rebuild-on-open rule; an index built "now" must
    /// never look stale against the production age, or every palette open
    /// would spawn a git process.
    #[test]
    fn is_stale_reports_age_against_the_requested_window() {
        let index = index_of(&["a.rs"]);
        assert!(!is_stale(&index, DEFAULT_INDEX_MAX_AGE));
        // A zero window means "always rebuild", which is how a user-triggered
        // refresh will be expressed.
        assert!(is_stale(&index, Duration::from_secs(0)));
    }
}
