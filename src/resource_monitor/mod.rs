//! Live resource usage for the status bar and pane headers.
//!
//! Nothing in the UI used to tick: `cpu_pct` and `net_kbps` were never
//! written, `mem_gb` only changed when the Sessions panel was opened, the
//! clock stayed at its seed value and every pane header read `pid 0 ·
//! 0.0%`. This module owns those numbers.
//!
//! Once a second a background thread:
//!
//! 1. asks the daemon which shell pid backs each session (cached, refreshed
//!    when the UI's pane→session set changes or every
//!    `ROOTS_REFRESH_INTERVAL`),
//! 2. snapshots the machine's process table and files every process under
//!    its nearest root — a session shell, the daemon, or the UI — so a
//!    tab's number covers the agent, its node children and the `git` it
//!    just spawned, not only the shell ([`tree`]),
//! 3. reads each attributed process's CPU time and working set and diffs
//!    CPU against the previous tick ([`cpu`]),
//! 4. writes the totals, the per-pane figures, the per-root tree map
//!    (`AppState::resource_trees`, read by the status bar's tab item, the
//!    sidebar chips and the Sessions panel) and, when the daemon was just
//!    listed, the Sessions panel rows into `AppState`, then requests a
//!    rebuild **only if a displayed value changed**.
//!
//! Unknown is rendered as `--`, never as `0.0`: before the first CPU diff,
//! when the daemon cannot be listed, or on a platform without a sampler.
//! The `↓ k/s` figure is PTY output received from the daemon across all
//! sessions; per-process network I/O is not cheaply observable on Windows.
//!
//! Sampling happens off the render path; only the write-back takes the
//! state lock, and the daemon round trip never happens under it.

pub mod cpu;
pub mod platform;
pub mod telemetry;
pub mod tree;

mod monitor;
pub use monitor::start;

use std::collections::HashMap;
use std::sync::Arc;

use crate::state::{AppState, SessionSnapshot};

const GIB: f32 = 1024.0 * 1024.0 * 1024.0;

/// Usage of one pane's process tree, rooted at its session's shell.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneResources {
    pub pane_id: u32,
    /// The shell pid the daemon reported for the pane's session.
    pub pid: u32,
    /// `None` until a second tick provides a baseline.
    pub cpu_pct: Option<f32>,
    /// Working set summed over the tree.
    pub mem_bytes: u64,
    pub process_count: u32,
}

/// Usage of one process tree, keyed in `AppState::resource_trees` by its
/// root pid: the UI, the daemon or a session's shell. The Sessions panel,
/// the sidebar chips and the status bar's tab item read it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeUsage {
    /// `None` until a second tick provides a baseline.
    pub cpu_pct: Option<f32>,
    /// Working set summed over the tree.
    pub mem_bytes: u64,
    pub process_count: u32,
    /// Image name of the root process (`pwsh.exe`), the shell that is
    /// actually running rather than whatever the pane was seeded with.
    pub root_exe: Option<String>,
    /// Sampled members, shared cheaply with UI snapshots.
    pub processes: Arc<[ProcessUsage]>,
}

/// Working set of an individual process from the same tick as its tree total.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessUsage {
    pub pid: u32,
    pub name: String,
    pub mem_bytes: u64,
}

/// The daemon's session list as of a roots refresh. Applied to the state
/// so the Sessions panel rows follow the daemon without a manual refresh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DaemonListing {
    pub daemon_pid: Option<u32>,
    pub daemon_memory_rss_bytes: Option<u64>,
    pub sessions: Vec<SessionSnapshot>,
}

/// What one tick found. Every total is `None` when it is not known.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceReport {
    /// UI + daemon + every session tree, as a share of the whole machine.
    pub cpu_pct: Option<f32>,
    /// Working set of every process in every tree.
    pub mem_bytes: Option<u64>,
    /// PTY output received from the daemon since the previous tick, in KiB/s.
    pub net_kbps: Option<f32>,
    pub clock_hhmm: Option<String>,
    pub panes: Vec<PaneResources>,
    /// Every sampled tree by root pid; empty when nothing could be sampled.
    pub trees: HashMap<u32, TreeUsage>,
    /// Fresh daemon rows when the roots were refreshed this tick.
    pub listing: Option<DaemonListing>,
    /// Whether the daemon could not be listed, so cached rows may be stale.
    pub listing_failed: bool,
    /// Process observations for owned sessions. Missing rows mean unknown,
    /// while a row with no profile means a successful scan found no agent.
    pub agents: Vec<ProcessAgentObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessAgentObservation {
    pub pane_id: u32,
    pub session_id: u64,
    pub profile: Option<String>,
}

/// Write a report into the state. Returns whether anything the status bar
/// or a pane header displays actually changed, so a quiet app is not
/// rebuilt once a second.
pub fn apply_report(state: &mut AppState, report: &ResourceReport) -> bool {
    let before = DisplayKey::of(state);

    state.cpu_pct = report.cpu_pct;
    state.mem_gb = report.mem_bytes.map(|bytes| bytes as f32 / GIB);
    state.net_kbps = report.net_kbps;
    if let Some(clock) = &report.clock_hhmm {
        state.clock_hhmm.clone_from(clock);
    }
    state.resource_trees.clone_from(&report.trees);
    if let Some(listing) = &report.listing {
        state.daemon_pid = listing.daemon_pid;
        state.daemon_memory_rss_bytes = listing.daemon_memory_rss_bytes;
        state.sessions.clone_from(&listing.sessions);
        state.sessions_stale = false;
    } else if report.listing_failed {
        state.sessions_stale = true;
    }

    let by_pane: HashMap<u32, &PaneResources> =
        report.panes.iter().map(|p| (p.pane_id, p)).collect();
    for pane in all_panes_mut(state) {
        match by_pane.get(&pane.id.0) {
            Some(res) => {
                pane.pid = res.pid;
                pane.cpu = res.cpu_pct.unwrap_or(0.0);
                pane.mem_bytes = res.mem_bytes;
            }
            None => {
                pane.pid = 0;
                pane.cpu = 0.0;
                pane.mem_bytes = 0;
            }
        }
    }

    let mut agents_changed = false;
    for observation in &report.agents {
        // The pane may have closed or reattached while the sampler was
        // outside the state lock. Never apply evidence from its old session.
        if state.pty_manager.session_id(observation.pane_id) == Some(observation.session_id) {
            agents_changed |= crate::state::classify_pane_process(
                state,
                observation.pane_id,
                observation.profile.as_deref(),
            );
        }
    }
    agents_changed || DisplayKey::of(state) != before
}

/// Every pane the UI can show: the active tab's live layout lives in
/// `state.panes`, inactive tabs keep theirs on the `TerminalTab`. Both are
/// updated so a tab switch shows current figures, not the ones from when
/// the tab was last active.
fn all_panes_mut(state: &mut AppState) -> impl Iterator<Item = &mut crate::state::Pane> {
    let AppState {
        panes, workspaces, ..
    } = state;
    panes.iter_mut().flatten().chain(
        workspaces
            .iter_mut()
            .flat_map(|ws| ws.tabs.iter_mut())
            .flat_map(|tab| tab.panes.iter_mut().flatten()),
    )
}

fn all_panes(state: &AppState) -> impl Iterator<Item = &crate::state::Pane> {
    state.panes.iter().flatten().chain(
        state
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.panes.iter().flatten()),
    )
}

/// Sampled processes behind the active tab's panes, deduplicated by pid so a
/// root shared by split panes isn't counted twice. Shared by the process
/// dialog and its `DisplayKey` change-detection.
pub(crate) fn active_tab_processes<'a>(
    panes: &'a [Vec<crate::state::Pane>],
    resource_trees: &'a HashMap<u32, TreeUsage>,
) -> Vec<&'a ProcessUsage> {
    let mut rows: Vec<&ProcessUsage> = panes
        .iter()
        .flatten()
        .filter(|pane| pane.pid != 0)
        .filter_map(|pane| resource_trees.get(&pane.pid))
        .flat_map(|tree| tree.processes.iter())
        .collect();
    rows.sort_unstable_by_key(|p| p.pid);
    rows.dedup_by_key(|p| p.pid);
    rows
}

/// Everything the UI renders from resource data, quantised to what is
/// visible (one decimal of percent, two of GiB, whole MiB per tree).
#[derive(Debug, PartialEq, Eq)]
struct DisplayKey {
    cpu_tenths: Option<i64>,
    mem_hundredths: Option<i64>,
    net_tenths: Option<i64>,
    clock: String,
    /// Pane id, pid, cpu tenths, whole MiB, process count: what the pane
    /// header, the sidebar chip and the status bar's tab item show.
    panes: Vec<(u32, u32, i64, u64, u32)>,
    /// What the Sessions panel shows, only while it is open; a closed panel
    /// must not cost a rebuild when an unowned session's memory moves.
    sessions_panel: Option<SessionsPanelKey>,
    process_details: Vec<(u32, String, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
struct SessionsPanelKey {
    /// Root pid, cpu tenths (`-1` unknown), whole MiB, process count.
    trees: Vec<(u32, i64, u64, u32)>,
    rows: Vec<SessionSnapshot>,
    daemon_pid: Option<u32>,
    daemon_mem_mib: Option<u64>,
    stale: bool,
}

impl DisplayKey {
    fn of(state: &AppState) -> Self {
        let tenths = |v: f32| (v * 10.0).round() as i64;
        let count_of = |pid: u32| {
            state
                .resource_trees
                .get(&pid)
                .map_or(0, |t| t.process_count)
        };
        let sessions_open = state.settings_open
            && state.settings_section == crate::state::SettingsSection::Sessions;
        Self {
            cpu_tenths: state.cpu_pct.map(tenths),
            mem_hundredths: state.mem_gb.map(|v| (v * 100.0).round() as i64),
            net_tenths: state.net_kbps.map(tenths),
            clock: state.clock_hhmm.clone(),
            panes: all_panes(state)
                .map(|p| {
                    (
                        p.id.0,
                        p.pid,
                        tenths(p.cpu),
                        p.mem_bytes >> 20,
                        count_of(p.pid),
                    )
                })
                .collect(),
            process_details: if state.process_details_open {
                active_tab_processes(&state.panes, &state.resource_trees)
                    .into_iter()
                    .map(|p| (p.pid, p.name.clone(), p.mem_bytes))
                    .collect()
            } else {
                Vec::new()
            },
            sessions_panel: sessions_open.then(|| {
                let mut trees: Vec<(u32, i64, u64, u32)> = state
                    .resource_trees
                    .iter()
                    .map(|(&pid, t)| {
                        (
                            pid,
                            t.cpu_pct.map_or(-1, tenths),
                            t.mem_bytes >> 20,
                            t.process_count,
                        )
                    })
                    .collect();
                trees.sort_unstable();
                SessionsPanelKey {
                    trees,
                    rows: state.sessions.clone(),
                    daemon_pid: state.daemon_pid,
                    daemon_mem_mib: state.daemon_memory_rss_bytes.map(|b| b >> 20),
                    stale: state.sessions_stale,
                }
            }),
        }
    }
}

/// `412M` / `1.2G`: the sidebar chip has room for little more.
pub fn format_short_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= MIB * 1024.0 {
        format!("{:.1}G", value / (MIB * 1024.0))
    } else {
        format!("{:.0}M", value / MIB)
    }
}

/// `412 MiB` below a GiB, `1.25 GiB` from there: compact enough for a pane
/// header.
pub fn format_compact_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= MIB * 1024.0 {
        format!("{:.2} GiB", value / (MIB * 1024.0))
    } else {
        format!("{:.0} MiB", value / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;

    fn first_pane_id(state: &AppState) -> u32 {
        state.panes[0][0].id.0
    }

    #[test]
    fn process_observations_move_panes_and_only_rebuild_on_transitions() {
        let mut state = seed_state();
        let pane_id = first_pane_id(&state);
        state
            .pty_manager
            .test_install_broken_inner_with_session(pane_id, 42);
        let mut report = ResourceReport::default();
        apply_report(&mut state, &report);
        report.agents.push(ProcessAgentObservation {
            pane_id,
            session_id: 42,
            profile: Some("codex".into()),
        });
        assert!(apply_report(&mut state, &report));
        assert!(crate::state::is_agent_pane(&state, pane_id));
        assert!(!apply_report(&mut state, &report));
        // An unavailable sample does not claim the process exited.
        assert!(!apply_report(&mut state, &ResourceReport::default()));
        assert!(crate::state::is_agent_pane(&state, pane_id));
        // A stale report cannot clear a pane now attached to another session.
        report.agents[0].session_id = 41;
        report.agents[0].profile = None;
        assert!(!apply_report(&mut state, &report));
        assert!(crate::state::is_agent_pane(&state, pane_id));
        report.agents[0].session_id = 42;
        assert!(apply_report(&mut state, &report));
        assert!(!crate::state::is_agent_pane(&state, pane_id));
        assert!(!apply_report(&mut state, &report));
    }

    #[test]
    fn apply_report_writes_totals_and_pane_figures_by_pane_id() {
        let mut state = seed_state();
        let pane_id = first_pane_id(&state);
        let report = ResourceReport {
            cpu_pct: Some(12.34),
            mem_bytes: Some(3 * 1024 * 1024 * 1024),
            net_kbps: Some(4.5),
            clock_hhmm: Some("09:41".into()),
            panes: vec![PaneResources {
                pane_id,
                pid: 4242,
                cpu_pct: Some(7.5),
                mem_bytes: 512 << 20,
                process_count: 3,
            }],
            ..ResourceReport::default()
        };

        assert!(apply_report(&mut state, &report));
        assert_eq!(state.cpu_pct, Some(12.34));
        assert_eq!(state.mem_gb, Some(3.0));
        assert_eq!(state.net_kbps, Some(4.5));
        assert_eq!(state.clock_hhmm, "09:41");
        let pane = &state.panes[0][0];
        assert_eq!(pane.pid, 4242);
        assert_eq!(pane.cpu, 7.5);
        assert_eq!(pane.mem_bytes, 512 << 20);
    }

    #[test]
    fn unknown_totals_clear_the_state_rather_than_leaving_stale_numbers() {
        let mut state = seed_state();
        state.cpu_pct = Some(50.0);
        state.mem_gb = Some(2.0);
        state.panes[0][0].pid = 99;
        state.panes[0][0].mem_bytes = 1;

        let report = ResourceReport {
            clock_hhmm: None,
            ..ResourceReport::default()
        };
        assert!(apply_report(&mut state, &report));
        assert_eq!(state.cpu_pct, None);
        assert_eq!(state.mem_gb, None);
        assert_eq!(state.net_kbps, None);
        let pane = &state.panes[0][0];
        assert_eq!(
            pane.pid, 0,
            "a pane the report does not cover reads unknown"
        );
        assert_eq!(pane.mem_bytes, 0);
    }

    #[test]
    fn a_report_that_changes_nothing_visible_requests_no_rebuild() {
        let mut state = seed_state();
        let pane_id = first_pane_id(&state);
        let mut report = ResourceReport {
            cpu_pct: Some(10.0),
            mem_bytes: Some(1 << 30),
            net_kbps: Some(0.0),
            clock_hhmm: Some("10:00".into()),
            panes: vec![PaneResources {
                pane_id,
                pid: 7,
                cpu_pct: Some(1.0),
                mem_bytes: 100 << 20,
                process_count: 1,
            }],
            ..ResourceReport::default()
        };
        assert!(apply_report(&mut state, &report));
        assert!(!apply_report(&mut state, &report), "identical report");

        // Below display resolution: 10.04% still renders as 10.0.
        report.cpu_pct = Some(10.04);
        report.panes[0].mem_bytes += 1;
        assert!(!apply_report(&mut state, &report), "sub-pixel change");

        report.cpu_pct = Some(10.1);
        assert!(apply_report(&mut state, &report), "one visible tenth");

        report.clock_hhmm = Some("10:01".into());
        assert!(apply_report(&mut state, &report), "clock minute");
    }

    #[test]
    fn compact_bytes_switch_units_at_a_gib() {
        assert_eq!(format_compact_bytes(0), "0 MiB");
        assert_eq!(format_compact_bytes(412 << 20), "412 MiB");
        assert_eq!(format_compact_bytes((1 << 30) + (256 << 20)), "1.25 GiB");
    }

    #[test]
    fn process_detail_changes_only_rebuild_when_open() {
        let mut state = seed_state();
        let pane_id = state.panes[0][0].id.0;
        let mut report = ResourceReport {
            panes: vec![PaneResources {
                pane_id,
                pid: 42,
                cpu_pct: None,
                mem_bytes: 100,
                process_count: 1,
            }],
            trees: HashMap::from([(
                42,
                TreeUsage {
                    mem_bytes: 100,
                    process_count: 1,
                    processes: vec![ProcessUsage {
                        pid: 42,
                        name: "shell.exe".into(),
                        mem_bytes: 100,
                    }]
                    .into(),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        apply_report(&mut state, &report);
        report.trees.get_mut(&42).unwrap().processes = vec![ProcessUsage {
            pid: 43,
            name: "node.exe".into(),
            mem_bytes: 100,
        }]
        .into();
        assert!(!apply_report(&mut state, &report));
        state.process_details_open = true;
        report.trees.get_mut(&42).unwrap().processes = vec![ProcessUsage {
            pid: 44,
            name: "git.exe".into(),
            mem_bytes: 100,
        }]
        .into();
        assert!(apply_report(&mut state, &report));
        assert!(!apply_report(&mut state, &report));
    }

    #[test]
    fn trees_and_daemon_rows_land_in_state_for_the_sessions_panel() {
        let mut state = seed_state();
        state.sessions_stale = true;
        let mut trees = HashMap::new();
        trees.insert(
            4242,
            TreeUsage {
                cpu_pct: Some(2.0),
                mem_bytes: 300 << 20,
                process_count: 4,
                root_exe: None,
                processes: Default::default(),
            },
        );
        let report = ResourceReport {
            trees,
            listing: Some(DaemonListing {
                daemon_pid: Some(77),
                daemon_memory_rss_bytes: Some(20 << 20),
                sessions: vec![SessionSnapshot {
                    session_id: 9,
                    pane_id: 1,
                    workspace_id: 1,
                    name: None,
                    pid: Some(4242),
                    memory_rss_bytes: Some(30 << 20),
                    alive: true,
                }],
            }),
            ..ResourceReport::default()
        };
        apply_report(&mut state, &report);
        assert_eq!(state.resource_trees[&4242].process_count, 4);
        assert_eq!(state.daemon_pid, Some(77));
        assert_eq!(state.daemon_memory_rss_bytes, Some(20 << 20));
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].pid, Some(4242));
        assert!(!state.sessions_stale, "a fresh listing clears stale");

        let failed = ResourceReport {
            listing_failed: true,
            ..ResourceReport::default()
        };
        apply_report(&mut state, &failed);
        assert!(
            state.sessions_stale,
            "a failed listing marks cached rows stale"
        );
        assert!(
            state.resource_trees.is_empty(),
            "nothing sampled, nothing claimed"
        );
        assert_eq!(state.sessions.len(), 1, "the cached rows themselves stay");
    }

    #[test]
    fn unowned_tree_changes_rebuild_only_while_the_sessions_panel_is_open() {
        let mut state = seed_state();
        let mut report = ResourceReport {
            clock_hhmm: Some("10:00".into()),
            ..ResourceReport::default()
        };
        report.trees.insert(
            555,
            TreeUsage {
                cpu_pct: Some(1.0),
                mem_bytes: 10 << 20,
                process_count: 1,
                root_exe: None,
                processes: Default::default(),
            },
        );
        assert!(apply_report(&mut state, &report));
        report.trees.get_mut(&555).unwrap().mem_bytes = 200 << 20;
        assert!(
            !apply_report(&mut state, &report),
            "closed panel: no pane shows pid 555, so nothing visible changed"
        );

        state.settings_open = true;
        state.settings_section = crate::state::SettingsSection::Sessions;
        report.trees.get_mut(&555).unwrap().mem_bytes = 300 << 20;
        assert!(
            apply_report(&mut state, &report),
            "open panel renders the tree, so the change is visible"
        );
    }

    #[test]
    fn a_pane_tree_process_count_change_is_visible_in_the_status_bar() {
        let mut state = seed_state();
        let pane_id = first_pane_id(&state);
        let mut report = ResourceReport {
            panes: vec![PaneResources {
                pane_id,
                pid: 7,
                cpu_pct: Some(1.0),
                mem_bytes: 100 << 20,
                process_count: 1,
            }],
            ..ResourceReport::default()
        };
        report.trees.insert(
            7,
            TreeUsage {
                cpu_pct: Some(1.0),
                mem_bytes: 100 << 20,
                process_count: 1,
                root_exe: None,
                processes: Default::default(),
            },
        );
        assert!(apply_report(&mut state, &report));
        report.trees.get_mut(&7).unwrap().process_count = 2;
        assert!(
            apply_report(&mut state, &report),
            "the tab item shows the process count"
        );
    }
}
