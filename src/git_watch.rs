//! Off-startup-path git branch resolution.
//!
//! Branch names are sidebar and titlebar decoration. Resolving one costs a
//! `git` process spawn, which on Windows is ~30ms of pure process-creation
//! overhead. Doing that once per workspace while building initial state put
//! that cost -- 220ms for the seven-workspace profile it was measured on --
//! directly in front of the first frame, for text nobody can read yet because
//! there is no window.
//!
//! So workspaces start at [`GitBranch::Pending`] and this module fills them in
//! on a background thread, then asks the UI to rebuild. Two things keep that
//! honest:
//!
//! - Results are written back keyed by workspace `num`, never by index. The
//!   user can close or reorder a workspace during the gap, and an index would
//!   then label the wrong one.
//! - Repositories are deduplicated by path first. Workspaces usually point at
//!   a handful of distinct repos, so seven workspaces are typically one or two
//!   actual spawns.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use unshit::app::{EventSink, ExternalEvent};

use crate::state::{GitBranch, MutexExt, SharedState};

/// Resolve every workspace's branch on a background thread and request one UI
/// rebuild when done.
///
/// Returns immediately. Safe to call before the event loop exists: `sink` is a
/// `OnceLock` that may still be empty, and the caller is expected to send its
/// own rebuild once it fills the lock, so a resolution that lands early is not
/// lost.
pub fn resolve_all_in_background(shared: SharedState, sink: Arc<OnceLock<EventSink>>) {
    let targets: Vec<(u32, PathBuf)> = {
        let guard = shared.lock_recover();
        guard
            .workspaces
            .iter()
            .filter_map(|ws| ws.path.clone().map(|p| (ws.num, p)))
            .collect()
    };
    if targets.is_empty() {
        return;
    }

    std::thread::Builder::new()
        .name("git-branch-resolve".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let resolved = resolve(&targets);
            let spawns = resolved.len();
            let applied = apply(&shared, &targets, &resolved);

            // The UI is already on screen by now, so this is the only thing
            // that makes the branch text appear.
            if let Some(sink) = sink.get() {
                let _ = sink.send(ExternalEvent::RequestRebuild);
            }

            log::info!(
                concat!(
                    r#"{{"event":"git.branches_resolved","level":"info","#,
                    r#""correlation_id":"process-{}","workspaces":{},"#,
                    r#""git_spawns":{},"applied":{},"duration_ms":{:.2}}}"#
                ),
                std::process::id(),
                targets.len(),
                spawns,
                applied,
                started.elapsed().as_secs_f64() * 1000.0,
            );
        })
        .ok();
}

/// One `git` spawn per *distinct* path, not per workspace.
fn resolve(targets: &[(u32, PathBuf)]) -> HashMap<PathBuf, GitBranch> {
    let mut by_path: HashMap<PathBuf, GitBranch> = HashMap::new();
    for (_, path) in targets {
        if by_path.contains_key(path) {
            continue;
        }
        let branch = match crate::git::detect_git_branch(path) {
            Some(name) => GitBranch::Known(name),
            None => GitBranch::Absent,
        };
        by_path.insert(path.clone(), branch);
    }
    by_path
}

/// Write results back by workspace `num`, skipping any workspace that has
/// disappeared or been re-pointed at a different directory while we were out.
/// Returns how many workspaces were updated.
fn apply(
    shared: &SharedState,
    targets: &[(u32, PathBuf)],
    resolved: &HashMap<PathBuf, GitBranch>,
) -> usize {
    let mut guard = shared.lock_recover();
    let mut applied = 0;
    for (num, path) in targets {
        let Some(branch) = resolved.get(path) else {
            continue;
        };
        let Some(ws) = guard.workspaces.iter_mut().find(|ws| ws.num == *num) else {
            continue;
        };
        if ws.path.as_deref() != Some(path.as_path()) {
            continue;
        }
        ws.git_branch = branch.clone();
        applied += 1;
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;
    use std::sync::Mutex;

    fn shared_with(paths: &[(u32, Option<PathBuf>)]) -> SharedState {
        let mut state = seed_state();
        state.workspaces.truncate(1);
        let template = state.workspaces[0].clone();
        state.workspaces.clear();
        for (num, path) in paths {
            let mut ws = template.clone();
            ws.num = *num;
            ws.path = path.clone();
            ws.git_branch = GitBranch::Pending;
            state.workspaces.push(ws);
        }
        Arc::new(Mutex::new(state))
    }

    #[test]
    fn distinct_paths_are_resolved_once_each() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let targets = vec![
            (1, repo.clone()),
            (2, repo.clone()),
            (3, repo.clone()),
            (4, std::env::temp_dir()),
        ];

        let resolved = resolve(&targets);

        // Four workspaces, two distinct directories, two spawns.
        assert_eq!(
            resolved.len(),
            2,
            "shared repositories must not be probed once per workspace"
        );
        assert!(matches!(resolved[&repo], GitBranch::Known(_)));
    }

    #[test]
    fn results_are_keyed_by_workspace_num_not_index() {
        // Workspace 1 is closed while resolution is in flight, so what was
        // index 1 becomes index 0. Keying by index would paint workspace 2
        // with workspace 1's branch.
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let other = std::env::temp_dir();
        let targets = vec![(1, repo.clone()), (2, other.clone())];

        let shared = shared_with(&[(2, Some(other.clone()))]);
        let mut resolved = HashMap::new();
        resolved.insert(repo, GitBranch::Known("from-workspace-1".to_string()));
        resolved.insert(other, GitBranch::Known("from-workspace-2".to_string()));

        let applied = apply(&shared, &targets, &resolved);

        assert_eq!(applied, 1, "only the surviving workspace may be updated");
        let guard = shared.lock_recover();
        assert_eq!(
            guard.workspaces[0].git_branch,
            GitBranch::Known("from-workspace-2".to_string())
        );
    }

    #[test]
    fn a_workspace_repointed_during_the_gap_is_left_alone() {
        let old = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let targets = vec![(1, old.clone())];

        // The user changed workspace 1's directory while git was running; the
        // stale answer describes a repo this workspace no longer points at.
        let shared = shared_with(&[(1, Some(std::env::temp_dir()))]);
        let mut resolved = HashMap::new();
        resolved.insert(old, GitBranch::Known("stale".to_string()));

        let applied = apply(&shared, &targets, &resolved);

        assert_eq!(applied, 0);
        let guard = shared.lock_recover();
        assert_eq!(guard.workspaces[0].git_branch, GitBranch::Pending);
    }

    #[test]
    fn a_directory_that_is_not_a_repo_resolves_to_absent_not_pending() {
        // Pending renders muted forever if nothing ever writes to it; the
        // "we looked and there is nothing" answer has to be recorded.
        let plain = std::env::temp_dir();
        let targets = vec![(1, plain.clone())];
        let shared = shared_with(&[(1, Some(plain.clone()))]);

        let resolved = resolve(&targets);
        apply(&shared, &targets, &resolved);

        let guard = shared.lock_recover();
        assert_eq!(guard.workspaces[0].git_branch, GitBranch::Absent);
    }
}
