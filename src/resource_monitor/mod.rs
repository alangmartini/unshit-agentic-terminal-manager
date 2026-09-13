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
//!    [`ROOTS_REFRESH_INTERVAL`]),
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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use unshit::app::{EventSink, ExternalEvent};

use crate::pty::SessionLister;
use crate::state::{AppState, MutexExt, SessionSnapshot, SharedState};
use cpu::{CpuTracker, ProcSample};
use telemetry::ResourceEventRecord;

/// How often usage is sampled. One second matches what people expect from
/// a task manager and keeps CPU percentages readable.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Root pids do not change for a session's lifetime, so the daemon is asked
/// again only when the UI's session set changes or this long has passed.
pub const ROOTS_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Ticks folded into one `resource.summary` telemetry record.
const SUMMARY_EVERY_TICKS: u32 = 60;

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
    pub profile: Option<&'static str>,
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
                observation.profile,
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

/// Start the sampler thread. Returns immediately. Safe before the event
/// loop exists: `sink` may still be empty, in which case the tick simply
/// does not request a rebuild and the next one does.
pub fn start(shared: SharedState, sink: Arc<OnceLock<EventSink>>) {
    let spawned = std::thread::Builder::new()
        .name("resource-monitor".into())
        .spawn(move || {
            // The start record is a synchronous file write; it belongs on
            // this thread, not on the startup path that spawned it.
            let mut started = ResourceEventRecord::new("resource.monitor_started", "info");
            started.interval_ms = Some(SAMPLE_INTERVAL.as_millis() as u64);
            started.logical_cpus = Some(platform::logical_cpus());
            started.platform_supported = Some(cfg!(windows));
            telemetry::record(&started);

            let mut monitor = Monitor::new();
            loop {
                let tick_started = Instant::now();
                let report = monitor.tick(&shared);
                let changed = {
                    let mut guard = shared.lock_recover();
                    apply_report(&mut guard, &report)
                };
                if changed {
                    if let Some(sink) = sink.get() {
                        let _ = sink.send(ExternalEvent::RequestRebuild);
                    }
                }
                monitor.after_tick(tick_started.elapsed(), &report);
                std::thread::sleep(
                    SAMPLE_INTERVAL
                        .saturating_sub(tick_started.elapsed())
                        .max(Duration::from_millis(200)),
                );
            }
        });
    if let Err(err) = spawned {
        let mut record = ResourceEventRecord::new("resource.monitor_spawn_failed", "warn");
        record.reason = Some(err.to_string());
        telemetry::record(&record);
    }
}

/// One session's shell pid and, when this UI owns the session, its pane.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionRoot {
    pid: u32,
    session_id: u64,
    pane_id: Option<u32>,
}

#[derive(Debug, Default)]
struct RootsCache {
    sessions: Vec<SessionRoot>,
    daemon_pid: Option<u32>,
    /// Sorted `(pane_id, session_id)` the cache was built for.
    owned: Vec<(u32, u64)>,
    refreshed_at: Option<Instant>,
    /// Whether the last daemon list failed; transitions are logged once.
    failing: bool,
    /// Whether the daemon has ever been listed; until then totals are
    /// unknown rather than "UI only".
    ever_listed: bool,
    /// Rows from the latest successful list, handed to the next report
    /// once so the Sessions panel follows the daemon.
    fresh: Option<DaemonListing>,
}

struct Monitor {
    tracker: CpuTracker,
    roots: RootsCache,
    logical_cpus: usize,
    last_tick: Option<(Instant, u64)>,
    /// Where lifecycle and summary records go; tests substitute a no-op so
    /// they do not write to the profile's telemetry file.
    emit: fn(&ResourceEventRecord),
    // Summary telemetry accumulators.
    ticks: u32,
    max_tick: Duration,
    total_tick: Duration,
    list_failures: u32,
    last_processes: usize,
    last_unsampled: usize,
}

impl Monitor {
    fn new() -> Self {
        Self::with_emitter(telemetry::record)
    }

    fn with_emitter(emit: fn(&ResourceEventRecord)) -> Self {
        Self {
            tracker: CpuTracker::new(),
            roots: RootsCache::default(),
            logical_cpus: platform::logical_cpus(),
            last_tick: None,
            emit,
            ticks: 0,
            max_tick: Duration::ZERO,
            total_tick: Duration::ZERO,
            list_failures: 0,
            last_processes: 0,
            last_unsampled: 0,
        }
    }

    fn tick(&mut self, shared: &SharedState) -> ResourceReport {
        let (lister, owned) = {
            let guard = shared.lock_recover();
            let mut owned: Vec<(u32, u64)> = guard.pty_manager.sessions_iter().collect();
            owned.sort_unstable();
            (guard.pty_manager.session_lister(), owned)
        };
        self.refresh_roots(lister, owned);

        let now_wall = Instant::now();
        let now_100ns = platform::now_100ns();
        let output_bytes = crate::pty::output_bytes_total();
        let mut report = ResourceReport {
            clock_hhmm: platform::local_time_hhmm(),
            listing: self.roots.fresh.take(),
            listing_failed: self.roots.failing,
            ..ResourceReport::default()
        };

        if let Some((prev_wall, prev_bytes)) = self.last_tick {
            let secs = now_wall.duration_since(prev_wall).as_secs_f32();
            if secs > 0.0 {
                report.net_kbps =
                    Some(output_bytes.saturating_sub(prev_bytes) as f32 / secs / 1024.0);
            }
        }

        if self.roots.ever_listed {
            self.sample_trees(&mut report, now_100ns, now_wall);
        }

        self.last_tick = Some((now_wall, output_bytes));
        report
    }

    fn refresh_roots(&mut self, lister: Option<SessionLister>, owned: Vec<(u32, u64)>) {
        let Some(lister) = lister else {
            self.note_list_outcome(Err(std::io::ErrorKind::NotConnected));
            return;
        };
        let stale = self
            .roots
            .refreshed_at
            .is_none_or(|at| at.elapsed() >= ROOTS_REFRESH_INTERVAL);
        if !stale && self.roots.owned == owned && !self.roots.failing {
            return;
        }
        match lister.list() {
            Ok(snapshot) => {
                let pane_for: HashMap<u64, u32> = owned
                    .iter()
                    .map(|&(pane, session)| (session, pane))
                    .collect();
                self.roots.sessions = snapshot
                    .sessions
                    .iter()
                    .filter(|s| s.alive)
                    .filter_map(|s| {
                        Some(SessionRoot {
                            pid: s.pid?,
                            session_id: s.id,
                            pane_id: pane_for.get(&s.id).copied(),
                        })
                    })
                    .collect();
                self.roots.daemon_pid = snapshot.daemon_pid;
                self.roots.fresh = Some(DaemonListing {
                    daemon_pid: snapshot.daemon_pid,
                    daemon_memory_rss_bytes: snapshot.daemon_memory_rss_bytes,
                    sessions: snapshot
                        .sessions
                        .iter()
                        .map(|s| SessionSnapshot {
                            session_id: s.id,
                            pane_id: s.pane_id,
                            workspace_id: s.workspace_id,
                            name: s.name.clone(),
                            pid: s.pid,
                            memory_rss_bytes: s.memory_rss_bytes,
                            alive: s.alive,
                        })
                        .collect(),
                });
                self.roots.owned = owned;
                self.roots.refreshed_at = Some(Instant::now());
                self.roots.ever_listed = true;
                self.note_list_outcome(Ok(()));
            }
            Err(err) => self.note_list_outcome(Err(err.kind())),
        }
    }

    fn note_list_outcome(&mut self, outcome: Result<(), std::io::ErrorKind>) {
        match outcome {
            Ok(()) => {
                if self.roots.failing {
                    (self.emit)(&ResourceEventRecord::new("resource.list_recovered", "info"));
                }
                self.roots.failing = false;
            }
            Err(kind) => {
                self.list_failures = self.list_failures.saturating_add(1);
                if !self.roots.failing {
                    let mut record = ResourceEventRecord::new("resource.list_failed", "warn");
                    record.reason = Some(format!("{kind:?}"));
                    (self.emit)(&record);
                }
                self.roots.failing = true;
            }
        }
    }

    fn sample_trees(&mut self, report: &mut ResourceReport, now_100ns: u64, now_wall: Instant) {
        let Some((table, image_names)) = platform::enumerate_processes_named() else {
            self.last_processes = 0;
            self.last_unsampled = 0;
            return;
        };

        let ui_pid = std::process::id();
        let mut roots: HashSet<u32> = HashSet::new();
        roots.insert(ui_pid);
        roots.extend(self.roots.daemon_pid);
        roots.extend(self.roots.sessions.iter().map(|s| s.pid));

        // Pass one finds the candidates; pass two rejects stale parent
        // links now that creation times are known for every candidate.
        let candidates = tree::attribute(&table, &roots, |_| None);
        let samples: HashMap<u32, ProcSample> = candidates
            .keys()
            .filter_map(|&pid| platform::sample_process(pid).map(|s| (pid, s)))
            .collect();
        let owner = tree::attribute(&table, &roots, |pid| {
            samples.get(&pid).map(|s| s.creation_100ns)
        });

        if !self.roots.failing {
            report.agents = observe_agents(
                &self.roots.sessions,
                &table,
                &owner,
                &image_names,
                platform::process_command_line,
            );
        }

        let attributed: Vec<ProcSample> = owner
            .keys()
            .filter_map(|pid| samples.get(pid).copied())
            .collect();
        self.last_processes = owner.len();
        self.last_unsampled = owner.len().saturating_sub(attributed.len());

        let deltas = self.tracker.advance(now_100ns, &attributed);
        let wall_delta_100ns = self
            .last_tick
            .map(|(prev, _)| now_wall.duration_since(prev).as_nanos() as u64 / 100)
            .unwrap_or(0);

        let mut per_root: HashMap<u32, (u64, u64, u32)> = HashMap::new();
        for sample in &attributed {
            let root = owner[&sample.pid];
            let entry = per_root.entry(root).or_default();
            entry.0 += deltas.as_ref().map_or(0, |d| d[&sample.pid]);
            entry.1 += sample.working_set_bytes;
            entry.2 += 1;
        }

        let total_cpu: u64 = per_root.values().map(|v| v.0).sum();
        let total_mem: u64 = per_root.values().map(|v| v.1).sum();
        report.mem_bytes = Some(total_mem);
        report.cpu_pct = deltas
            .as_ref()
            .map(|_| cpu::percent(total_cpu, wall_delta_100ns, self.logical_cpus));

        report.trees = per_root
            .iter()
            .map(|(&root, &(cpu_delta, mem, count))| {
                let usage = TreeUsage {
                    cpu_pct: deltas
                        .as_ref()
                        .map(|_| cpu::percent(cpu_delta, wall_delta_100ns, self.logical_cpus)),
                    mem_bytes: mem,
                    process_count: count,
                    root_exe: image_names.get(&root).cloned(),
                };
                (root, usage)
            })
            .collect();
        report.panes = self
            .roots
            .sessions
            .iter()
            .filter_map(|session| {
                let pane_id = session.pane_id?;
                let usage = report.trees.get(&session.pid)?;
                Some(PaneResources {
                    pane_id,
                    pid: session.pid,
                    cpu_pct: usage.cpu_pct,
                    mem_bytes: usage.mem_bytes,
                    process_count: usage.process_count,
                })
            })
            .collect();
    }

    fn after_tick(&mut self, took: Duration, report: &ResourceReport) {
        self.ticks += 1;
        self.max_tick = self.max_tick.max(took);
        self.total_tick += took;
        if self.ticks < SUMMARY_EVERY_TICKS {
            return;
        }
        let mut record = ResourceEventRecord::new("resource.summary", "info");
        record.ticks = Some(self.ticks);
        record.sessions = Some(self.roots.sessions.len());
        record.processes = Some(self.last_processes);
        record.unsampled = Some(self.last_unsampled);
        record.max_tick_ms = Some(self.max_tick.as_secs_f64() * 1000.0);
        record.mean_tick_ms = Some(self.total_tick.as_secs_f64() * 1000.0 / f64::from(self.ticks));
        record.list_failures = Some(self.list_failures);
        record.cpu_pct = report.cpu_pct;
        record.mem_bytes = report.mem_bytes;
        (self.emit)(&record);

        self.ticks = 0;
        self.max_tick = Duration::ZERO;
        self.total_tick = Duration::ZERO;
        self.list_failures = 0;
    }
}

/// Reuse the monitor's attributed tree. Prefer the outermost harness when
/// it launches another agent, and ignore processes from other UI sessions.
fn observe_agents(
    sessions: &[SessionRoot],
    table: &[tree::ProcessRecord],
    owner: &HashMap<u32, u32>,
    image_names: &HashMap<u32, String>,
    mut command_line: impl FnMut(u32) -> Option<String>,
) -> Vec<ProcessAgentObservation> {
    use crate::agents::process::{classify_process, needs_command_line};
    let parents: HashMap<u32, u32> = table.iter().map(|p| (p.pid, p.parent_pid)).collect();
    let mut observations = Vec::new();
    for session in sessions.iter().filter(|s| s.pane_id.is_some()) {
        let mut pids: Vec<u32> = table
            .iter()
            .filter(|p| owner.get(&p.pid) == Some(&session.pid))
            .map(|p| p.pid)
            .collect();
        pids.sort_unstable_by_key(|&pid| {
            let mut at = pid;
            let mut depth = 0;
            while at != session.pid && depth < table.len() {
                let Some(&parent) = parents.get(&at) else {
                    break;
                };
                at = parent;
                depth += 1;
            }
            (depth, pid)
        });
        let mut profile = None;
        let mut unknown = false;
        for pid in pids {
            let Some(image) = image_names.get(&pid) else {
                unknown = true;
                continue;
            };
            let command = if needs_command_line(image) {
                let command = command_line(pid);
                unknown |= command.is_none();
                command
            } else {
                None
            };
            if let Some(agent) = classify_process(image, command.as_deref()) {
                profile = Some(agent.id);
                break;
            }
        }
        // Failure to read an interpreter does not prove an agent exited.
        if profile.is_some() || !unknown {
            observations.push(ProcessAgentObservation {
                pane_id: session.pane_id.unwrap(),
                session_id: session.session_id,
                profile,
            });
        }
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;

    fn first_pane_id(state: &AppState) -> u32 {
        state.panes[0][0].id.0
    }

    #[test]
    fn process_scan_detects_wrapped_agents_in_owned_trees_and_handles_exit() {
        use tree::ProcessRecord;
        let sessions = vec![
            SessionRoot {
                pid: 10,
                pane_id: Some(1),
                session_id: 100,
            },
            SessionRoot {
                pid: 20,
                pane_id: None,
                session_id: 200,
            },
        ];
        let mut table = vec![
            ProcessRecord {
                pid: 10,
                parent_pid: 1,
            },
            ProcessRecord {
                pid: 11,
                parent_pid: 10,
            },
            ProcessRecord {
                pid: 12,
                parent_pid: 11,
            },
            ProcessRecord {
                pid: 20,
                parent_pid: 1,
            },
            ProcessRecord {
                pid: 21,
                parent_pid: 20,
            },
        ];
        let names = HashMap::from([
            (10, "pwsh.exe".into()),
            (11, "node.exe".into()),
            (12, "claude.exe".into()),
            (20, "pwsh.exe".into()),
            (21, "node.exe".into()),
        ]);
        let roots = HashSet::from([10, 20]);
        let owner = tree::attribute(&table, &roots, |_| None);
        let observed = observe_agents(&sessions, &table, &owner, &names, |pid| {
            assert_eq!(pid, 11, "never query unrelated sessions");
            Some(r#"node "C:\Users\Test User\node_modules\@github\copilot\npm-loader.js""#.into())
        });
        assert_eq!(
            observed,
            vec![ProcessAgentObservation {
                pane_id: 1,
                session_id: 100,
                profile: Some("copilot"),
            }],
            "outer harness wins over the nested agent"
        );
        table.retain(|p| p.pid != 12);
        let owner = tree::attribute(&table, &roots, |_| None);
        assert!(
            observe_agents(&sessions, &table, &owner, &names, |_| None).is_empty(),
            "an unreadable runtime leaves membership unknown"
        );
        table.retain(|p| p.pid != 11);
        let owner = tree::attribute(&table, &roots, |_| None);
        let observed = observe_agents(&sessions, &table, &owner, &names, |_| panic!("no runtime"));
        assert_eq!(
            observed[0].profile, None,
            "shell remains after the agent exits"
        );
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
            profile: Some("codex"),
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

    fn quiet_monitor() -> Monitor {
        Monitor::with_emitter(|_| {})
    }

    #[test]
    fn a_missing_daemon_is_a_counted_failure_that_is_logged_once() {
        static EMITTED: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
        fn capture(record: &ResourceEventRecord) {
            EMITTED.lock().unwrap().push(record.event);
        }
        EMITTED.lock().unwrap().clear();

        let mut monitor = Monitor::with_emitter(capture);
        monitor.refresh_roots(None, vec![]);
        monitor.refresh_roots(None, vec![]);
        assert!(monitor.roots.failing);
        assert_eq!(monitor.list_failures, 2);
        assert!(!monitor.roots.ever_listed);
        assert_eq!(
            *EMITTED.lock().unwrap(),
            vec!["resource.list_failed"],
            "the transition is logged, not every tick"
        );

        monitor.note_list_outcome(Ok(()));
        assert!(!monitor.roots.failing);
        assert_eq!(
            *EMITTED.lock().unwrap(),
            vec!["resource.list_failed", "resource.list_recovered"]
        );
    }

    #[test]
    fn tick_without_a_daemon_reports_only_clock_and_throughput() {
        let shared: SharedState = Arc::new(std::sync::Mutex::new(seed_state()));
        let mut monitor = quiet_monitor();
        let first = monitor.tick(&shared);
        assert_eq!(first.cpu_pct, None);
        assert_eq!(first.mem_bytes, None);
        assert_eq!(first.net_kbps, None, "no previous tick to diff against");
        assert!(first.panes.is_empty());
        std::thread::sleep(Duration::from_millis(2));
        let second = monitor.tick(&shared);
        assert_eq!(second.cpu_pct, None, "still no daemon listing");
        assert!(second.net_kbps.is_some(), "throughput needs no daemon");
    }

    #[test]
    fn summary_is_emitted_every_sixty_ticks_and_resets() {
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn count(record: &ResourceEventRecord) {
            if record.event == "resource.summary" {
                COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        COUNT.store(0, std::sync::atomic::Ordering::Relaxed);

        let mut monitor = Monitor::with_emitter(count);
        let report = ResourceReport::default();
        for _ in 0..SUMMARY_EVERY_TICKS - 1 {
            monitor.after_tick(Duration::from_millis(3), &report);
        }
        assert_eq!(COUNT.load(std::sync::atomic::Ordering::Relaxed), 0);
        monitor.after_tick(Duration::from_millis(9), &report);
        assert_eq!(COUNT.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(monitor.ticks, 0, "accumulators reset after a summary");
        assert_eq!(monitor.max_tick, Duration::ZERO);
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
