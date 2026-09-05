//! CPU-time bookkeeping across ticks.
//!
//! A process reports cumulative kernel+user time; a percentage needs the
//! difference between two ticks. Baselines are keyed by `(pid, creation
//! time)` so a pid handed to a new process after the old one exited does
//! not inherit the old baseline and show a negative or absurd delta.
//!
//! A process seen for the first time counts its whole lifetime only if it
//! was born after the previous tick (a short-lived `git` an agent spawned
//! and reaped inside one interval is exactly the load a user asks about).
//! A first-seen process that predates the previous tick has no usable
//! baseline and contributes zero this tick.

use std::collections::HashMap;

/// One process at one instant. Times are FILETIME 100ns ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcSample {
    pub pid: u32,
    pub creation_100ns: u64,
    /// Kernel + user time consumed so far.
    pub cpu_100ns: u64,
    pub working_set_bytes: u64,
}

#[derive(Debug, Default)]
pub struct CpuTracker {
    baselines: HashMap<(u32, u64), u64>,
    prev_tick_100ns: Option<u64>,
}

impl CpuTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record this tick's samples and return each process's CPU time
    /// consumed since the previous tick. `None` on the very first tick,
    /// when there is nothing to diff against. `now_100ns` is the wall clock
    /// in the same units as `creation_100ns`.
    pub fn advance(&mut self, now_100ns: u64, samples: &[ProcSample]) -> Option<HashMap<u32, u64>> {
        let prev_tick = self.prev_tick_100ns;
        let mut deltas = HashMap::with_capacity(samples.len());
        let mut next: HashMap<(u32, u64), u64> = HashMap::with_capacity(samples.len());

        for sample in samples {
            let key = (sample.pid, sample.creation_100ns);
            let delta = match (self.baselines.get(&key), prev_tick) {
                (Some(&baseline), _) => sample.cpu_100ns.saturating_sub(baseline),
                (None, Some(prev)) if sample.creation_100ns >= prev => sample.cpu_100ns,
                _ => 0,
            };
            deltas.insert(sample.pid, delta);
            next.insert(key, sample.cpu_100ns);
        }

        self.baselines = next;
        self.prev_tick_100ns = Some(now_100ns);
        prev_tick.map(|_| deltas)
    }
}

/// Machine-wide percentage for `cpu_delta_100ns` of work over
/// `wall_delta_100ns` of time on `logical_cpus` cores, clamped to 100.
pub fn percent(cpu_delta_100ns: u64, wall_delta_100ns: u64, logical_cpus: usize) -> f32 {
    if wall_delta_100ns == 0 || logical_cpus == 0 {
        return 0.0;
    }
    let capacity = wall_delta_100ns as f64 * logical_cpus as f64;
    ((cpu_delta_100ns as f64 / capacity) * 100.0).clamp(0.0, 100.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, creation: u64, cpu: u64) -> ProcSample {
        ProcSample {
            pid,
            creation_100ns: creation,
            cpu_100ns: cpu,
            working_set_bytes: 1,
        }
    }

    #[test]
    fn first_tick_has_no_deltas_but_records_baselines() {
        let mut tracker = CpuTracker::new();
        assert!(tracker.advance(1_000, &[sample(1, 10, 500)]).is_none());
        let deltas = tracker.advance(2_000, &[sample(1, 10, 800)]).unwrap();
        assert_eq!(deltas[&1], 300);
    }

    #[test]
    fn a_process_born_inside_the_interval_counts_its_whole_life() {
        let mut tracker = CpuTracker::new();
        tracker.advance(1_000, &[]);
        let deltas = tracker.advance(2_000, &[sample(7, 1_500, 250)]).unwrap();
        assert_eq!(deltas[&7], 250);
    }

    #[test]
    fn a_first_seen_process_older_than_the_last_tick_contributes_nothing() {
        let mut tracker = CpuTracker::new();
        tracker.advance(1_000, &[]);
        // Existed since 200 but was only attributed to us now (a new
        // session appeared); its lifetime CPU is not this tick's load.
        let deltas = tracker.advance(2_000, &[sample(7, 200, 900_000)]).unwrap();
        assert_eq!(deltas[&7], 0);
        let deltas = tracker.advance(3_000, &[sample(7, 200, 900_100)]).unwrap();
        assert_eq!(deltas[&7], 100, "tracked normally from the next tick on");
    }

    #[test]
    fn pid_reuse_does_not_inherit_the_old_baseline() {
        let mut tracker = CpuTracker::new();
        tracker.advance(1_000, &[sample(9, 10, 5_000)]);
        // Same pid, new creation time after the previous tick: a fresh
        // process, so its own 40 ticks count and 5_000 is not subtracted.
        let deltas = tracker.advance(2_000, &[sample(9, 1_200, 40)]).unwrap();
        assert_eq!(deltas[&9], 40);
    }

    #[test]
    fn exited_processes_drop_out_of_the_baselines() {
        let mut tracker = CpuTracker::new();
        tracker.advance(1_000, &[sample(1, 10, 5), sample(2, 10, 5)]);
        let deltas = tracker.advance(2_000, &[sample(1, 10, 6)]).unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(tracker.baselines.len(), 1);
    }

    #[test]
    fn percent_is_machine_wide_and_clamped() {
        // 1s of CPU over 1s of wall on 4 cores = 25%.
        assert_eq!(percent(10_000_000, 10_000_000, 4), 25.0);
        assert_eq!(percent(50_000_000, 10_000_000, 4), 100.0);
        assert_eq!(percent(5, 0, 4), 0.0);
        assert_eq!(percent(5, 10, 0), 0.0);
    }
}
