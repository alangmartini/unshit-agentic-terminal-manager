//! The background sampler thread: tracks session roots, walks each root's
//! process tree once a second and turns the samples into a [`ResourceReport`]
//! (see the module-level docs in `resource_monitor` for the full pipeline).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use unshit::app::{EventSink, ExternalEvent};

use crate::pty::SessionLister;
use crate::state::{MutexExt, SessionSnapshot, SharedState};

use super::cpu::{CpuTracker, ProcSample};
use super::telemetry::ResourceEventRecord;
use super::{
    apply_report, cpu, platform, telemetry, tree, DaemonListing, PaneResources, ProcessUsage,
    ResourceReport, TreeUsage,
};

/// How often usage is sampled. One second matches what people expect from
/// a task manager and keeps CPU percentages readable.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Root pids do not change for a session's lifetime, so the daemon is asked
/// again only when the UI's session set changes or this long has passed.
pub const ROOTS_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Ticks folded into one `resource.summary` telemetry record.
const SUMMARY_EVERY_TICKS: u32 = 60;

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
            started.platform_supported = Some(cfg!(any(windows, target_os = "macos")));
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
        let mut members: HashMap<u32, Vec<ProcessUsage>> = HashMap::new();
        for sample in &attributed {
            let root = owner[&sample.pid];
            let entry = per_root.entry(root).or_default();
            entry.0 += deltas.as_ref().map_or(0, |d| d[&sample.pid]);
            entry.1 += sample.working_set_bytes;
            entry.2 += 1;
            members.entry(root).or_default().push(ProcessUsage {
                pid: sample.pid,
                name: image_names
                    .get(&sample.pid)
                    .cloned()
                    .unwrap_or_else(|| "Unknown process".into()),
                mem_bytes: sample.working_set_bytes,
            });
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
                    processes: members.remove(&root).unwrap_or_default().into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;

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

    #[cfg(windows)]
    #[test]
    fn sampled_process_members_match_tree_totals() {
        let mut monitor = Monitor::with_emitter(|_| {});
        let mut report = ResourceReport::default();
        monitor.sample_trees(&mut report, platform::now_100ns(), Instant::now());
        let tree = &report.trees[&std::process::id()];
        assert!(!tree.processes.is_empty());
        assert_eq!(tree.process_count as usize, tree.processes.len());
        assert_eq!(
            tree.mem_bytes,
            tree.processes.iter().map(|p| p.mem_bytes).sum::<u64>()
        );
        assert!(tree
            .processes
            .iter()
            .any(|p| p.pid == std::process::id() && !p.name.is_empty()));
    }
}
