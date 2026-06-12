//! Shared one-dimensional scroll animation: the browser-calibrated ease
//! curve, the wheel-distance duration/slope ramps, and the retargetable
//! [`ScrollMotion`] sampler.
//!
//! This is the single implementation behind every animated scroll in the
//! framework. The built-in smooth-scroll containers (`SmoothScroll` in
//! `crate::app`) delegate their per-axis sampling here, and app-managed
//! scroll surfaces (terminal scrollback panes registering grid-animation
//! hooks) drive their own `ScrollMotion` with the duration/slope the wheel
//! dispatch hands them on each `ScrollEvent` — so both kinds of surface
//! animate with bit-identical feel, validated by the settings-page scroll
//! regression suite.
//!
//! Everything here is pure: [`ScrollMotion::sample`] takes an injected
//! timestamp, so callers (and Phase 4's vblank-anchored clock) choose what
//! "now" means.

use std::time::{Duration, Instant};

use crate::app::ScrollTuning;

/// Below this distance (px) a retarget is a no-op and a finishing animation
/// is considered settled.
pub const SMOOTH_SCROLL_EPSILON: f32 = 0.5;
const BROWSER_WHEEL_RAMP_START_PX: f32 = 100.0;
const BROWSER_WHEEL_RAMP_END_PX: f32 = 400.0;
const BROWSER_MIN_DURATION_RATIO: f32 = 0.52;
const BROWSER_DURATION_RAMP_EXPONENT: f32 = 1.35;
const BROWSER_INITIAL_SLOPE_MIN: f32 = 0.25;
const BROWSER_INITIAL_SLOPE_MAX: f32 = 0.95;
const EASE_IN_OUT_X1: f32 = 0.42;
const EASE_IN_OUT_Y1: f32 = 0.0;
const EASE_IN_OUT_X2: f32 = 0.58;
const EASE_IN_OUT_Y2: f32 = 1.0;

/// A single-axis eased scroll animation from `start` to `target`,
/// parameterized entirely at construction so sampling is pure.
///
/// The position space is the caller's: the smooth-scroll containers use
/// element scroll offsets, the terminal uses bottom-anchored scrollback
/// pixels. Callers that need to re-anchor a motion under concurrent
/// content changes may shift `start` and `target` by the same amount;
/// the sampled displacement stays continuous in content space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMotion {
    pub start: f32,
    pub target: f32,
    pub started_at: Instant,
    pub duration: Duration,
    /// Initial slope of the ease curve (see [`browser_scroll_ease`]);
    /// carries velocity continuity across retargets.
    pub initial_slope: f32,
}

impl ScrollMotion {
    /// Sample the motion at `now`. Returns `(position, velocity, complete)`
    /// where `velocity` is in position units per second. Past the duration
    /// the position pins to `target` with zero velocity and `complete` is
    /// `true`. Pure: same `now`, same answer.
    pub fn sample(self, now: Instant) -> (f32, f32, bool) {
        let elapsed = now.saturating_duration_since(self.started_at);
        if self.duration.is_zero() {
            return (self.target, 0.0, true);
        }
        let complete = elapsed >= self.duration;
        let duration_secs = self.duration.as_secs_f32();
        let (progress, progress_velocity) = if complete {
            (1.0, 0.0)
        } else {
            browser_scroll_ease(elapsed.as_secs_f32() / duration_secs, self.initial_slope)
        };
        let position = self.start + (self.target - self.start) * progress;
        let velocity = (self.target - self.start) * progress_velocity / duration_secs;
        (position, velocity, complete)
    }

    /// Start or retarget a motion for one more wheel event, with the same
    /// semantics as the framework's container smooth scroll:
    ///
    /// - **Wheel-train compounding**: the new target is based on the
    ///   previous *target* (not the current position), so successive
    ///   notches add up instead of restarting from wherever the animation
    ///   happens to be.
    /// - The target is clamped to `[0, max]`.
    /// - A retarget that would move less than [`SMOOTH_SCROLL_EPSILON`]
    ///   from `current` is a no-op (`None`).
    /// - **Velocity continuity**: a retarget mid-flight raises the new
    ///   curve's initial slope to match the in-flight velocity
    ///   (`velocity * duration / new_delta`), never lowering it below the
    ///   distance-derived `initial_slope`.
    ///
    /// `delta` follows wheel conventions: the new target is `base - delta`,
    /// so a negative delta scrolls toward larger positions.
    pub fn retarget(
        active: Option<ScrollMotion>,
        current: f32,
        delta: f32,
        max: f32,
        now: Instant,
        duration: Duration,
        initial_slope: f32,
    ) -> Option<ScrollMotion> {
        let base = active.map(|motion| motion.target).unwrap_or(current);
        let target = (base - delta).clamp(0.0, max);

        if (target - current).abs() < SMOOTH_SCROLL_EPSILON {
            return None;
        }

        let continuity_slope = active.and_then(|motion| {
            let (_, velocity, complete) = motion.sample(now);
            if complete || duration.is_zero() {
                return None;
            }
            let new_delta = target - current;
            if new_delta.abs() < SMOOTH_SCROLL_EPSILON {
                None
            } else {
                Some(velocity * duration.as_secs_f32() / new_delta)
            }
        });
        let initial_slope =
            continuity_slope.map(|slope| slope.max(initial_slope)).unwrap_or(initial_slope);

        Some(ScrollMotion { start: current, target, started_at: now, duration, initial_slope })
    }
}

/// The browser-calibrated ease curve: a cubic bezier whose first control
/// point is raised with `initial_slope` so retargeted animations launch at
/// their inherited velocity. Returns `(progress, progress_velocity)` for a
/// normalized time `x` in `[0, 1]`.
pub fn browser_scroll_ease(x: f32, initial_slope: f32) -> (f32, f32) {
    let y1 = EASE_IN_OUT_Y1 + EASE_IN_OUT_X1 * initial_slope.clamp(-1000.0, 1000.0);
    let x = x.clamp(0.0, 1.0);
    cubic_bezier_y_and_velocity(x, EASE_IN_OUT_X1, y1, EASE_IN_OUT_X2, EASE_IN_OUT_Y2)
}

pub fn cubic_bezier_y_and_velocity(x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> (f32, f32) {
    let mut t = x;
    for _ in 0..6 {
        let current_x = cubic_bezier_axis(t, x1, x2);
        let dx = cubic_bezier_axis_derivative(t, x1, x2);
        if dx.abs() < 0.000_001 {
            break;
        }
        let next = t - (current_x - x) / dx;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        t = next;
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..8 {
        let current_x = cubic_bezier_axis(t, x1, x2);
        if (current_x - x).abs() <= 0.000_01 {
            break;
        }
        if current_x < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) * 0.5;
    }
    let y = cubic_bezier_axis(t, y1, y2);
    let dx = cubic_bezier_axis_derivative(t, x1, x2);
    let dy = cubic_bezier_axis_derivative(t, y1, y2);
    let velocity = if dx.abs() < 0.000_001 { 0.0 } else { dy / dx };
    (y, velocity)
}

pub fn cubic_bezier_axis(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * t * p1 + 3.0 * inv * t * t * p2 + t * t * t
}

pub fn cubic_bezier_axis_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * p1 + 6.0 * inv * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// The axis with the larger magnitude; ties go to the vertical axis.
pub(crate) fn dominant_delta(delta: (f32, f32)) -> f32 {
    if delta.0.abs() > delta.1.abs() {
        delta.0
    } else {
        delta.1
    }
}

/// Duration ramp calibrated against Edge wheel metrics: small deltas get
/// the full tuned duration, deltas past 400px floor at 52% of it.
pub fn browser_like_wheel_duration(delta: (f32, f32), tuning: ScrollTuning) -> Duration {
    let tuning = tuning.sanitized();
    let max_ms = tuning.smooth_scroll_duration_ms as f32;
    let min_ms = (max_ms * BROWSER_MIN_DURATION_RATIO).max(16.0);
    let distance = dominant_delta(delta).abs();
    let duration_ms = if distance <= BROWSER_WHEEL_RAMP_START_PX {
        max_ms
    } else if distance >= BROWSER_WHEEL_RAMP_END_PX {
        min_ms
    } else {
        let t = (distance - BROWSER_WHEEL_RAMP_START_PX)
            / (BROWSER_WHEEL_RAMP_END_PX - BROWSER_WHEEL_RAMP_START_PX);
        min_ms + (max_ms - min_ms) * (1.0 - t).powf(BROWSER_DURATION_RAMP_EXPONENT)
    };
    Duration::from_millis(duration_ms.round() as u64)
}

/// Initial-slope ramp: larger wheel deltas start more front-loaded, again
/// matching the measured Edge behavior.
pub fn browser_like_initial_slope(delta: (f32, f32)) -> f32 {
    let distance = dominant_delta(delta).abs();
    if distance <= BROWSER_WHEEL_RAMP_START_PX {
        return BROWSER_INITIAL_SLOPE_MIN;
    }
    if distance >= BROWSER_WHEEL_RAMP_END_PX {
        return BROWSER_INITIAL_SLOPE_MAX;
    }
    let t = (distance - BROWSER_WHEEL_RAMP_START_PX)
        / (BROWSER_WHEEL_RAMP_END_PX - BROWSER_WHEEL_RAMP_START_PX);
    BROWSER_INITIAL_SLOPE_MIN + (BROWSER_INITIAL_SLOPE_MAX - BROWSER_INITIAL_SLOPE_MIN) * t.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_eases_to_exact_target() {
        // Parity with the pre-refactor `SmoothScroll` sampler: slope 0 over
        // 100ms reaches ~50% at the midpoint and lands exactly on target.
        let now = Instant::now();
        let motion = ScrollMotion {
            start: 0.0,
            target: 100.0,
            started_at: now,
            duration: Duration::from_millis(100),
            initial_slope: 0.0,
        };

        let (mid, _, done) = motion.sample(now + Duration::from_millis(50));
        assert!((mid - 50.0).abs() < 0.1);
        assert!(!done);

        let (end, velocity, done) = motion.sample(now + Duration::from_millis(100));
        assert_eq!(end, 100.0);
        assert_eq!(velocity, 0.0);
        assert!(done);
    }

    #[test]
    fn zero_duration_motion_completes_instantly_at_target() {
        let now = Instant::now();
        let motion = ScrollMotion {
            start: 10.0,
            target: 90.0,
            started_at: now,
            duration: Duration::ZERO,
            initial_slope: 0.25,
        };
        assert_eq!(motion.sample(now), (90.0, 0.0, true));
    }

    #[test]
    fn motion_velocity_is_positive_mid_flight_toward_a_larger_target() {
        let now = Instant::now();
        let motion = ScrollMotion {
            start: 0.0,
            target: 200.0,
            started_at: now,
            duration: Duration::from_millis(180),
            initial_slope: 0.25,
        };
        let (_, velocity, complete) = motion.sample(now + Duration::from_millis(90));
        assert!(velocity > 0.0);
        assert!(!complete);
    }

    #[test]
    fn sample_is_pure_in_the_injected_timestamp() {
        // Phase 4 readiness: the sampler must answer identically for the
        // same injected timestamp regardless of wall-clock time.
        let now = Instant::now();
        let motion = ScrollMotion {
            start: 0.0,
            target: 120.0,
            started_at: now,
            duration: Duration::from_millis(180),
            initial_slope: 0.25,
        };
        let probe = now + Duration::from_millis(67);
        assert_eq!(motion.sample(probe), motion.sample(probe));
    }

    #[test]
    fn retarget_compounds_wheel_ticks_from_pending_target() {
        // Mirrors `smooth_scroll_compounds_wheel_ticks_from_pending_target`
        // in crate::app: the 1-D retarget must keep the container
        // semantics exactly.
        let now = Instant::now();
        let first =
            ScrollMotion::retarget(None, 0.0, -80.0, 500.0, now, Duration::from_millis(80), 0.25)
                .expect("first wheel tick should start the motion");

        let second = ScrollMotion::retarget(
            Some(first),
            12.0,
            -80.0,
            500.0,
            now + Duration::from_millis(20),
            Duration::from_millis(80),
            0.25,
        )
        .expect("second wheel tick should extend the target");

        assert_eq!(first.target, 80.0);
        assert_eq!(second.start, 12.0);
        assert_eq!(second.target, 160.0);
        assert!(second.initial_slope > 0.25, "retarget must preserve in-flight velocity");
    }

    #[test]
    fn retarget_clamps_target_to_bounds() {
        let now = Instant::now();
        let up =
            ScrollMotion::retarget(None, 10.0, 300.0, 500.0, now, Duration::from_millis(120), 0.25)
                .expect("a clamped-to-zero target still differs from current");
        assert_eq!(up.target, 0.0, "targets below zero clamp to the live edge");

        let down = ScrollMotion::retarget(
            None,
            450.0,
            -300.0,
            500.0,
            now,
            Duration::from_millis(120),
            0.25,
        )
        .expect("a clamped-to-max target still differs from current");
        assert_eq!(down.target, 500.0, "targets past max clamp to the top of scrollback");
    }

    #[test]
    fn retarget_within_epsilon_is_a_no_op() {
        let now = Instant::now();
        assert!(ScrollMotion::retarget(
            None,
            100.0,
            -0.25,
            500.0,
            now,
            Duration::from_millis(120),
            0.25,
        )
        .is_none());
        // At the boundary, a delta that clamps back onto the current
        // position is also a no-op.
        assert!(ScrollMotion::retarget(
            None,
            0.0,
            50.0,
            500.0,
            now,
            Duration::from_millis(120),
            0.25,
        )
        .is_none());
    }

    #[test]
    fn completed_motion_contributes_no_continuity_slope() {
        let now = Instant::now();
        let finished = ScrollMotion {
            start: 0.0,
            target: 80.0,
            started_at: now,
            duration: Duration::from_millis(80),
            initial_slope: 0.9,
        };
        let next = ScrollMotion::retarget(
            Some(finished),
            80.0,
            -80.0,
            500.0,
            now + Duration::from_millis(200),
            Duration::from_millis(80),
            0.25,
        )
        .expect("a fresh notch after completion starts a new motion");
        assert_eq!(
            next.initial_slope, 0.25,
            "a settled animation must not inflate the next motion's launch velocity"
        );
    }

    #[test]
    fn one_notch_at_8ms_cadence_yields_at_least_12_distinct_positions() {
        // H4 at the unit level: the default 180ms duration sampled on the
        // 8ms animation cadence must produce >= 12 distinct positions, so
        // a single wheel notch renders as motion rather than a jump.
        let now = Instant::now();
        let motion = ScrollMotion::retarget(
            None,
            0.0,
            -120.0,
            10_000.0,
            now,
            browser_like_wheel_duration((0.0, -120.0), ScrollTuning::default()),
            browser_like_initial_slope((0.0, -120.0)),
        )
        .expect("a notch-sized delta must animate");

        let mut positions: Vec<f32> = Vec::new();
        let mut tick = now;
        loop {
            tick += Duration::from_millis(8);
            let (position, _, complete) = motion.sample(tick);
            if positions.last() != Some(&position) {
                positions.push(position);
            }
            if complete {
                break;
            }
        }
        assert!(
            positions.len() >= 12,
            "expected >= 12 distinct sampled positions per notch, got {}",
            positions.len()
        );
        assert_eq!(*positions.last().unwrap(), 120.0, "the motion must land exactly on target");
    }

    #[test]
    fn browser_like_wheel_duration_matches_edge_wheel_metrics() {
        let tuning = ScrollTuning::default();
        assert_eq!(browser_like_wheel_duration((0.0, -100.0), tuning), Duration::from_millis(180));
        assert_eq!(browser_like_wheel_duration((0.0, -200.0), tuning), Duration::from_millis(144));
        assert_eq!(browser_like_wheel_duration((0.0, -400.0), tuning), Duration::from_millis(94));
    }

    #[test]
    fn browser_like_initial_slope_ramps_with_distance() {
        let small = browser_like_initial_slope((0.0, -100.0));
        let mid = browser_like_initial_slope((0.0, -250.0));
        let large = browser_like_initial_slope((0.0, -400.0));
        assert_eq!(small, BROWSER_INITIAL_SLOPE_MIN);
        assert_eq!(large, BROWSER_INITIAL_SLOPE_MAX);
        assert!(small < mid && mid < large);
    }

    #[test]
    fn browser_scroll_ease_hits_curve_endpoints_and_is_monotonic() {
        let slope = browser_like_initial_slope((0.0, -100.0));
        let (start, _) = browser_scroll_ease(0.0, slope);
        let (end, _) = browser_scroll_ease(1.0, slope);
        assert!(start.abs() < 0.001);
        assert!((end - 1.0).abs() < 0.001);

        let mut previous = 0.0;
        for step in 1..=20 {
            let (progress, _) = browser_scroll_ease(step as f32 / 20.0, slope);
            assert!(progress >= previous - 0.0001, "ease curve must be monotonic");
            previous = progress;
        }
    }
}
