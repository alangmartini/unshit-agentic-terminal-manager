//! Storage for the cell metrics published by the renderer.
//!
//! Application code reads these to compute PTY column/row counts that match
//! the renderer exactly. Normal builds share one process-wide pair of atomics.
//! With the `test-cell-metrics` feature (or core unit tests) the values are
//! isolated per thread; spawned workers must publish their own metrics.

#[cfg(not(any(test, feature = "test-cell-metrics")))]
mod store {
    use std::sync::atomic::{AtomicU32, Ordering};

    static CELL_W: AtomicU32 = AtomicU32::new(0);
    static CELL_H: AtomicU32 = AtomicU32::new(0);

    pub fn set(w: f32, h: f32) {
        CELL_W.store(w.to_bits(), Ordering::Relaxed);
        CELL_H.store(h.to_bits(), Ordering::Relaxed);
    }

    pub fn get() -> (f32, f32) {
        (
            f32::from_bits(CELL_W.load(Ordering::Relaxed)),
            f32::from_bits(CELL_H.load(Ordering::Relaxed)),
        )
    }
}

#[cfg(any(test, feature = "test-cell-metrics"))]
mod store {
    use std::cell::Cell;

    std::thread_local! {
        static CELL_METRICS: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    }

    pub fn set(w: f32, h: f32) {
        CELL_METRICS.set((w, h));
    }

    pub fn get() -> (f32, f32) {
        CELL_METRICS.get()
    }
}

pub(crate) use store::{get, set};
