//! One persistent, deadline-extended animation waker for the whole app.
//!
//! This is the Timer-fallback tick source for surfaces without a
//! vsync-paced present mode (no Fifo). On the default vsync-paced path
//! the renderer's blocking swapchain acquire anchors animation frames to
//! the display clock, [`AnimationWaker::extend_until`] is never called,
//! and the lazily-spawned thread never exists. Only when the app falls
//! back to timer pacing do animation producers (container smooth scroll,
//! grid-animation hooks) share this waker, whose tick interval is the
//! display's true refresh period.
//!
//! The previous design spawned one short-lived waker thread per wheel
//! notch; a fast wheel spin stacked a dozen threads all sleeping on the
//! same interval. This module replaces them with a single thread for the
//! app lifetime that ticks while a deadline is in the future and parks on
//! a condvar otherwise; `extend_until` only ever moves the deadline
//! forward.
//!
//! Each tick feeds the existing consumer path:
//! [`ExternalEvent::RequestAnimationFrame`] into the event channel, then
//! `EventLoopProxy::wake_up`. The tick thread's sleep runs off the winit
//! event-loop thread, so it does not violate the "event loop never
//! blocks" gate.

use std::sync::{Arc, Condvar, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use crate::event_sink::ExternalEvent;
use winit::event_loop::EventLoopProxy;

pub struct AnimationWaker {
    shared: Arc<WakerShared>,
    event_tx: flume::Sender<ExternalEvent>,
    proxy_cell: Arc<OnceLock<EventLoopProxy>>,
    interval: Duration,
    /// The thread is spawned lazily on the first `extend_until` so apps
    /// that never animate never pay for it.
    spawn: Once,
}

struct WakerShared {
    state: Mutex<WakerState>,
    condvar: Condvar,
}

#[derive(Default)]
struct WakerState {
    /// The instant past which the waker stops ticking and parks.
    /// `None` while parked with no pending work.
    deadline: Option<Instant>,
    /// Set when the owning [`AnimationWaker`] drops: the thread exits
    /// instead of parking forever, so a dropped app does not leak the
    /// thread (plus its channel sender and proxy `Arc`s).
    shutdown: bool,
}

impl AnimationWaker {
    pub fn new(
        event_tx: flume::Sender<ExternalEvent>,
        proxy_cell: Arc<OnceLock<EventLoopProxy>>,
        interval: Duration,
    ) -> Self {
        Self {
            shared: Arc::new(WakerShared {
                state: Mutex::new(WakerState::default()),
                condvar: Condvar::new(),
            }),
            event_tx,
            proxy_cell,
            interval,
            spawn: Once::new(),
        }
    }

    /// Keep the waker ticking until at least `deadline`. The effective
    /// deadline only moves forward (`max(current, deadline)`), so animation
    /// producers can call this unconditionally on every start or retarget;
    /// a parked waker resumes immediately.
    pub fn extend_until(&self, deadline: Instant) {
        self.ensure_thread();
        {
            let mut guard = self.shared.state.lock().unwrap();
            guard.deadline = Some(extended_deadline(guard.deadline, deadline));
        }
        self.shared.condvar.notify_one();
    }

    fn ensure_thread(&self) {
        self.spawn.call_once(|| {
            let shared = Arc::clone(&self.shared);
            let event_tx = self.event_tx.clone();
            let proxy_cell = Arc::clone(&self.proxy_cell);
            let interval = self.interval;
            std::thread::Builder::new()
                .name("animation-waker".into())
                .spawn(move || waker_loop(&shared, &event_tx, &proxy_cell, interval))
                .expect("spawning the animation waker thread cannot fail");
        });
    }
}

impl Drop for AnimationWaker {
    fn drop(&mut self) {
        let mut guard = self.shared.state.lock().unwrap();
        guard.shutdown = true;
        drop(guard);
        self.shared.condvar.notify_one();
    }
}

/// Pure deadline-merge: an existing later deadline wins, otherwise adopt
/// the requested one (including from the parked `None` state).
fn extended_deadline(current: Option<Instant>, requested: Instant) -> Instant {
    match current {
        Some(existing) if existing >= requested => existing,
        _ => requested,
    }
}

fn waker_loop(
    shared: &WakerShared,
    event_tx: &flume::Sender<ExternalEvent>,
    proxy_cell: &OnceLock<EventLoopProxy>,
    interval: Duration,
) {
    loop {
        // Park until a deadline in the future exists. The deadline is
        // re-read from the mutex on every iteration of the outer loop, so
        // an `extend_until` mid-animation is honored without restarting
        // anything. A shutdown request exits from both the parked and the
        // ticking state.
        {
            let mut guard = shared.state.lock().unwrap();
            loop {
                if guard.shutdown {
                    return;
                }
                match guard.deadline {
                    Some(deadline) if Instant::now() <= deadline => break,
                    _ => {
                        guard.deadline = None;
                        guard = shared.condvar.wait(guard).unwrap();
                    }
                }
            }
        }

        std::thread::sleep(interval);
        if shared.state.lock().unwrap().shutdown {
            return;
        }
        if event_tx.send(ExternalEvent::RequestAnimationFrame).is_err() {
            // The app side dropped its receiver: shut the thread down.
            return;
        }
        if let Some(proxy) = proxy_cell.get() {
            proxy.wake_up();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_deadline_only_moves_forward() {
        let now = Instant::now();
        let near = now + Duration::from_millis(10);
        let far = now + Duration::from_millis(100);

        assert_eq!(extended_deadline(None, near), near, "a parked waker adopts any deadline");
        assert_eq!(extended_deadline(Some(near), far), far, "later requests extend");
        assert_eq!(
            extended_deadline(Some(far), near),
            far,
            "earlier requests never shorten the active window"
        );
        assert_eq!(extended_deadline(Some(far), far), far, "idempotent at equality");
    }

    fn drain(rx: &flume::Receiver<ExternalEvent>) -> usize {
        rx.try_iter().count()
    }

    #[test]
    fn waker_ticks_until_deadline_then_parks_and_resumes_on_extend() {
        let (tx, rx) = flume::unbounded::<ExternalEvent>();
        let waker = AnimationWaker::new(tx, Arc::new(OnceLock::new()), Duration::from_millis(1));

        // Active phase: ticks arrive while the deadline is in the future.
        waker.extend_until(Instant::now() + Duration::from_millis(40));
        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("an active waker must deliver animation frames");
        assert!(matches!(first, ExternalEvent::RequestAnimationFrame));

        // Stops: once the deadline passes, the thread parks and the
        // channel quiesces.
        std::thread::sleep(Duration::from_millis(120));
        drain(&rx);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(drain(&rx), 0, "a parked waker must not produce further ticks");

        // Resumes: extending the deadline wakes the same thread again.
        waker.extend_until(Instant::now() + Duration::from_millis(40));
        rx.recv_timeout(Duration::from_secs(2)).expect("an extended waker must resume ticking");
    }

    #[test]
    fn dropping_the_waker_stops_the_thread_mid_window() {
        let (tx, rx) = flume::unbounded::<ExternalEvent>();
        let waker = AnimationWaker::new(tx, Arc::new(OnceLock::new()), Duration::from_millis(1));

        // The deadline is far in the future, so without the shutdown path
        // the thread would keep ticking long after the drop.
        waker.extend_until(Instant::now() + Duration::from_secs(60));
        rx.recv_timeout(Duration::from_secs(2)).expect("the waker must tick while active");
        drop(waker);

        // Allow at most one in-flight tick to land, then the channel must
        // quiesce for good.
        std::thread::sleep(Duration::from_millis(50));
        drain(&rx);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(drain(&rx), 0, "a dropped waker must not keep ticking");
    }

    #[test]
    fn overlapping_extensions_share_one_thread_and_one_window() {
        // The replacement contract for the per-notch spawner: many rapid
        // "notches" extend one deadline instead of stacking threads. We
        // can't count threads portably, but we can assert the tick rate
        // stays at the single-thread cadence (a 1ms interval cannot
        // deliver more events than elapsed-ms plus scheduling slack even
        // with 10 overlapping extensions).
        let (tx, rx) = flume::unbounded::<ExternalEvent>();
        let waker = AnimationWaker::new(tx, Arc::new(OnceLock::new()), Duration::from_millis(1));

        let start = Instant::now();
        for _ in 0..10 {
            waker.extend_until(start + Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(100));
        let elapsed_ms = start.elapsed().as_millis() as usize;
        let ticks = drain(&rx);
        assert!(ticks > 0, "the waker must have ticked during the active window");
        assert!(
            ticks <= elapsed_ms + 5,
            "10 overlapping extensions must share one ticking thread, got {ticks} ticks in {elapsed_ms}ms"
        );
    }
}
