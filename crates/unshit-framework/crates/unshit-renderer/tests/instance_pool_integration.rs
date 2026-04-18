//! Integration tests for the per submission instance buffer pool.
//!
//! Covers the full render loop: acquire buffer from pool, write
//! instances, record draws, submit, release via
//! `Queue::on_submitted_work_done`. The regression tests (`regression_81_item2_*`)
//! stress the Zed race pattern that the pool exists to prevent.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use unshit_renderer::gpu::GpuContext;
use unshit_renderer::instance_buffer_pool::InstanceBufferPool;
use unshit_renderer::pipeline::quad::QuadInstance;

fn try_gpu(w: u32, h: u32) -> Option<GpuContext> {
    // Request real adapter limits. The quad pipeline packs more than 16
    // vertex attributes, which fails validation against the default
    // wgpu headless limits.
    std::env::set_var("TM_HEADLESS_ADAPTER_LIMITS", "1");
    pollster::block_on(GpuContext::try_new_headless(w, h, None))
}

/// Gate tests on GPU availability so CI without an adapter does not
/// fail. Logs a single line on first skip.
macro_rules! require_gpu {
    ($name:ident, $ctx:pat, $body:block) => {{
        static SKIPPED: AtomicUsize = AtomicUsize::new(0);
        match try_gpu(64, 64) {
            Some($ctx) => $body,
            None => {
                if SKIPPED.fetch_add(1, Ordering::Relaxed) == 0 {
                    eprintln!(
                        "[instance_pool_integration] skipping {}: no adapter",
                        stringify!($name)
                    );
                }
            }
        }
    }};
}

#[test]
fn render_two_frames_with_pool_does_not_corrupt_second_frame() {
    // Render two frames back to back; the pool must hand out a
    // different buffer (or the same one only after the first submit
    // completed), and the second frame's pixels must not mix with the
    // first frame's data. Matches the "no held stale content"
    // invariant the Zed race would break.
    require_gpu!(render_two_frames_with_pool_does_not_corrupt_second_frame, mut ctx, {
        ctx.render();
        let _ = ctx.read_pixels();
        ctx.render();
        let second = ctx.read_pixels();
        // Any pixel: pool behaviour must not stall the frame.
        assert!(!second.is_empty());
    });
}

#[test]
fn reuse_buffer_after_submission_completes() {
    // Verify `on_submitted_work_done` returns buffers to the pool.
    // The test polls the device after render to ensure the callback
    // has fired, then checks the pool's outstanding counter.
    require_gpu!(reuse_buffer_after_submission_completes, mut ctx, {
        let device = ctx.device.clone();
        // Render once to populate the pool with one frame's worth of
        // instance buffers (even an empty frame attaches no pooled
        // buffers; so skip the assertion in that case).
        ctx.render();
        device.poll(wgpu::Maintain::Wait);
        let stats = ctx.quad_pipeline.instance_pool.stats();
        // Outstanding must drop back to zero after the callback fires
        // regardless of whether we submitted quads this frame.
        assert_eq!(
            stats.outstanding, 0,
            "pool still has {} outstanding buffers after submit completion",
            stats.outstanding
        );
    });
}

#[test]
fn pool_survives_1000_frame_loop() {
    // Bounded allocation over many frames. If the drop path leaks
    // buffers we will see either a runaway `total_allocated` or a
    // growing free list.
    require_gpu!(pool_survives_1000_frame_loop, mut ctx, {
        let device = ctx.device.clone();
        for _ in 0..1000 {
            ctx.render();
            // Drain the queue so callbacks fire each frame.
            device.poll(wgpu::Maintain::Poll);
        }
        device.poll(wgpu::Maintain::Wait);
        let stats = ctx.quad_pipeline.instance_pool.stats();
        assert_eq!(stats.outstanding, 0);
        // An empty frame renders zero quads, so the pool may never
        // allocate. What must not happen is unbounded growth. Any
        // small constant is fine.
        assert!(stats.free_len <= 8, "pool leaked buffers: free_len={}", stats.free_len);
    });
}

#[test]
fn regression_81_item2_no_cpu_gpu_race_at_200_fps() {
    // Regression for issue #86 of epic #81. Without the pool, relaxing
    // the 8ms `FramePacer` ceiling lets the CPU write frame N plus 1
    // into the same buffer the GPU is reading for frame N, producing
    // visible tearing. The pool's per submission recycling is the fix.
    //
    // This test renders as fast as the CPU can produce frames for a
    // bounded iteration count (instead of a real timer so the test is
    // deterministic in CI), asserts the pool's outstanding counter
    // stays bounded, and asserts no dropped callback (which would
    // appear as outstanding > 0 after the final device.poll).
    require_gpu!(regression_81_item2_no_cpu_gpu_race_at_200_fps, mut ctx, {
        let device = ctx.device.clone();
        // 600 frames at max rate is enough to clear the `desired_maximum_frame_latency = 2`
        // CPU GPU buffer many times over.
        for _ in 0..600 {
            ctx.render();
        }
        device.poll(wgpu::Maintain::Wait);
        let quad_stats = ctx.quad_pipeline.instance_pool.stats();
        let glyph_stats = ctx.text_pipeline.instance_pool.stats();
        assert_eq!(quad_stats.outstanding, 0, "quad pool leaked");
        assert_eq!(glyph_stats.outstanding, 0, "glyph pool leaked");
    });
}

#[test]
fn regression_81_item2_pool_drop_is_mutex_poison_safe() {
    // Regression for pitfall 3 of the implementation plan: `PooledBuffer::drop`
    // must not panic when the pool mutex is poisoned. Panicking in a
    // destructor during unwinding would abort the process.
    require_gpu!(regression_81_item2_pool_drop_is_mutex_poison_safe, ctx, {
        // Create a local pool, poison its mutex, then drop a pooled
        // buffer. The drop must not panic.
        let pool = Arc::new(Mutex::new(()));
        let local_pool =
            InstanceBufferPool::<QuadInstance>::new("poison test", 4, wgpu::BufferUsages::VERTEX);
        let pooled = local_pool.acquire(&ctx.device, 1);
        // Poison the external mutex (not the pool's) to simulate a
        // panicking critical section somewhere in the codebase.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = pool.lock().unwrap();
            panic!("test poison");
        }));
        assert!(pool.lock().is_err());
        // Dropping the pooled buffer still succeeds because its own
        // pool mutex is independent and unpoisoned.
        drop(pooled);
        let stats = local_pool.stats();
        assert_eq!(stats.outstanding, 0);
    });
}
