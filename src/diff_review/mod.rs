pub mod git;

use crate::state::{AppState, MutexExt, SharedState};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

pub const PAGE_LINES: usize = 200;
pub const PAGE_FILES: usize = 100;

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
    pub selected: usize,
    pub page: usize,
    pub file_page: usize,
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
            selected: 0,
            page: 0,
            file_page: 0,
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
        let lines = report
            .as_ref()
            .ok()
            .and_then(|r| r.files.get(selected).map(|f| git::patch(r, f)));
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
    lines: Option<Result<Vec<git::Line>, String>>,
) -> bool {
    let Some(review) = state.diff_review.as_mut().filter(|r| r.request == id) else {
        return false;
    };
    review.loading = false;
    match report {
        Ok(report) => {
            review.report = Some(report);
            match lines {
                Some(Ok(lines)) => review.lines = Arc::new(lines),
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
    review.lines = Arc::default();
    review.page = 0;
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
                review.lines = Arc::default();
                review.error = Some("Choose between 1 and 10000 commits.".into());
                return;
            }
        },
    };
    review.report = None;
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
        "diff.refresh" => refresh(review),
        "diff.mode:last" | "diff.mode:unpushed" | "diff.mode:base" => {
            review.mode = match command {
                "diff.mode:base" => "base",
                "diff.mode:unpushed" => "unpushed",
                _ => "last",
            };
            refresh(review);
        }
        "diff.prev" => review.page = review.page.saturating_sub(1),
        "diff.next" if (review.page + 1) * PAGE_LINES < review.lines.len() => review.page += 1,
        "diff.files_prev" => review.file_page = review.file_page.saturating_sub(1),
        "diff.files_next"
            if review
                .report
                .as_ref()
                .is_some_and(|r| (review.file_page + 1) * PAGE_FILES < r.files.len()) =>
        {
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
