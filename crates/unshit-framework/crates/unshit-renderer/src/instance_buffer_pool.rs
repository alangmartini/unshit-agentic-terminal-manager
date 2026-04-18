//! Per submission `wgpu::Buffer` recycling pool.
//!
//! Background
//!
//! Every per pipeline instance buffer was historically a single
//! `wgpu::Buffer` owned by the pipeline struct. Every frame called
//! `queue.write_buffer` into that same buffer and then recorded draws
//! from it. While the 8ms `FramePacer` ceiling was in place this was
//! safe because the driver's implicit frame latency kept CPU at most one
//! frame ahead of GPU. Once the ceiling is removed and the app sustains
//! 200 plus fps on a fast display, CPU writes can race GPU reads on the
//! same instance buffer.
//!
//! `InstanceBufferPool<T>` is the portable wgpu 24 equivalent of Zed's
//! Metal `InstanceBufferPool` (`metal_renderer.rs:56-109`). Pipelines no
//! longer own a bare `wgpu::Buffer`. Instead they acquire one from the
//! pool each frame (or each batch, for images), write into it, and
//! release it back to the pool inside the `Queue::on_submitted_work_done`
//! callback for the submit that referenced the buffer.
//!
//! Lifetime protocol
//!
//! * `pool.acquire(device, min_elements)` returns a `PooledBuffer<T>`.
//!   The pool either recycles a free buffer at the current target size
//!   or allocates a new one via the grow policy.
//! * Caller writes instance data via `pooled.write(queue, instances)`.
//! * Caller records draws against `pooled.as_buffer().slice(..)`.
//! * After `queue.submit`, the caller hands the `PooledBuffer<T>` to
//!   `queue.on_submitted_work_done(move || drop(pooled))`. When the
//!   callback runs (after GPU is done reading) the `Drop` impl returns
//!   the buffer to the pool.
//!
//! Per type separation
//!
//! One pool per pipeline, parameterised by `T`. Unifying would couple
//! pipelines with wildly different typical capacities (quads 4096,
//! glyphs 16384, svg 64) and waste memory. The `PhantomData<T>` also
//! prevents cross type reuse at compile time.

use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, Mutex};

use bytemuck::Pod;

/// Hard cap on a single pooled buffer size, in bytes.
///
/// Matches the `256 MB` mental cap used by Zed's Metal pool and by the
/// default wgpu 24 desktop adapter limits. Hitting the cap logs a warn
/// and truncates the caller's instances rather than allocating beyond
/// the limit.
pub const BUFFER_SIZE_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// Shared inner state. Behind `Arc<Mutex<...>>` because
/// `on_submitted_work_done` callbacks can fire on a different thread
/// than the one that called `acquire`.
struct Inner {
    /// Free buffers at the current `buffer_size_bytes`. Undersized
    /// leftover buffers after a grow are dropped rather than reused.
    free: Vec<wgpu::Buffer>,
    /// Current target size in bytes.
    buffer_size_bytes: u64,
    /// Hard cap on growth.
    grow_cap_bytes: u64,
    /// Diagnostic counter: total live buffers this pool has allocated
    /// (does not decrement on drop, it is lifetime total).
    total_allocated: u64,
    /// Diagnostic counter: total buffers currently outstanding (acquired
    /// but not yet released).
    outstanding: u64,
}

/// Statistics snapshot for tests and metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    pub free_len: usize,
    pub buffer_size_bytes: u64,
    pub total_allocated: u64,
    pub outstanding: u64,
}

pub struct InstanceBufferPool<T: Pod> {
    inner: Arc<Mutex<Inner>>,
    label: &'static str,
    usage: wgpu::BufferUsages,
    _ty: PhantomData<fn() -> T>,
}

/// A buffer borrowed from the pool for one submit cycle.
///
/// Dropping the value returns the buffer to the pool when the size
/// matches the current target. If the size no longer matches (the pool
/// grew or was reset after this buffer was acquired), the buffer is
/// dropped for real, letting wgpu reclaim the memory.
pub struct PooledBuffer<T: Pod> {
    buffer: Option<wgpu::Buffer>,
    buffer_size_bytes: u64,
    pool: Arc<Mutex<Inner>>,
    _ty: PhantomData<fn() -> T>,
}

impl<T: Pod> InstanceBufferPool<T> {
    /// Build a pool for `T` with an initial per buffer capacity of
    /// `initial_elements` and the `VERTEX | COPY_DST` usage flags plus
    /// any extra flags supplied by the caller.
    pub fn new(
        label: &'static str,
        initial_elements: usize,
        extra_usage: wgpu::BufferUsages,
    ) -> Self {
        let base_usage = wgpu::BufferUsages::COPY_DST;
        let usage = base_usage | extra_usage;
        let buffer_size_bytes = (initial_elements.max(1) * size_of::<T>()) as u64;
        let grow_cap_bytes = BUFFER_SIZE_CAP_BYTES;
        Self {
            inner: Arc::new(Mutex::new(Inner {
                free: Vec::new(),
                buffer_size_bytes,
                grow_cap_bytes,
                total_allocated: 0,
                outstanding: 0,
            })),
            label,
            usage,
            _ty: PhantomData,
        }
    }

    /// Override the hard cap on buffer size. Primarily for tests.
    pub fn set_grow_cap_bytes(&self, cap: u64) {
        let mut inner = self.inner.lock().expect("instance buffer pool mutex poisoned");
        inner.grow_cap_bytes = cap;
        if inner.buffer_size_bytes > cap {
            inner.buffer_size_bytes = cap;
            inner.free.clear();
        }
    }

    /// Acquire a buffer with capacity for at least `min_elements` items.
    /// Grows (doubling to `next_power_of_two`) when needed, capped at
    /// `grow_cap_bytes`. If the cap would be exceeded the pool clamps
    /// to `grow_cap_bytes / size_of::<T>()` and logs a warn; the caller
    /// must detect the clamp (by comparing their slice length against
    /// the returned buffer's capacity before writing) to avoid GPU side
    /// out of bounds reads.
    pub fn acquire(&self, device: &wgpu::Device, min_elements: usize) -> PooledBuffer<T> {
        let needed_bytes = (min_elements.max(1) * size_of::<T>()) as u64;
        let mut inner = self.inner.lock().expect("instance buffer pool mutex poisoned");

        if needed_bytes > inner.buffer_size_bytes {
            let next_elements = min_elements.next_power_of_two().max(1);
            let mut new_size = (next_elements * size_of::<T>()) as u64;
            if new_size > inner.grow_cap_bytes {
                log::warn!(
                    "InstanceBufferPool<{}> grow cap hit: asked {} bytes, capped at {} bytes",
                    std::any::type_name::<T>(),
                    new_size,
                    inner.grow_cap_bytes
                );
                new_size = inner.grow_cap_bytes;
            }
            inner.buffer_size_bytes = new_size;
            // Drop all smaller free buffers; they cannot host the new size.
            inner.free.clear();
        }

        let buffer = if let Some(buf) = inner.free.pop() {
            buf
        } else {
            inner.total_allocated += 1;
            let size = inner.buffer_size_bytes;
            let label = self.label;
            let usage = self.usage;
            drop(inner);
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            });
            let mut inner = self.inner.lock().expect("instance buffer pool mutex poisoned");
            inner.outstanding += 1;
            return PooledBuffer {
                buffer: Some(buf),
                buffer_size_bytes: inner.buffer_size_bytes,
                pool: self.inner.clone(),
                _ty: PhantomData,
            };
        };
        inner.outstanding += 1;
        PooledBuffer {
            buffer: Some(buffer),
            buffer_size_bytes: inner.buffer_size_bytes,
            pool: self.inner.clone(),
            _ty: PhantomData,
        }
    }

    /// Drop all free buffers and reset the per buffer capacity to
    /// `new_element_capacity`. Outstanding buffers keep their current
    /// size and will be dropped on release because their size no longer
    /// matches the target. Primarily useful when shrinking after a
    /// workload spike.
    pub fn reset(&self, new_element_capacity: usize) {
        let mut inner = self.inner.lock().expect("instance buffer pool mutex poisoned");
        inner.buffer_size_bytes = (new_element_capacity.max(1) * size_of::<T>()) as u64;
        inner.free.clear();
    }

    /// Diagnostic snapshot.
    pub fn stats(&self) -> PoolStats {
        let inner = self.inner.lock().expect("instance buffer pool mutex poisoned");
        PoolStats {
            free_len: inner.free.len(),
            buffer_size_bytes: inner.buffer_size_bytes,
            total_allocated: inner.total_allocated,
            outstanding: inner.outstanding,
        }
    }

    /// Current per buffer byte capacity.
    pub fn buffer_size_bytes(&self) -> u64 {
        self.inner.lock().expect("instance buffer pool mutex poisoned").buffer_size_bytes
    }
}

impl<T: Pod> PooledBuffer<T> {
    /// The underlying GPU buffer. Use for binding and draw recording.
    pub fn as_buffer(&self) -> &wgpu::Buffer {
        self.buffer.as_ref().expect("pooled buffer already returned")
    }

    /// Write instance data at offset 0.
    ///
    /// If the slice exceeds the buffer capacity it is truncated to the
    /// capacity and a warn is logged. Callers should have sized the
    /// pool before reaching this path.
    pub fn write(&self, queue: &wgpu::Queue, instances: &[T]) {
        if instances.is_empty() {
            return;
        }
        let buf = self.buffer.as_ref().expect("pooled buffer already returned");
        let requested = std::mem::size_of_val(instances) as u64;
        let cap = self.buffer_size_bytes;
        let bytes = if requested > cap {
            log::warn!(
                "InstanceBufferPool<{}> write truncated: asked {} bytes, cap {} bytes",
                std::any::type_name::<T>(),
                requested,
                cap
            );
            let max_elements = (cap as usize) / size_of::<T>();
            bytemuck::cast_slice(&instances[..max_elements])
        } else {
            bytemuck::cast_slice(instances)
        };
        queue.write_buffer(buf, 0, bytes);
    }

    /// Buffer capacity in bytes at acquire time.
    pub fn size_bytes(&self) -> u64 {
        self.buffer_size_bytes
    }
}

impl<T: Pod> Drop for PooledBuffer<T> {
    fn drop(&mut self) {
        let Some(buffer) = self.buffer.take() else {
            return;
        };
        // A poisoned mutex means a thread panicked while holding the
        // lock. We must not propagate a second panic out of Drop.
        let Ok(mut inner) = self.pool.lock() else {
            log::warn!(
                "InstanceBufferPool<{}> mutex poisoned during release; leaking buffer",
                std::any::type_name::<T>()
            );
            return;
        };
        inner.outstanding = inner.outstanding.saturating_sub(1);
        if self.buffer_size_bytes == inner.buffer_size_bytes {
            inner.free.push(buffer);
        }
        // Undersized or oversized leftover; drop for real.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct TestInstance {
        _a: [f32; 4],
        _b: [f32; 4],
    }

    // 32 bytes.
    const SIZE: u64 = std::mem::size_of::<TestInstance>() as u64;

    fn try_device() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("pool test device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            },
            None,
        ))
        .ok()?;
        Some((Arc::new(device), Arc::new(queue)))
    }

    /// Tests are gated on a wgpu adapter being available. Headless CI
    /// without a GPU returns early with a skipped log line.
    macro_rules! require_gpu {
        ($name:ident, $dq:pat, $body:block) => {{
            static SKIPPED: AtomicUsize = AtomicUsize::new(0);
            match try_device() {
                Some($dq) => $body,
                None => {
                    if SKIPPED.fetch_add(1, Ordering::Relaxed) == 0 {
                        eprintln!(
                            "[instance_buffer_pool::tests] skipping {}: no adapter",
                            stringify!($name)
                        );
                    }
                }
            }
        }};
    }

    #[test]
    fn pool_acquire_below_capacity_reuses_buffer() {
        require_gpu!(pool_acquire_below_capacity_reuses_buffer, (device, _queue), {
            let pool =
                InstanceBufferPool::<TestInstance>::new("test", 64, wgpu::BufferUsages::VERTEX);
            let b1 = pool.acquire(&device, 16);
            let before = pool.stats();
            assert_eq!(before.outstanding, 1);
            drop(b1);
            let after_drop = pool.stats();
            assert_eq!(after_drop.outstanding, 0);
            assert_eq!(after_drop.free_len, 1);
            let b2 = pool.acquire(&device, 16);
            let after_reacquire = pool.stats();
            assert_eq!(after_reacquire.free_len, 0);
            // No extra allocation occurred.
            assert_eq!(after_reacquire.total_allocated, before.total_allocated);
            drop(b2);
        });
    }

    #[test]
    fn pool_acquire_above_capacity_grows_and_drops_small_buffers() {
        require_gpu!(
            pool_acquire_above_capacity_grows_and_drops_small_buffers,
            (device, _queue),
            {
                let pool =
                    InstanceBufferPool::<TestInstance>::new("test", 4, wgpu::BufferUsages::VERTEX);
                // Seed the free list with small buffers.
                let small = pool.acquire(&device, 4);
                let small2 = pool.acquire(&device, 4);
                drop(small);
                drop(small2);
                assert_eq!(pool.stats().free_len, 2);
                let initial_size = pool.buffer_size_bytes();
                // Grow past the current capacity.
                let big = pool.acquire(&device, 100);
                let stats = pool.stats();
                // Free list was purged: smaller buffers cannot host the new size.
                assert_eq!(stats.free_len, 0);
                assert!(pool.buffer_size_bytes() > initial_size);
                // Next power of two of 100 is 128. 128 * 32 = 4096 bytes.
                assert_eq!(pool.buffer_size_bytes(), 128 * SIZE);
                drop(big);
            }
        );
    }

    #[test]
    fn pool_grow_cap_truncates() {
        require_gpu!(pool_grow_cap_truncates, (device, _queue), {
            let pool =
                InstanceBufferPool::<TestInstance>::new("test", 4, wgpu::BufferUsages::VERTEX);
            // Tiny cap: 2 * SIZE = 64 bytes, so asking for many elements
            // must clamp back down.
            pool.set_grow_cap_bytes(2 * SIZE);
            let pooled = pool.acquire(&device, 100);
            assert_eq!(pool.buffer_size_bytes(), 2 * SIZE);
            drop(pooled);
        });
    }

    #[test]
    fn pool_drop_returns_buffer_when_size_matches() {
        require_gpu!(pool_drop_returns_buffer_when_size_matches, (device, _queue), {
            let pool =
                InstanceBufferPool::<TestInstance>::new("test", 64, wgpu::BufferUsages::VERTEX);
            let pooled = pool.acquire(&device, 16);
            drop(pooled);
            assert_eq!(pool.stats().free_len, 1);
        });
    }

    #[test]
    fn pool_drop_discards_buffer_when_size_mismatches_after_reset() {
        require_gpu!(
            pool_drop_discards_buffer_when_size_mismatches_after_reset,
            (device, _queue),
            {
                let pool =
                    InstanceBufferPool::<TestInstance>::new("test", 64, wgpu::BufferUsages::VERTEX);
                let pooled = pool.acquire(&device, 16);
                pool.reset(8);
                assert_eq!(pool.stats().free_len, 0);
                drop(pooled);
                // Old sized buffer must not have been recycled because the
                // target size changed.
                assert_eq!(pool.stats().free_len, 0);
            }
        );
    }

    #[test]
    fn pool_reset_clears_existing_free_buffers() {
        require_gpu!(pool_reset_clears_existing_free_buffers, (device, _queue), {
            let pool =
                InstanceBufferPool::<TestInstance>::new("test", 64, wgpu::BufferUsages::VERTEX);
            let a = pool.acquire(&device, 16);
            let b = pool.acquire(&device, 16);
            drop(a);
            drop(b);
            assert_eq!(pool.stats().free_len, 2);
            pool.reset(4);
            assert_eq!(pool.stats().free_len, 0);
        });
    }

    #[test]
    fn pool_acquire_from_multiple_threads_is_race_free() {
        require_gpu!(pool_acquire_from_multiple_threads_is_race_free, (device, _queue), {
            let pool = Arc::new(InstanceBufferPool::<TestInstance>::new(
                "test",
                64,
                wgpu::BufferUsages::VERTEX,
            ));

            let mut handles = Vec::new();
            for _ in 0..8 {
                let pool = pool.clone();
                let device = device.clone();
                handles.push(std::thread::spawn(move || {
                    for _ in 0..20 {
                        let pooled = pool.acquire(&device, 16);
                        drop(pooled);
                    }
                }));
            }
            for h in handles {
                h.join().expect("thread panicked");
            }
            let stats = pool.stats();
            assert_eq!(stats.outstanding, 0);
            // All releases land back on the free list, bounded by the
            // pool's growth. Because grow only happens on demand and
            // the threads all ask for the same size, total_allocated
            // should equal the final free_len.
            assert_eq!(stats.free_len as u64, stats.total_allocated);
        });
    }

    #[test]
    fn pooled_buffer_write_round_trip() {
        require_gpu!(pooled_buffer_write_round_trip, (device, queue), {
            let pool = InstanceBufferPool::<TestInstance>::new(
                "roundtrip",
                4,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC,
            );
            let pooled = pool.acquire(&device, 2);
            let expected = [
                TestInstance { _a: [1.0, 2.0, 3.0, 4.0], _b: [5.0, 6.0, 7.0, 8.0] },
                TestInstance { _a: [9.0, 10.0, 11.0, 12.0], _b: [13.0, 14.0, 15.0, 16.0] },
            ];
            pooled.write(&queue, &expected);

            // Copy to a MAP_READ staging buffer so we can read back.
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("roundtrip staging"),
                size: (2 * SIZE),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("roundtrip encoder"),
            });
            encoder.copy_buffer_to_buffer(pooled.as_buffer(), 0, &staging, 0, 2 * SIZE);
            queue.submit(std::iter::once(encoder.finish()));
            let slice = staging.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().unwrap();
            let data = slice.get_mapped_range();
            let round: &[TestInstance] = bytemuck::cast_slice(&data);
            assert_eq!(round.len(), 2);
            for (a, b) in round.iter().zip(expected.iter()) {
                assert_eq!(a._a, b._a);
                assert_eq!(a._b, b._b);
            }
            drop(data);
            staging.unmap();
            drop(pooled);
        });
    }

    #[test]
    fn pool_write_truncates_when_slice_exceeds_capacity() {
        require_gpu!(pool_write_truncates_when_slice_exceeds_capacity, (device, queue), {
            let pool =
                InstanceBufferPool::<TestInstance>::new("trunc", 4, wgpu::BufferUsages::VERTEX);
            pool.set_grow_cap_bytes(2 * SIZE);
            let pooled = pool.acquire(&device, 4);
            assert_eq!(pool.buffer_size_bytes(), 2 * SIZE);
            // Oversized slice: 4 > 2.
            let big = [TestInstance::zeroed(); 4];
            pooled.write(&queue, &big);
            // Does not panic; truncation warns.
        });
    }

    /// Compile time check: pools of different `T` do not interoperate.
    /// Ensures `PhantomData<fn() -> T>` keeps the variance correct and
    /// API cannot accidentally swap a `PooledBuffer<A>` into a pool of
    /// `PooledBuffer<B>`. This is a type system guard, not a runtime
    /// assertion. A cross type swap would fail to compile.
    #[test]
    fn pool_phantomdata_prevents_cross_type_reuse_compile_only() {
        fn _only_takes_a(_p: InstanceBufferPool<TestInstance>) {}
        fn _only_takes_b(_p: InstanceBufferPool<[f32; 8]>) {}
        // The two pool types are distinct and cannot be interchanged.
    }
}
