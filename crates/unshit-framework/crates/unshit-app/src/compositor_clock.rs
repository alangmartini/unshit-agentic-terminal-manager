//! Display-clock wake source for non-blocking Windows Mailbox surfaces.
//!
//! `DCompositionWaitForCompositorClock` wakes on the compositor heartbeat,
//! which is the vblank-aligned clock DirectComposition uses to latch frames.
//! Keeping that wait on one lazily spawned thread lets the winit event loop
//! remain non-blocking while avoiding timer drift and swapchain-acquire stalls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use crate::event_sink::ExternalEvent;
use winit::event_loop::EventLoopProxy;

type WaitForCompositorClock = unsafe extern "system" fn(u32, *const std::ffi::c_void, u32) -> i32;

const COMPOSITOR_WAIT_TIMEOUT_MS: u32 = 50;

/// One app-lifetime compositor-clock waiter. The thread exists only after an
/// active frame window is requested, and parks again when that window expires.
pub(crate) struct CompositorClockWaker {
    shared: Arc<WakerShared>,
    event_tx: flume::Sender<ExternalEvent>,
    proxy_cell: Arc<OnceLock<EventLoopProxy>>,
    fallback_interval: Duration,
    supported: bool,
    spawn: Once,
}

struct WakerShared {
    state: Mutex<WakerState>,
    condvar: Condvar,
    event_pending: AtomicBool,
}

#[derive(Default)]
struct WakerState {
    deadline: Option<Instant>,
    shutdown: bool,
}

impl CompositorClockWaker {
    pub(crate) fn new(
        event_tx: flume::Sender<ExternalEvent>,
        proxy_cell: Arc<OnceLock<EventLoopProxy>>,
        fallback_interval: Duration,
    ) -> Self {
        Self {
            shared: Arc::new(WakerShared {
                state: Mutex::new(WakerState::default()),
                condvar: Condvar::new(),
                event_pending: AtomicBool::new(false),
            }),
            event_tx,
            proxy_cell,
            fallback_interval,
            supported: compositor_wait_fn().is_some(),
            spawn: Once::new(),
        }
    }

    /// Whether this process can resolve the native compositor-clock API.
    /// No thread is created by this probe.
    pub(crate) fn is_supported(&self) -> bool {
        self.supported
    }

    /// Keep compositor ticks flowing through at least `deadline`. Calls only
    /// extend an existing window, so independent animation and input producers
    /// cannot accidentally shorten one another's active period.
    pub(crate) fn extend_until(&self, deadline: Instant) {
        if !self.supported {
            return;
        }
        self.ensure_thread();
        {
            let mut guard = self.shared.state.lock().unwrap();
            guard.deadline = Some(extended_deadline(guard.deadline, deadline));
        }
        self.shared.condvar.notify_one();
    }

    /// Release the one-event coalescing slot after the UI thread drains a
    /// compositor tick. A slow frame can therefore miss heartbeats without
    /// building an unbounded queue of stale redraws.
    pub(crate) fn acknowledge_tick(&self) {
        self.shared.event_pending.store(false, Ordering::Release);
    }

    fn ensure_thread(&self) {
        self.spawn.call_once(|| {
            let shared = Arc::clone(&self.shared);
            let event_tx = self.event_tx.clone();
            let proxy_cell = Arc::clone(&self.proxy_cell);
            let fallback_interval = self.fallback_interval;
            std::thread::Builder::new()
                .name("compositor-clock-waker".into())
                .spawn(move || {
                    waker_loop(&shared, &event_tx, &proxy_cell, fallback_interval);
                })
                .expect("spawning the compositor clock waker cannot fail");
        });
    }
}

impl Drop for CompositorClockWaker {
    fn drop(&mut self) {
        let mut guard = self.shared.state.lock().unwrap();
        guard.shutdown = true;
        drop(guard);
        self.shared.condvar.notify_one();
    }
}

fn extended_deadline(current: Option<Instant>, requested: Instant) -> Instant {
    match current {
        Some(existing) if existing >= requested => existing,
        _ => requested,
    }
}

fn wait_until_active(shared: &WakerShared) -> bool {
    let mut guard = shared.state.lock().unwrap();
    loop {
        if guard.shutdown {
            return false;
        }
        match guard.deadline {
            Some(deadline) if Instant::now() <= deadline => return true,
            _ => {
                guard.deadline = None;
                guard = shared.condvar.wait(guard).unwrap();
            }
        }
    }
}

fn tick_is_still_active(shared: &WakerShared, tick_at: Instant) -> bool {
    let guard = shared.state.lock().unwrap();
    !guard.shutdown && guard.deadline.is_some_and(|deadline| tick_at <= deadline)
}

fn enqueue_tick(
    shared: &WakerShared,
    event_tx: &flume::Sender<ExternalEvent>,
    tick_at: Instant,
) -> Option<bool> {
    if shared.event_pending.swap(true, Ordering::AcqRel) {
        return Some(false);
    }
    if event_tx.send(ExternalEvent::RequestCompositorFrame { tick_at }).is_err() {
        shared.event_pending.store(false, Ordering::Release);
        return None;
    }
    Some(true)
}

fn waker_loop(
    shared: &WakerShared,
    event_tx: &flume::Sender<ExternalEvent>,
    proxy_cell: &OnceLock<EventLoopProxy>,
    fallback_interval: Duration,
) {
    #[cfg(target_os = "windows")]
    let _multimedia_thread = crate::app::MultimediaRenderThread::register("compositor_clock");

    let correlation_id = format!("process-{}", std::process::id());
    let wait_fn = compositor_wait_fn().expect("supported compositor clock must stay resolved");
    let mut use_native_clock = true;
    log::info!(
        "{{\"event\":\"frame_scheduler.compositor_clock_started\",\"correlation_id\":{correlation_id:?},\"fallback_interval_ns\":{}}}",
        fallback_interval.as_nanos(),
    );

    loop {
        if !wait_until_active(shared) {
            return;
        }

        if use_native_clock {
            // SAFETY: count is zero, so the API does not dereference the null
            // handle array. `wait_fn` was resolved from dcomp.dll and the
            // module is deliberately retained for the process lifetime.
            let result = unsafe { wait_fn(0, std::ptr::null(), COMPOSITOR_WAIT_TIMEOUT_MS) };
            if result != 0 {
                log::warn!(
                    "{{\"event\":\"frame_scheduler.compositor_clock_wait_failed\",\"correlation_id\":{correlation_id:?},\"hresult\":{},\"fallback\":\"monotonic_timer\"}}",
                    result as u32,
                );
                use_native_clock = false;
                std::thread::sleep(fallback_interval);
            }
        } else {
            std::thread::sleep(fallback_interval);
        }

        let tick_at = Instant::now();
        if !tick_is_still_active(shared, tick_at) {
            continue;
        }
        match enqueue_tick(shared, event_tx, tick_at) {
            Some(true) => {
                if let Some(proxy) = proxy_cell.get() {
                    proxy.wake_up();
                }
            }
            Some(false) => {}
            None => return,
        }
    }
}

pub(crate) fn compositor_wait_fn() -> Option<WaitForCompositorClock> {
    static WAIT_FN: OnceLock<Option<WaitForCompositorClock>> = OnceLock::new();
    *WAIT_FN.get_or_init(load_compositor_wait_fn)
}

#[cfg(target_os = "windows")]
fn load_compositor_wait_fn() -> Option<WaitForCompositorClock> {
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};

    let library_name = "dcomp.dll\0".encode_utf16().collect::<Vec<_>>();
    // SAFETY: `library_name` is NUL terminated and lives through the call. The
    // returned module is intentionally never freed because the resolved
    // function pointer is stored in a process-lifetime `OnceLock`.
    let module = unsafe { LoadLibraryW(library_name.as_ptr()) };
    if module.is_null() {
        return None;
    }
    // SAFETY: module is a live dcomp.dll handle and the symbol name is a
    // process-lifetime NUL-terminated C string.
    let address = unsafe { GetProcAddress(module, c"DCompositionWaitForCompositorClock".as_ptr()) };
    if address.is_null() {
        return None;
    }
    // SAFETY: Microsoft documents this exported symbol with the exact ABI and
    // argument layout represented by `WaitForCompositorClock`.
    Some(unsafe {
        std::mem::transmute::<*mut std::ffi::c_void, WaitForCompositorClock>(address.cast())
    })
}

#[cfg(not(target_os = "windows"))]
fn load_compositor_wait_fn() -> Option<WaitForCompositorClock> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_extensions_only_move_forward() {
        let now = Instant::now();
        let near = now + Duration::from_millis(10);
        let far = now + Duration::from_millis(100);

        assert_eq!(extended_deadline(None, near), near);
        assert_eq!(extended_deadline(Some(near), far), far);
        assert_eq!(extended_deadline(Some(far), near), far);
    }

    #[test]
    fn pending_tick_coalesces_until_acknowledged() {
        let shared = WakerShared {
            state: Mutex::new(WakerState::default()),
            condvar: Condvar::new(),
            event_pending: AtomicBool::new(false),
        };
        let (tx, rx) = flume::unbounded();
        let first_tick = Instant::now();

        assert_eq!(enqueue_tick(&shared, &tx, first_tick), Some(true));
        assert_eq!(enqueue_tick(&shared, &tx, first_tick + Duration::from_millis(8)), Some(false));
        assert_eq!(rx.len(), 1, "only one stale compositor event may queue");

        shared.event_pending.store(false, Ordering::Release);
        assert_eq!(enqueue_tick(&shared, &tx, first_tick + Duration::from_millis(16)), Some(true));
        assert_eq!(rx.len(), 2, "acknowledging opens exactly one new slot");
    }
}
