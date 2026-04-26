//! Criterion bench for `DaemonPty::write` latency.
//!
//! Phase 2 of the 120fps perf initiative (#135) made the render-thread
//! `write` call fire-and-forget. This bench guards against regression:
//! if someone reintroduces a sync `reply_rx.recv()` on the render path,
//! the `write_async_slow_daemon` time will jump from sub-microsecond
//! channel-send cost to the daemon round-trip cost (orders of magnitude
//! slower).
//!
//! The bench uses the same in-memory slow-daemon harness the unit test
//! `pty::tests::write_returns_immediately_even_when_daemon_is_infinitely_slow`
//! relies on. It does NOT spin up a real daemon; the goal is to measure
//! the cost the render thread pays per keystroke, which by design must
//! be independent of daemon health.
//!
//! `pty.rs` has no crate-internal references, so we pull it in via
//! `#[path]` rather than turning the package into a library. This keeps
//! the diff for #135 surgical.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// Bring the bin's pty module in via a path attribute so the bench can
// reach the public `DaemonPty` API without making the whole package a
// library. The included module's `#[cfg(test)]` block compiles but is
// never invoked here, so silence the dead-code and unused-import
// warnings it emits in the bench profile.
#[allow(dead_code, unused_imports)]
#[path = "../src/pty.rs"]
mod pty;

use pty::DaemonPty;

fn write_async_slow_daemon(c: &mut Criterion) {
    c.bench_function("DaemonPty::write fire-and-forget (slow daemon)", |b| {
        let mut shim = DaemonPty::new();
        let (_guard, _err_tx) = shim.test_install_slow_daemon_inner(7, 42);
        let payload = b"x";
        b.iter(|| {
            // Discarding the result is intentional: the bench measures
            // the cost of queueing a write, which is what the render
            // thread experiences. The result is always Ok() in this
            // setup because the parked `cmd_rx` keeps the channel open.
            let _ = black_box(shim.write(black_box(7), black_box(payload)));
        });
    });
}

criterion_group!(benches, write_async_slow_daemon);
criterion_main!(benches);
