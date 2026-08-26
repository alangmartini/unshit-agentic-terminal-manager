//! Reattaching restored panes that are not the active one.
//!
//! `main` brings the **active** pane up eagerly, before the window exists: the
//! renderer needs a terminal in hand to publish cell metrics, and a pane the
//! user is about to look at should not appear empty. Every other restored pane
//! is a different story. A saved layout can carry a dozen panes across tabs and
//! workspaces, each one a synchronous round trip to `unshit-ptyd`, and none of
//! them are visible on the first frame. Doing that work up front put 145ms of
//! IPC in front of the window on the seven-workspace profile it was measured
//! on.
//!
//! So it happens here instead, on a background thread, once the window is on
//! its way up. Two properties make that safe:
//!
//! - **The lock is taken per pane, not for the whole sweep.** Holding
//!   `SharedState` across a dozen daemon round trips would stall the UI thread
//!   for exactly as long as the old code stalled startup -- moving the work to
//!   a thread would have moved the freeze rather than removed it.
//! - **Readers are picked up by the next rebuild.** `bridge::register_reader`
//!   parks a reader in a global; `bridge::build_subscriptions` drains it, and
//!   the framework reconciles subscriptions after every tree rebuild. So one
//!   `RequestRebuild` at the end wires up everything this sweep attached.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use unshit::app::{EventSink, ExternalEvent};

use crate::shell::ShellSpec;
use crate::state::{MutexExt, SharedState};

/// One restored pane waiting to be reattached.
struct Target {
    workspace_id: u32,
    pane_id: u32,
    cwd: Option<PathBuf>,
    shell: Option<ShellSpec>,
}

/// Grid dimensions to spawn a fresh shell at before the real window size is
/// known. Matches the estimate `main` uses for the active pane.
struct InitialGrid {
    cols: u16,
    rows: u16,
}

/// Kick off the reattach sweep and return immediately.
///
/// `sink` may still be empty: the caller fills it right before this is called,
/// and sends its own rebuild afterwards, so a sweep that finishes unusually
/// early is not stranded.
pub fn attach_background_panes_in_background(shared: SharedState, sink: Arc<OnceLock<EventSink>>) {
    std::thread::Builder::new()
        .name("pane-reattach".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let summary = attach_background_panes(&shared);

            // Rebuild whenever there was anything to do, not only on
            // success: a failed attach still writes a spawn failure into
            // state that the user needs to see, and readers registered
            // here only become live subscriptions on the next rebuild.
            if summary.targets > 0 {
                if let Some(sink) = sink.get() {
                    let _ = sink.send(ExternalEvent::RequestRebuild);
                }
            }

            log::info!(
                concat!(
                    r#"{{"event":"panes.background_reattached","level":"info","#,
                    r#""correlation_id":"process-{}","targets":{},"reattached":{},"#,
                    r#""spawned":{},"failed":{},"duration_ms":{:.2}}}"#
                ),
                std::process::id(),
                summary.targets,
                summary.reattached,
                summary.spawned,
                summary.failed,
                started.elapsed().as_secs_f64() * 1000.0,
            );
        })
        .ok();
}

/// What a sweep did, for telemetry and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepSummary {
    /// Panes that needed work. Excludes panes that already had a terminal.
    pub targets: usize,
    /// Found a surviving daemon session and replayed its snapshot.
    pub reattached: usize,
    /// Session was gone, so a fresh shell was started.
    pub spawned: usize,
    pub failed: usize,
}

impl SweepSummary {
    /// Panes that ended up with a live terminal, either way. Non-zero means
    /// readers were registered and a rebuild is needed to subscribe to them.
    pub fn attached(&self) -> usize {
        self.reattached + self.spawned
    }
}

/// Reattach every restored pane that is not the active one.
///
/// Takes and releases the state lock once per pane so a long sweep cannot
/// block the UI thread.
pub fn attach_background_panes(shared: &SharedState) -> SweepSummary {
    let (targets, grid) = collect_targets(shared);
    let mut summary = SweepSummary {
        targets: targets.len(),
        ..SweepSummary::default()
    };

    for target in targets {
        match attach_one(shared, &target, &grid) {
            Outcome::Reattached => summary.reattached += 1,
            Outcome::Spawned => summary.spawned += 1,
            Outcome::Failed => summary.failed += 1,
            Outcome::Skipped => summary.targets -= 1,
        }
    }

    summary
}

enum Outcome {
    Reattached,
    Spawned,
    Failed,
    /// Already had a terminal or an editor; nothing to do.
    Skipped,
}

/// Snapshot every non-active restored pane under a single short lock.
///
/// The active workspace's live tabs are mirrored into `workspaces[active].tabs`
/// by `restore_layout`, so iterating `workspaces` covers every pane.
fn collect_targets(shared: &SharedState) -> (Vec<Target>, InitialGrid) {
    let guard = shared.lock_recover();
    let terminal_font_size = guard.terminal_font_size_pt as f32;
    let cell_w_est = terminal_font_size * guard.cell_width_ratio;
    let cell_h_est = terminal_font_size * crate::state::CSS_LINE_HEIGHT;
    let grid = InitialGrid {
        cols: ((1280.0_f32 - 284.0) / cell_w_est).max(1.0) as u16,
        rows: ((800.0_f32 - 109.0) / cell_h_est).max(1.0) as u16,
    };
    let active_pane_id = guard.active_pane.0;

    let targets = guard
        .workspaces
        .iter()
        .flat_map(|ws| {
            let ws_num = ws.num;
            let cwd = ws.path.clone();
            let shell = crate::shell::resolve(Some(&ws.shell), Some(&guard.default_shell));
            ws.tabs
                .iter()
                .flat_map(|tab| tab.panes.iter().flatten())
                .filter(|pane| pane.id.0 != active_pane_id)
                .map(move |pane| Target {
                    workspace_id: ws_num,
                    pane_id: pane.id.0,
                    cwd: cwd.clone(),
                    shell: shell.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();

    (targets, grid)
}

/// Reattach a single pane, holding the state lock only for this one pane.
///
/// A cache hit replays the surviving daemon session's snapshot; a miss (the
/// shell exited while we were gone, or an upgrade) spawns a fresh shell.
fn attach_one(shared: &SharedState, target: &Target, grid: &InitialGrid) -> Outcome {
    let Target {
        workspace_id,
        pane_id,
        cwd,
        shell,
    } = target;
    let (workspace_id, pane_id) = (*workspace_id, *pane_id);

    let mut guard = shared.lock_recover();
    if guard.terminals.contains_key(&pane_id) || guard.editors.contains_key(&pane_id) {
        return Outcome::Skipped;
    }
    let spawn_plan =
        crate::state::pane_agent_spawn_plan(&guard, pane_id, cwd.clone(), shell.clone());
    let launch_prepared = crate::state::prepare_agent_resume_launch(
        &mut guard,
        pane_id,
        workspace_id,
        &spawn_plan,
        "background",
    );
    let reconcile_result = if launch_prepared {
        guard.pty_manager.attach_or_spawn(
            pane_id,
            workspace_id,
            grid.cols,
            grid.rows,
            spawn_plan.cwd.as_deref(),
            spawn_plan.shell.as_ref(),
        )
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "agent recovery launch preflight was not durable",
        ))
    };

    match reconcile_result {
        Ok((Some(snapshot), reader)) => {
            let rows = snapshot.grid.rows();
            let cols = snapshot.grid.cols();
            let mut terminal = crate::terminal::Terminal::new(rows, cols);
            terminal.apply_snapshot(&snapshot);
            guard
                .terminals
                .insert(pane_id, Arc::new(std::sync::Mutex::new(terminal)));
            crate::bridge::register_reader(pane_id, reader);
            crate::state::apply_agent_spawn_outcome(
                &mut guard,
                pane_id,
                workspace_id,
                &spawn_plan,
                true,
                "background",
            );
            log::info!(
                "reattached background pane {} (workspace {}) to surviving session ({}x{})",
                pane_id,
                workspace_id,
                cols,
                rows
            );
            Outcome::Reattached
        }
        Ok((None, reader)) => {
            let terminal = crate::terminal::Terminal::new(grid.rows as usize, grid.cols as usize);
            guard
                .terminals
                .insert(pane_id, Arc::new(std::sync::Mutex::new(terminal)));
            crate::bridge::register_reader(pane_id, reader);
            crate::state::apply_agent_spawn_outcome(
                &mut guard,
                pane_id,
                workspace_id,
                &spawn_plan,
                false,
                "background",
            );
            log::info!(
                "background pane {} (workspace {}) had no surviving session; spawned fresh",
                pane_id,
                workspace_id
            );
            Outcome::Spawned
        }
        Err(e) => {
            crate::state::record_agent_spawn_failure(
                &mut guard,
                pane_id,
                workspace_id,
                &spawn_plan,
                "background",
                &e,
            );
            log::error!(
                "failed to reattach/spawn background pane {} (workspace {}): {}",
                pane_id,
                workspace_id,
                e
            );
            Outcome::Failed
        }
    }
}
