//! Double buffered cache with a one frame eviction horizon.
//!
//! Ports Zed's `LineLayoutCache` pattern to our renderer. The cache stores two
//! maps, `previous` and `current`. Lookups first check `current`; on a miss
//! the entry is promoted out of `previous` into `current`. At the end of the
//! frame the two maps are swapped and the new `current` (the old `previous`)
//! is cleared. Any entry that is not touched during a frame is automatically
//! dropped one frame later. This is a self cleaning LRU with a one frame
//! horizon, no timers, no explicit eviction scheduling.
//!
//! Design notes:
//! - `finish_frame` uses `std::mem::swap` plus `FxHashMap::clear`. The clear
//!   retains capacity so the heap allocation is not churned every frame.
//! - `get_or_promote` removes from `previous` before inserting into `current`
//!   so a key cannot live in both halves at once.
//! - No internal synchronization: the single render thread owns the cache
//!   mutably. Adding a `Mutex` or `RwLock` would be pure overhead because
//!   `build_render_batch` already takes the cache by `&mut`.
//! - `clear` empties both halves, for coarse invalidations like font family
//!   or DPI changes.

use std::hash::Hash;

use rustc_hash::FxHashMap;

/// Two frame cache with a one frame eviction horizon.
///
/// See module docs for semantics. The cache is deliberately not wrapped in a
/// lock because the renderer owns it mutably on a single thread.
pub struct DoubleBufferedCache<K, V> {
    previous: FxHashMap<K, V>,
    current: FxHashMap<K, V>,
}

impl<K, V> Default for DoubleBufferedCache<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> DoubleBufferedCache<K, V>
where
    K: Eq + Hash,
{
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self { previous: FxHashMap::default(), current: FxHashMap::default() }
    }

    /// Construct an empty cache preallocating `cap` slots in each half.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            previous: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
            current: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
        }
    }

    /// Look up `k`. If present in `current`, return a reference. If only in
    /// `previous`, move the entry into `current` and return a reference into
    /// `current`. Returns `None` if neither half contains the key.
    pub fn get_or_promote(&mut self, k: &K) -> Option<&V>
    where
        K: Clone,
    {
        self.get_or_promote_tracked(k).map(|(v, _)| v)
    }

    /// Like [`Self::get_or_promote`] but also reports whether the entry was
    /// promoted out of `previous` on this call. The flag is `true` when the
    /// lookup moved the entry across halves, `false` when it was already in
    /// `current`. Useful for diagnostic counters that classify hits without
    /// doing a second hash lookup.
    pub fn get_or_promote_tracked(&mut self, k: &K) -> Option<(&V, bool)>
    where
        K: Clone,
    {
        if self.current.contains_key(k) {
            return self.current.get(k).map(|v| (v, false));
        }
        let value = self.previous.remove(k)?;
        self.current.insert(k.clone(), value);
        self.current.get(k).map(|v| (v, true))
    }

    /// Look up `k`. If present in `current`, return a mutable reference. If
    /// only in `previous`, promote the entry and return a mutable reference
    /// into `current`. Returns `None` if neither half contains the key.
    pub fn get_mut_or_promote(&mut self, k: &K) -> Option<&mut V>
    where
        K: Clone,
    {
        if !self.current.contains_key(k) {
            let value = self.previous.remove(k)?;
            self.current.insert(k.clone(), value);
        }
        self.current.get_mut(k)
    }

    /// Look up `k` without promoting it. Prefer `get_or_promote` on the hot
    /// render path; this is only useful for diagnostics.
    pub fn peek(&self, k: &K) -> Option<&V> {
        self.current.get(k).or_else(|| self.previous.get(k))
    }

    /// Insert into `current`. Does not touch `previous`. An existing entry
    /// in `current` is overwritten; an entry in `previous` with the same key
    /// is not removed but will be evicted at the next `finish_frame`.
    pub fn insert(&mut self, k: K, v: V) {
        self.current.insert(k, v);
    }

    /// Remove `k` from both halves if present. Used when a cached entry is
    /// known to be invalid mid frame (for example an atlas residency miss).
    pub fn remove(&mut self, k: &K) {
        self.current.remove(k);
        self.previous.remove(k);
    }

    /// Swap `previous` and `current`, then clear the new `current`. Any entry
    /// that was only in the old `previous` (not promoted this frame) is
    /// dropped. Allocations are retained across the swap because `clear` keeps
    /// capacity.
    pub fn finish_frame(&mut self) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
    }

    /// Total live entries across both halves.
    pub fn len(&self) -> usize {
        self.previous.len() + self.current.len()
    }

    /// True when both halves are empty.
    pub fn is_empty(&self) -> bool {
        self.previous.is_empty() && self.current.is_empty()
    }

    /// Hard reset: empty both halves. Call on coarse invalidations like a
    /// font family or DPI change.
    pub fn clear(&mut self) {
        self.previous.clear();
        self.current.clear();
    }

    /// Diagnostic: count of entries in the current frame's map.
    pub fn current_len(&self) -> usize {
        self.current.len()
    }

    /// Diagnostic: count of entries in the previous frame's map.
    pub fn previous_len(&self) -> usize {
        self.previous.len()
    }

    /// Diagnostic: capacity of the current frame's map. Used by tests that
    /// assert `finish_frame` does not reallocate.
    pub fn current_capacity(&self) -> usize {
        self.current.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_is_empty() {
        let cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn insert_then_get_current_returns_value() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 42);
        assert_eq!(cache.get_or_promote(&1), Some(&42));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn missing_key_returns_none() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        assert_eq!(cache.get_or_promote(&7), None);
    }

    #[test]
    fn entry_promoted_from_previous_to_current_on_get() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 42);
        cache.finish_frame();
        // After the swap, the entry lives in previous. Touching it promotes.
        assert_eq!(cache.previous_len(), 1);
        assert_eq!(cache.current_len(), 0);

        assert_eq!(cache.get_or_promote(&1), Some(&42));

        assert_eq!(cache.previous_len(), 0, "previous must be empty after promotion");
        assert_eq!(cache.current_len(), 1, "current must hold the promoted entry");
    }

    #[test]
    fn entry_not_touched_for_one_frame_survives() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 42);
        cache.finish_frame(); // moves to previous
        assert_eq!(cache.get_or_promote(&1), Some(&42)); // promotes back
        cache.finish_frame(); // moves to previous again
        assert_eq!(cache.get_or_promote(&1), Some(&42), "entry must still be reachable");
    }

    #[test]
    fn entry_not_touched_for_two_frames_evicted() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 42);
        cache.finish_frame(); // moves to previous
        cache.finish_frame(); // drops (previous was not touched)
        assert_eq!(
            cache.get_or_promote(&1),
            None,
            "untouched entry must evict after 2 finish_frame"
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn get_promoted_value_matches_original() {
        let mut cache: DoubleBufferedCache<String, String> = DoubleBufferedCache::new();
        cache.insert("hello".to_string(), "world".to_string());
        cache.finish_frame();
        let v = cache.get_or_promote(&"hello".to_string());
        assert_eq!(v.map(|s| s.as_str()), Some("world"));
    }

    #[test]
    fn clear_empties_both_halves() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 10);
        cache.finish_frame();
        cache.insert(2, 20);
        assert_eq!(cache.previous_len(), 1);
        assert_eq!(cache.current_len(), 1);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.previous_len(), 0);
        assert_eq!(cache.current_len(), 0);
    }

    #[test]
    fn finish_frame_preserves_capacity() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::with_capacity(64);
        let cap_before = cache.current_capacity();
        // Insert enough entries that the underlying map has to grow only if it
        // started small, but with_capacity(64) should accommodate.
        for i in 0..32 {
            cache.insert(i, i);
        }
        let cap_after_insert = cache.current_capacity();
        cache.finish_frame();
        let cap_after_finish = cache.current_capacity();
        assert!(
            cap_after_finish >= cap_after_insert.min(cap_before),
            "finish_frame must retain capacity; before={cap_before}, after_insert={cap_after_insert}, \
             after_finish={cap_after_finish}",
        );
    }

    #[test]
    fn insert_overwrites_in_current_but_not_previous() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 10);
        cache.finish_frame(); // moves (1, 10) to previous
        cache.insert(1, 20); // writes to current only
                             // Lookup should prefer current.
        assert_eq!(cache.get_or_promote(&1), Some(&20));
        // previous still had (1, 10) but after get_or_promote found (1, 20)
        // in current, the entry in previous is orphaned. It gets dropped at
        // the next finish_frame.
        cache.finish_frame();
        cache.finish_frame();
        assert_eq!(cache.get_or_promote(&1), None);
    }

    #[test]
    fn peek_finds_in_either_half_without_promotion() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 42);
        cache.finish_frame();
        assert_eq!(cache.peek(&1), Some(&42));
        // peek must not promote.
        assert_eq!(cache.previous_len(), 1);
        assert_eq!(cache.current_len(), 0);
    }

    #[test]
    fn get_mut_or_promote_allows_in_place_mutation() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 10);
        cache.finish_frame();
        if let Some(v) = cache.get_mut_or_promote(&1) {
            *v = 99;
        }
        assert_eq!(cache.get_or_promote(&1), Some(&99));
        assert_eq!(cache.previous_len(), 0);
        assert_eq!(cache.current_len(), 1);
    }

    #[test]
    fn remove_clears_key_from_both_halves() {
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 10);
        cache.finish_frame(); // (1, 10) in previous
        cache.insert(1, 20); // (1, 20) in current
        cache.remove(&1);
        assert_eq!(cache.get_or_promote(&1), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn promote_from_previous_only_if_not_in_current() {
        // Regression for a potential leak: a key that exists in both halves
        // must not leave a dangling entry in previous after promotion.
        let mut cache: DoubleBufferedCache<u32, u32> = DoubleBufferedCache::new();
        cache.insert(1, 10);
        cache.finish_frame();
        // (1, 10) in previous. Now also insert into current.
        cache.insert(1, 20);
        // Lookup finds (1, 20) in current; previous entry remains orphaned
        // until the next finish_frame.
        assert_eq!(cache.get_or_promote(&1), Some(&20));
        assert_eq!(cache.current_len(), 1);
        // Finish a frame: current (1, 20) -> previous, old previous dropped.
        cache.finish_frame();
        assert_eq!(cache.previous_len(), 1);
        assert_eq!(cache.get_or_promote(&1), Some(&20));
    }
}
