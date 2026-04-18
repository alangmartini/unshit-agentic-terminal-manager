//! Integration test for issue #83.
//!
//! Verifies that the double buffered `ShapedTextCache` bounds its live set to
//! the last two frames of activity, even when every frame feeds a disjoint
//! working set of 100 unique strings. Before the refactor, the cache kept
//! everything and grew linearly with the number of unique texts that ever
//! touched the screen.

use unshit_renderer::double_buffered::DoubleBufferedCache;

#[test]
fn double_buffered_cache_is_bounded_across_many_frames() {
    // Drive the cache with 1000 unique keys spread across 10 frames
    // (100 per frame, no key repeats across frames). After 10
    // `finish_frame` calls the live set must be bounded by two frames of
    // activity, i.e. at most 200 entries. With a single unbounded map the
    // cache would carry all 1000.
    let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::with_capacity(256);

    const PER_FRAME: u32 = 100;
    const FRAMES: u32 = 10;
    for frame in 0..FRAMES {
        for i in 0..PER_FRAME {
            let k = frame * PER_FRAME + i;
            cache.insert(k, k);
        }
        cache.finish_frame();
    }

    let bound = (2 * PER_FRAME) as usize;
    assert!(
        cache.len() <= bound,
        "live set must stay within two frames ({bound}), got {}",
        cache.len(),
    );
    assert!(cache.len() >= PER_FRAME as usize, "still has the last frame of data in previous");
}

#[test]
fn lookups_in_current_do_not_grow_previous() {
    // Repeated lookups of the same key in the same frame must not cause
    // duplicate entries to accumulate in either half.
    let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
    cache.insert(42, 7);
    for _ in 0..100 {
        assert_eq!(cache.get_or_promote(&42), Some(&7));
    }
    assert_eq!(cache.len(), 1, "in-frame repeat hits do not duplicate");
}

#[test]
fn warm_working_set_survives_indefinitely() {
    // A stable warm set of keys touched every frame should never evict,
    // no matter how many frames pass. This models the terminal's core
    // hot characters (space, common ASCII, cursor).
    let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
    let warm_keys: Vec<u32> = (0..50).collect();
    for &k in &warm_keys {
        cache.insert(k, k * 2);
    }

    for _ in 0..200 {
        // Touch every warm key.
        for &k in &warm_keys {
            assert!(cache.get_or_promote(&k).is_some(), "warm key {k} missing");
        }
        cache.finish_frame();
    }

    // After 200 frames, the warm set must still be reachable.
    for &k in &warm_keys {
        assert_eq!(
            cache.get_or_promote(&k),
            Some(&(k * 2)),
            "warm key {k} evicted despite being touched every frame",
        );
    }
}
