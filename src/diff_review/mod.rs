pub mod git;
pub mod split;

use crate::state::{AppState, MutexExt, SharedState};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

pub const PAGE_LINES: usize = 200;
pub const PAGE_FILES: usize = 100;

#[derive(Clone, Debug)]
pub struct Hunk {
    pub unified_row: usize,
    pub split_row: usize,
}

pub fn collect_hunks(lines: &[git::Line], rows: &[split::Row]) -> Vec<Hunk> {
    rows.iter()
        .enumerate()
        .filter_map(|(split_row, row)| match row {
            split::Row::Shared(line) if lines[*line].kind == "hunk" => Some(Hunk {
                unified_row: *line,
                split_row,
            }),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Review {
    pub root: PathBuf,
    pub mode: &'static str,
    pub count: String,
    pub base_ref: String,
    pub request: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub report: Option<Arc<git::Report>>,
    pub lines: Arc<Vec<git::Line>>,
    pub split_rows: Arc<Vec<split::Row>>,
    pub side_by_side: bool,
    pub selected: usize,
    pub row_start: usize,
    pub hunks: Arc<Vec<Hunk>>,
    pub active_hunk: Option<usize>,
    pub file_page: usize,
    pub file_filter: String,
    /// Inputs own their live buffer; only explicit Clear remounts the input.
    pub file_filter_reset: u64,
    pub file_matches: Arc<Vec<usize>>,
}

impl Review {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            mode: "last",
            count: "1".into(),
            base_ref: "main".into(),
            request: 0,
            loading: false,
            error: None,
            report: None,
            lines: Arc::default(),
            split_rows: Arc::default(),
            side_by_side: false,
            selected: 0,
            row_start: 0,
            hunks: Arc::default(),
            active_hunk: None,
            file_page: 0,
            file_filter: String::new(),
            file_filter_reset: 0,
            file_matches: Arc::default(),
        }
    }

    pub fn row_count(&self) -> usize {
        if self.side_by_side {
            self.split_rows.len()
        } else {
            self.lines.len()
        }
    }

    pub fn set_file_filter(&mut self, query: &str) {
        self.file_filter = query.chars().take(256).collect();
        self.rebuild_file_matches();
    }

    fn hunk_row(&self, index: usize) -> usize {
        let hunk = &self.hunks[index];
        if self.side_by_side {
            hunk.split_row
        } else {
            hunk.unified_row
        }
    }

    pub fn hunk_target(&self, forward: bool) -> Option<usize> {
        if self.loading {
            return None;
        }
        if let Some(index) = self.active_hunk {
            if forward {
                (index + 1 < self.hunks.len()).then_some(index + 1)
            } else {
                index.checked_sub(1)
            }
        } else if forward {
            (0..self.hunks.len()).find(|&index| self.hunk_row(index) >= self.row_start)
        } else {
            (0..self.hunks.len()).rfind(|&index| self.hunk_row(index) < self.row_start)
        }
    }

    pub fn is_active_hunk_line(&self, line: usize) -> bool {
        self.active_hunk
            .is_some_and(|index| self.hunks[index].unified_row == line)
    }

    fn clear_patch(&mut self) {
        self.lines = Arc::default();
        self.split_rows = Arc::default();
        self.hunks = Arc::default();
        self.active_hunk = None;
        self.row_start = 0;
    }

    fn rebuild_file_matches(&mut self) {
        let needle = self.file_filter.trim().replace('\\', "/").to_lowercase();
        self.file_matches = Arc::new(
            self.report
                .as_ref()
                .map(|report| {
                    report
                        .files
                        .iter()
                        .enumerate()
                        .filter_map(|(index, file)| {
                            let matches = |path: &str| {
                                path.replace('\\', "/").to_lowercase().contains(&needle)
                            };
                            (needle.is_empty()
                                || matches(&file.path)
                                || file.old_path.as_deref().is_some_and(matches))
                            .then_some(index)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        );
        self.file_page = 0;
    }
}

struct LoadedPatch {
    lines: Arc<Vec<git::Line>>,
    split_rows: Arc<Vec<split::Row>>,
    hunks: Arc<Vec<Hunk>>,
}

impl LoadedPatch {
    fn new(lines: Vec<git::Line>) -> Self {
        let split_rows = Arc::new(split::align(&lines));
        let hunks = Arc::new(collect_hunks(&lines, &split_rows));
        Self {
            lines: Arc::new(lines),
            split_rows,
            hunks,
        }
    }
}

enum Query {
    Range(PathBuf, git::Range),
    File(Arc<git::Report>, usize),
}
struct Job {
    id: u64,
    query: Query,
}
#[derive(Default)]
struct Queue {
    latest: Mutex<Option<Job>>,
    ready: Condvar,
}
static QUEUE: OnceLock<Arc<Queue>> = OnceLock::new();
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// One worker, one replaceable queued request. Repeated clicks coalesce.
pub fn start(shared: SharedState, sink: unshit::app::EventSink) {
    let queue = Arc::new(Queue::default());
    if QUEUE.set(queue.clone()).is_err() {
        return;
    }
    std::thread::spawn(move || loop {
        let job = {
            let mut guard = queue.latest.lock_recover();
            while guard.is_none() {
                guard = queue.ready.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
            guard.take().unwrap()
        };
        let (report, selected) = match job.query {
            Query::Range(root, range) => (git::load(&root, &range).map(Arc::new), 0),
            Query::File(report, index) => (Ok(report), index),
        };
        let lines = report.as_ref().ok().and_then(|r| {
            r.files
                .get(selected)
                .map(|f| git::patch(r, f).map(LoadedPatch::new))
        });
        let mut state = shared.lock_recover();
        let applied = apply(&mut state, job.id, report, lines);
        drop(state);
        if applied {
            let _ = sink.send(unshit::app::ExternalEvent::RequestRebuild);
        }
    });
}

fn apply(
    state: &mut AppState,
    id: u64,
    report: Result<Arc<git::Report>, String>,
    lines: Option<Result<LoadedPatch, String>>,
) -> bool {
    let Some(review) = state.diff_review.as_mut().filter(|r| r.request == id) else {
        return false;
    };
    review.loading = false;
    match report {
        Ok(report) => {
            let changed = !review
                .report
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, &report));
            review.report = Some(report);
            if changed {
                review.rebuild_file_matches();
            }
            match lines {
                Some(Ok(patch)) => {
                    review.lines = patch.lines;
                    review.split_rows = patch.split_rows;
                    review.hunks = patch.hunks;
                    review.active_hunk = None;
                    review.row_start = 0;
                }
                Some(Err(error)) => review.error = Some(error),
                None => {}
            }
        }
        Err(error) => review.error = Some(error),
    }
    true
}

fn submit(review: &mut Review, query: Query) {
    review.request = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    review.clear_patch();
    review.error = None;
    let Some(queue) = QUEUE.get() else {
        review.loading = false;
        review.error = Some("Git review worker is not available.".into());
        return;
    };
    review.loading = true;
    *queue.latest.lock_recover() = Some(Job {
        id: review.request,
        query,
    });
    queue.ready.notify_one();
}

fn refresh(review: &mut Review) {
    let range = match review.mode {
        "unpushed" => git::Range::Unpushed,
        "base" => git::Range::Base(review.base_ref.clone()),
        _ => match review.count.parse::<usize>() {
            Ok(n) if (1..=10_000).contains(&n) => git::Range::Last(n),
            _ => {
                review.request = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                review.loading = false;
                review.report = None;
                review.file_matches = Arc::default();
                review.clear_patch();
                review.error = Some("Choose between 1 and 10000 commits.".into());
                return;
            }
        },
    };
    review.report = None;
    review.file_matches = Arc::default();
    review.selected = 0;
    review.file_page = 0;
    submit(review, Query::Range(review.root.clone(), range));
}

pub fn dispatch(state: &mut AppState, command: &str) -> bool {
    if command == "diff.open" {
        let root = state
            .pty_manager
            .spawn_cwd(state.active_pane.0)
            .map(PathBuf::from)
            .or_else(|| {
                state
                    .agent_restarts
                    .get(&state.active_pane.0)
                    .map(|r| r.cwd.clone())
            })
            .or_else(|| crate::state::active_workspace_cwd(state))
            .unwrap_or_default();
        let mut review = Review::new(root);
        refresh(&mut review);
        state.diff_review = Some(review);
        return true;
    }
    if command == "diff.close" {
        return state.diff_review.take().is_some();
    }
    let Some(review) = state.diff_review.as_mut() else {
        return false;
    };
    match command {
        "diff.hunk_prev" | "diff.hunk_next" => {
            let Some(index) = review.hunk_target(command == "diff.hunk_next") else {
                return false;
            };
            review.active_hunk = Some(index);
            review.row_start = review.hunk_row(index);
        }
        "diff.file_start" if review.row_start > 0 || review.active_hunk.is_some() => {
            review.row_start = 0;
            review.active_hunk = None;
        }
        "diff.filter_clear" => {
            review.set_file_filter("");
            review.file_filter_reset = review.file_filter_reset.wrapping_add(1);
        }
        other if let Some(query) = other.strip_prefix("diff.filter:") => {
            review.set_file_filter(query)
        }
        "diff.view:unified" | "diff.view:split" => {
            let split = command == "diff.view:split";
            if review.side_by_side == split {
                return false;
            }
            review.side_by_side = split;
            review.row_start = review.active_hunk.map_or(0, |index| review.hunk_row(index));
        }
        "diff.refresh" => refresh(review),
        "diff.mode:last" | "diff.mode:unpushed" | "diff.mode:base" => {
            review.mode = match command {
                "diff.mode:base" => "base",
                "diff.mode:unpushed" => "unpushed",
                _ => "last",
            };
            refresh(review);
        }
        "diff.prev" if review.row_start > 0 => {
            review.row_start = review.row_start.saturating_sub(PAGE_LINES);
            review.active_hunk = None;
        }
        "diff.next" if review.row_start + PAGE_LINES < review.row_count() => {
            review.row_start += PAGE_LINES;
            review.active_hunk = None;
        }
        "diff.files_prev" => review.file_page = review.file_page.saturating_sub(1),
        "diff.files_next" if (review.file_page + 1) * PAGE_FILES < review.file_matches.len() => {
            review.file_page += 1
        }
        _ => {
            let Some(index) = command
                .strip_prefix("diff.file:")
                .and_then(|n| n.parse::<usize>().ok())
            else {
                return false;
            };
            let Some(report) = review.report.clone().filter(|r| index < r.files.len()) else {
                return false;
            };
            review.selected = index;
            submit(review, Query::File(report, index));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;

    fn state_with_patch(text: &str) -> AppState {
        let mut state = seed_state();
        let mut review = Review::new(".".into());
        review.request = 42;
        state.diff_review = Some(review);
        let report = Arc::new(git::Report {
            root: ".".into(),
            base: "base".into(),
            head: "head".into(),
            label: "test".into(),
            files: vec![],
        });
        assert!(apply(
            &mut state,
            42,
            Ok(report),
            Some(Ok(LoadedPatch::new(git::parse_patch(text))))
        ));
        state
    }

    #[test]
    fn hunk_navigation_crosses_pages_without_reloading_git() {
        let patch = format!("diff --git a/a b/a\n@@ -1,211 +1,211 @@\n{}{} same\n@@ -900 +900 @@\n-before\n+after\n", "-old\n".repeat(210), "+new\n".repeat(210));
        let mut state = state_with_patch(&patch);
        let lines = state.diff_review.as_ref().unwrap().lines.clone();
        assert!(!dispatch(&mut state, "diff.hunk_prev"));
        assert!(dispatch(&mut state, "diff.hunk_next"));
        assert_eq!(state.diff_review.as_ref().unwrap().row_start, 1);
        assert!(dispatch(&mut state, "diff.hunk_next"));
        assert_eq!(state.diff_review.as_ref().unwrap().row_start, 423);
        assert_eq!(state.diff_review.as_ref().unwrap().active_hunk, Some(1));
        assert!(!dispatch(&mut state, "diff.hunk_next"));
        assert!(dispatch(&mut state, "diff.view:split"));
        assert_eq!(state.diff_review.as_ref().unwrap().row_start, 213);
        assert_eq!(state.diff_review.as_ref().unwrap().active_hunk, Some(1));
        assert!(dispatch(&mut state, "diff.hunk_prev"));
        assert!(!dispatch(&mut state, "diff.hunk_prev"));
        let review = state.diff_review.as_ref().unwrap();
        assert_eq!(review.request, 42);
        assert!(Arc::ptr_eq(&lines, &review.lines));
        assert!(dispatch(&mut state, "diff.next"));
        assert_eq!(state.diff_review.as_ref().unwrap().row_start, 201);
        assert_eq!(state.diff_review.as_ref().unwrap().active_hunk, None);
        assert!(dispatch(&mut state, "diff.hunk_next"));
        assert_eq!(state.diff_review.as_ref().unwrap().row_start, 213);
        assert!(dispatch(&mut state, "diff.file_start"));
        assert_eq!(state.diff_review.as_ref().unwrap().row_start, 0);
        assert_eq!(state.diff_review.as_ref().unwrap().active_hunk, None);
        assert!(!dispatch(&mut state, "diff.file_start"));
        assert!(dispatch(&mut state, "diff.hunk_next"));
        state.diff_review.as_mut().unwrap().count = "0".into();
        assert!(dispatch(&mut state, "diff.refresh"));
        assert!(state.diff_review.as_ref().unwrap().hunks.is_empty());
        assert_eq!(state.diff_review.as_ref().unwrap().active_hunk, None);
        assert!(!dispatch(&mut state, "diff.hunk_next"));
    }

    #[test]
    fn metadata_only_patches_have_no_hunk_navigation() {
        for text in [
            "",
            "diff --git a/a b/a\nBinary files a/a and b/a differ\n",
            "diff --git a/a b/b\nrename from a\nrename to b\n",
        ] {
            let mut state = state_with_patch(text);
            assert!(state.diff_review.as_ref().unwrap().hunks.is_empty());
            assert!(!dispatch(&mut state, "diff.hunk_prev"));
            assert!(!dispatch(&mut state, "diff.hunk_next"));
        }
    }

    #[test]
    fn file_filter_matches_current_and_renamed_paths_without_reloading_patch() {
        let mut review = Review::new(".".into());
        review.report = Some(Arc::new(git::Report {
            root: ".".into(),
            base: "base".into(),
            head: "head".into(),
            label: "test".into(),
            files: vec![
                git::File {
                    path: "src/Main.rs".into(),
                    old_path: None,
                    added: Some(1),
                    removed: Some(0),
                },
                git::File {
                    path: "assets/new.css".into(),
                    old_path: Some("styles/OLD.css".into()),
                    added: Some(0),
                    removed: Some(0),
                },
                git::File {
                    path: "README.md".into(),
                    old_path: None,
                    added: Some(1),
                    removed: Some(0),
                },
            ],
        }));
        let lines = review.lines.clone();
        review.selected = 2;
        review.row_start = 3 * PAGE_LINES;
        review.file_page = 2;
        review.request = 42;
        review.loading = true;
        review.set_file_filter(" SRC\\main ");
        assert_eq!(&*review.file_matches, &[0]);
        assert_eq!(review.file_page, 0);
        review.set_file_filter("STYLES/old");
        assert_eq!(&*review.file_matches, &[1]);
        review.set_file_filter("absent");
        assert!(review.file_matches.is_empty());
        review.set_file_filter("");
        assert_eq!(&*review.file_matches, &[0, 1, 2]);
        assert_eq!(review.selected, 2);
        assert_eq!(review.row_start, 3 * PAGE_LINES);
        assert_eq!(review.request, 42);
        assert!(review.loading);
        assert!(Arc::ptr_eq(&review.lines, &lines));
    }

    #[test]
    fn filtered_pagination_uses_original_indices_and_survives_patch_results() {
        let mut state = seed_state();
        let mut review = Review::new(".".into());
        let file = git::File {
            path: "unmatched".into(),
            old_path: None,
            added: Some(1),
            removed: Some(0),
        };
        let mut files = vec![file.clone()];
        files.extend((0..201).map(|n| git::File {
            path: format!("src/file{n}.rs"),
            ..file.clone()
        }));
        let report = Arc::new(git::Report {
            root: ".".into(),
            base: "base".into(),
            head: "head".into(),
            label: "test".into(),
            files,
        });
        review.report = Some(report.clone());
        review.set_file_filter("src/");
        state.diff_review = Some(review);
        assert!(dispatch(&mut state, "diff.files_next"));
        let index = state.diff_review.as_ref().unwrap().file_matches[PAGE_FILES];
        assert_eq!(index, 101);
        assert!(dispatch(&mut state, &format!("diff.file:{index}")));
        let review = state.diff_review.as_ref().unwrap();
        let id = review.request;
        let matches = review.file_matches.clone();
        assert_eq!(review.selected, 101);
        assert!(apply(&mut state, id, Ok(report.clone()), None));
        let review = state.diff_review.as_ref().unwrap();
        assert_eq!(review.file_page, 1);
        assert!(Arc::ptr_eq(&matches, &review.file_matches));
        assert!(dispatch(&mut state, "diff.files_next"));
        assert!(!dispatch(&mut state, "diff.files_next"));
        let mut refreshed = (*report).clone();
        refreshed.files.truncate(1);
        assert!(apply(&mut state, id, Ok(Arc::new(refreshed)), None));
        let review = state.diff_review.as_ref().unwrap();
        assert!(review.file_matches.is_empty());
        assert_eq!(review.file_filter, "src/");
        assert_eq!(review.file_page, 0);
    }

    #[test]
    fn view_switch_reuses_patch_and_paginates_aligned_rows() {
        let mut state = seed_state();
        let mut review = Review::new(".".into());
        let patch = format!(
            "@@ -1,201 +1,201 @@\n{}{}",
            "-before\n".repeat(201),
            "+after\n".repeat(201)
        );
        review.lines = Arc::new(git::parse_patch(&patch));
        review.split_rows = Arc::new(split::align(&review.lines));
        let original = review.lines.clone();
        review.request = 42;
        review.loading = true;
        state.diff_review = Some(review);
        assert!(dispatch(&mut state, "diff.view:split"));
        assert!(dispatch(&mut state, "diff.next"));
        assert!(!dispatch(&mut state, "diff.next"));
        let review = state.diff_review.as_ref().unwrap();
        assert_eq!(review.row_start, PAGE_LINES);
        assert_eq!(review.row_count(), 202);
        assert_eq!(review.request, 42);
        assert!(review.loading);
        assert!(Arc::ptr_eq(&review.lines, &original));
        assert!(dispatch(&mut state, "diff.view:unified"));
        let review = state.diff_review.as_ref().unwrap();
        assert_eq!(review.row_start, 0);
        assert_eq!(review.row_count(), 403);
        assert!(Arc::ptr_eq(&review.lines, &original));
    }

    #[test]
    fn late_results_cannot_overwrite_new_selection_or_reopen_closed_review() {
        let mut state = seed_state();
        let mut review = Review::new(".".into());
        review.request = 42;
        review.loading = true;
        state.diff_review = Some(review);
        assert!(!apply(&mut state, 41, Err("stale".into()), None));
        assert!(state.diff_review.as_ref().unwrap().loading);
        assert!(state.diff_review.as_ref().unwrap().error.is_none());
        assert!(apply(&mut state, 42, Err("current".into()), None));
        assert_eq!(
            state.diff_review.as_ref().unwrap().error.as_deref(),
            Some("current")
        );
        assert!(dispatch(&mut state, "diff.close"));
        assert!(!apply(&mut state, 42, Err("closed".into()), None));
        assert!(state.diff_review.is_none());
    }

    #[test]
    fn palette_opens_review_and_escape_closes_it_without_new_terminals() {
        let mut state = seed_state();
        let tabs = state.tabs.len();
        let next_id = state.next_id;
        crate::state::dispatch(&mut state, "palette.toggle");
        assert!(crate::state::dispatch(
            &mut state,
            "palette.execute:git_diff_review"
        ));
        assert!(state.diff_review.is_some());
        assert!(!state.palette_open);
        assert_eq!(state.tabs.len(), tabs);
        assert_eq!(state.next_id, next_id);
        assert!(crate::state::dispatch(&mut state, "modal.close"));
        assert!(state.diff_review.is_none());
    }
}
