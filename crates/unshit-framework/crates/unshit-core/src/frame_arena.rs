//! Per-frame bump allocator for transient element tree nodes.
//!
//! # Philosophy
//!
//! Zed resets a 1 MB bump allocator each frame: allocations during tree
//! construction become a single pointer bump per node, and end-of-frame
//! cleanup is a single pointer reset that preserves the underlying chunk
//! capacity for the next draw. This module provides the same shape for
//! the unshit framework via [`bumpalo::Bump`].
//!
//! # IMPORTANT: Drop safety invariant
//!
//! Anything allocated into the arena by value must be POD-like (no
//! meaningful `Drop`). `FrameArena::reset()` resets the bump pointer to
//! zero without running destructors on the previously allocated values.
//! This is fine for `&'a str` slices, [`bumpalo::collections::Vec`]
//! elements backed by the arena, and small `Copy` types. It is NOT fine
//! for values like `Arc`, owned `String`, `Box`, or any `Vec` backed by
//! the global allocator: skipping their `Drop` leaks the underlying
//! resource.
//!
//! `ElementDefBump<'a>` follows this rule by keeping closures
//! (`Arc<dyn Fn>`) as fields that get dropped when the user-owned clones
//! go out of scope, not when the arena resets. Do not add fields with
//! non-trivial `Drop` implementations to bump-allocated structs without
//! reviewing the invariant.
//!
//! # Threading
//!
//! `bumpalo::Bump` is `!Sync`. A `FrameArena` must live on the UI thread
//! and never be shared between threads. `AppState` is already UI-thread
//! local, so this is a one-line invariant.
//!
//! # Nested draws
//!
//! Unlike Zed we have no nested draw contexts. Every frame runs its
//! reconcile + batch + submit sequence to completion before the next
//! frame starts, so there is no need for a pointer stack scoping like
//! `ElementArenaScope`. The single-frame invariant is the whole story.

/// Thin newtype around [`bumpalo::Bump`] used as the per-frame transient
/// allocator. One instance lives on `AppState`; [`reset`](Self::reset) is
/// called at the end of each rendered frame.
pub struct FrameArena {
    bump: bumpalo::Bump,
}

impl Default for FrameArena {
    /// Default 1 MB capacity, matching Zed's `ELEMENT_ARENA`. The bump
    /// will grow transparently if a frame's tree overflows; the chunks
    /// are retained across [`reset`](Self::reset) so the arena converges
    /// on a steady-state peak capacity.
    fn default() -> Self {
        Self::with_capacity(1024 * 1024)
    }
}

impl FrameArena {
    /// Construct an arena with the specified starting capacity in bytes.
    ///
    /// A pre-warmed chunk is allocated immediately; subsequent frames
    /// reuse it unless the tree outgrows the pre-allocated chunk.
    pub fn with_capacity(bytes: usize) -> Self {
        Self { bump: bumpalo::Bump::with_capacity(bytes) }
    }

    /// Reset the bump pointer to zero.
    ///
    /// This is O(1) and preserves the underlying chunks. Destructors for
    /// values previously allocated into the arena are NOT run. See the
    /// module-level docs for the drop-safety invariant.
    pub fn reset(&mut self) {
        self.bump.reset();
    }

    /// Allocate a value of type `T` into the arena and return a mutable
    /// reference borrowed from the arena.
    pub fn alloc<T>(&self, value: T) -> &mut T {
        self.bump.alloc(value)
    }

    /// Copy the contents of `s` into the arena and return a borrowed
    /// `&str` reference that lives as long as the arena.
    pub fn alloc_str<'a>(&'a self, s: &str) -> &'a str {
        self.bump.alloc_str(s)
    }

    /// Return a shared reference to the underlying [`bumpalo::Bump`].
    ///
    /// Use this when a caller needs to construct
    /// [`bumpalo::collections::Vec`] / [`bumpalo::collections::String`]
    /// values that require a `&Bump` reference.
    pub fn bump(&self) -> &bumpalo::Bump {
        &self.bump
    }

    /// Total bytes currently allocated across all chunks, including the
    /// live high-water mark. After a [`reset`](Self::reset) the bump
    /// pointer is back to zero but the chunks are retained, so this
    /// reports the preserved capacity rather than live usage.
    pub fn allocated_bytes(&self) -> usize {
        self.bump.allocated_bytes()
    }

    /// Number of chunks currently held by the arena. Starts at 1 after
    /// the pre-warm chunk is allocated; grows by one every time the
    /// arena overflows its current chunk.
    ///
    /// Requires `&mut self` because `bumpalo::Bump::iter_allocated_chunks`
    /// mutably borrows the bump even though it only reads chunks; this
    /// is a bumpalo API restriction.
    pub fn chunk_count(&mut self) -> usize {
        self.bump.iter_allocated_chunks().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// New arena reports a single pre-warmed chunk.
    #[test]
    fn new_arena_has_pre_warmed_chunk() {
        let mut arena = FrameArena::with_capacity(4096);
        assert!(arena.chunk_count() >= 1, "arena should have at least one chunk after warm-up");
    }

    /// After a reset, bump pointer is back to zero but chunks persist.
    #[test]
    fn alloc_then_reset_preserves_chunks() {
        let mut arena = FrameArena::with_capacity(4096);
        for i in 0..100u32 {
            arena.alloc(i);
        }
        let chunks_before = arena.chunk_count();
        arena.reset();
        let chunks_after = arena.chunk_count();
        assert_eq!(
            chunks_after, chunks_before,
            "chunk count must not shrink after reset; the whole point is to keep capacity"
        );
    }

    /// Allocation is readable within the frame.
    #[test]
    fn alloc_survives_within_frame() {
        let arena = FrameArena::with_capacity(4096);
        let a = arena.alloc(42u32);
        let b = arena.alloc(99u32);
        assert_eq!(*a, 42);
        assert_eq!(*b, 99);
    }

    /// Reset must NOT run destructors on arena-allocated values. This
    /// documents the POD requirement. Storing a type with real `Drop`
    /// inside the arena leaks the underlying resource.
    #[test]
    fn reset_does_not_call_drop() {
        struct DropCounter<'a> {
            counter: &'a AtomicUsize,
        }

        impl Drop for DropCounter<'_> {
            fn drop(&mut self) {
                self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = AtomicUsize::new(0);
        let mut arena = FrameArena::with_capacity(4096);
        {
            arena.alloc(DropCounter { counter: &drops });
        }
        arena.reset();

        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "reset must not call Drop on arena-allocated values"
        );
    }

    /// Across many frames, the arena converges to a bounded steady state.
    /// This exercises the zed pattern: alloc per frame, reset per frame,
    /// no unbounded growth.
    #[test]
    fn bounded_memory_across_frames() {
        let mut arena = FrameArena::with_capacity(64 * 1024);
        let mut peak_bytes = 0;
        for _frame in 0..100 {
            for i in 0..500u32 {
                arena.alloc(i);
            }
            peak_bytes = peak_bytes.max(arena.allocated_bytes());
            arena.reset();
        }
        // After 100 frames of reset, capacity should still be bounded by
        // a small multiple of the pre-warm size. Allow 16x slack to be
        // safe against bumpalo's internal chunk-doubling strategy.
        assert!(
            arena.allocated_bytes() <= 16 * 64 * 1024,
            "arena grew unboundedly across reset cycles: {} bytes",
            arena.allocated_bytes()
        );
        assert!(peak_bytes > 0);
    }

    /// `alloc_str` copies the string into the arena and the returned
    /// reference is usable for the arena's lifetime.
    #[test]
    fn alloc_str_copies_into_arena() {
        let arena = FrameArena::with_capacity(4096);
        let s = arena.alloc_str("hello world");
        assert_eq!(s, "hello world");
    }

    /// Accessing `bump()` yields a usable reference for building
    /// `bumpalo::collections::Vec`.
    #[test]
    fn bump_returns_usable_reference() {
        let arena = FrameArena::with_capacity(4096);
        let mut v: bumpalo::collections::Vec<u32> = bumpalo::collections::Vec::new_in(arena.bump());
        v.push(1);
        v.push(2);
        v.push(3);
        assert_eq!(&v[..], &[1, 2, 3]);
    }

    /// Arc clones on the outside of the arena must survive the arena
    /// reset. The Arc refcount is independent of the arena and drop logic
    /// fires when the Arc handle itself goes out of scope.
    #[test]
    fn arcs_outside_arena_not_affected_by_reset() {
        struct DropCounter {
            counter: Arc<AtomicUsize>,
        }

        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let owned_arc = Arc::new(DropCounter { counter: Arc::clone(&drops) });

        {
            let mut arena = FrameArena::with_capacity(4096);
            // The Arc clone lives alongside the arena, NOT inside it.
            let cloned = Arc::clone(&owned_arc);
            // Store only a reference in the arena so dropping the arena
            // does not drop the arc.
            arena.alloc(42u32);
            drop(cloned);
            arena.reset();
        }
        assert_eq!(drops.load(Ordering::SeqCst), 0, "Arc still held by owned_arc must not drop");
        drop(owned_arc);
        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "Arc drops exactly once when the last handle is released"
        );
    }
}
