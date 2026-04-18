//! Integration tests for the monitor refresh plumbing on [`FramePacer`].
//!
//! The real wiring in `app.rs` reads `Window::current_monitor()` inside
//! winit event handlers. Constructing a real winit event loop just for
//! a unit test is disallowed on CI (no display on headless runners), so
//! we expose a thin trait, [`MonitorRefreshSource`], that both the real
//! window and test fakes can satisfy. The pacer update helper,
//! [`unshit_app::frame_pacer::refresh_pacer_from_source`], takes this
//! trait and knows nothing about winit.
//!
//! This file covers three behaviors (capillary #82):
//! 1. A source that reports a known rate updates the pacer accordingly.
//! 2. A source that cannot determine the rate leaves the pacer at the
//!    historic 8ms default.
//! 3. Swapping rates (the multi-monitor crossing case) updates an
//!    already-running pacer.

use std::time::Duration;
use unshit_app::frame_pacer::{refresh_pacer_from_source, FramePacer, MonitorRefreshSource};

struct FakeSource(Option<u32>);

impl MonitorRefreshSource for FakeSource {
    fn current_refresh_mhz(&self) -> Option<u32> {
        self.0
    }
}

#[test]
fn refresh_pacer_from_source_uses_reported_mhz_on_known_monitor() {
    let mut pacer = FramePacer::new();
    let source = FakeSource(Some(144_000));
    refresh_pacer_from_source(&mut pacer, &source);
    assert_eq!(pacer.min_interval(), Duration::from_nanos(6_944_444));
}

#[test]
fn refresh_pacer_from_source_falls_back_to_default_when_no_monitor() {
    let mut pacer = FramePacer::with_refresh_rate_mhz(240_000);
    // Set a non-default interval so we can verify the fallback overwrites it.
    assert_eq!(pacer.min_interval(), Duration::from_nanos(4_166_666));

    let source = FakeSource(None);
    refresh_pacer_from_source(&mut pacer, &source);
    assert_eq!(pacer.min_interval(), FramePacer::DEFAULT_MIN_INTERVAL);
}

#[test]
fn refresh_pacer_from_source_updates_pacer_when_rate_changes() {
    let mut pacer = FramePacer::with_refresh_rate_mhz(120_000);
    assert_eq!(pacer.min_interval(), Duration::from_nanos(8_333_333));

    // Simulate dragging the window onto a 240Hz display.
    let faster = FakeSource(Some(240_000));
    refresh_pacer_from_source(&mut pacer, &faster);
    assert_eq!(pacer.min_interval(), Duration::from_nanos(4_166_666));

    // And back to 60Hz.
    let slower = FakeSource(Some(60_000));
    refresh_pacer_from_source(&mut pacer, &slower);
    // 1e12 / 60_000 = 16_666_666 ns.
    assert_eq!(pacer.min_interval(), Duration::from_nanos(16_666_666));
}
