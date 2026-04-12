//! LRU bounded cache for tessellated SVG geometry.
//!
//! Tessellation is expensive relative to drawing. Icons reused many times in
//! the same frame (or across frames) must pay the lyon cost only once. This
//! cache memoizes the `Arc<SvgGeometry>` output keyed on the raw `d` string
//! (or a serialization for non path primitives) plus the subset of
//! presentation attributes that can change the triangle list: fill color,
//! stroke color, stroke width, line cap, and line join.
//!
//! Capacity defaults to 256 entries and is configurable via `set_capacity`.
//! When we evict an entry we drop it entirely, including any GPU side buffer
//! handle associated with the entry (held as an opaque `u64` token so the
//! cache itself has no direct wgpu dependency).

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHasher};
use unshit_core::style::types::Color;
use unshit_core::svg::types::{PathCommand, StrokeLineCap, StrokeLineJoin, SvgPrimitive};

use crate::svg_tess::{tessellate, SvgGeometry, DEFAULT_TOLERANCE};
use unshit_core::svg::types::SvgAttrs;

/// Default LRU capacity. Tuned for typical UI surfaces with a few dozen
/// distinct icons plus spare headroom. Raise it per app via
/// `GpuContext::set_svg_cache_capacity`.
pub const DEFAULT_CACHE_CAPACITY: usize = 256;

/// Key under which a tessellated geometry is cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SvgTessKey {
    pub path_hash: u64,
    pub fill_rgba: u32,
    pub stroke_rgba: u32,
    pub stroke_width_mils: i32,
    pub linecap: u8,
    pub linejoin: u8,
}

impl SvgTessKey {
    /// Build a cache key from an SVG primitive plus the effective attributes
    /// that influence the triangle list.
    ///
    /// `effective_fill` and `effective_stroke` are the result of resolving
    /// `SvgPaint::Current` against the element computed color, so two uses of
    /// the same icon with different `color` styles still miss the cache and
    /// are tessellated separately.
    pub fn from_primitive(
        primitive: &SvgPrimitive,
        attrs: &SvgAttrs,
        effective_fill: Color,
        effective_stroke: Color,
    ) -> Self {
        let path_hash = hash_primitive(primitive);
        let stroke_width = attrs.stroke_width.unwrap_or(1.0);
        // Store stroke width in milli units so minor floating point jitter
        // does not thrash the cache while still capturing real differences.
        let stroke_width_mils = (stroke_width * 1000.0).round() as i32;
        let linecap = attrs.stroke_linecap.unwrap_or(StrokeLineCap::Butt).as_u8();
        let linejoin = attrs.stroke_linejoin.unwrap_or(StrokeLineJoin::Miter).as_u8();
        Self {
            path_hash,
            fill_rgba: pack_rgba(effective_fill),
            stroke_rgba: pack_rgba(effective_stroke),
            stroke_width_mils,
            linecap,
            linejoin,
        }
    }
}

fn pack_rgba(c: Color) -> u32 {
    ((c.r as u32) << 24) | ((c.g as u32) << 16) | ((c.b as u32) << 8) | (c.a as u32)
}

/// Hash the discriminating bytes of an `SvgPrimitive`. For paths we hash the
/// raw `d` string; for other shapes we serialize the shape parameters.
pub fn hash_primitive(primitive: &SvgPrimitive) -> u64 {
    let mut hasher = FxHasher::default();
    // Include a discriminant byte so `Rect` never collides with `Polygon`
    // even if both produced the same numeric sequence.
    match primitive {
        SvgPrimitive::Path { d, commands } => {
            0u8.hash(&mut hasher);
            if !d.is_empty() {
                d.hash(&mut hasher);
            } else {
                // Fall back to hashing the normalized commands.
                for cmd in commands {
                    hash_command(cmd, &mut hasher);
                }
            }
        }
        SvgPrimitive::Circle { cx, cy, r } => {
            1u8.hash(&mut hasher);
            hash_f32(*cx, &mut hasher);
            hash_f32(*cy, &mut hasher);
            hash_f32(*r, &mut hasher);
        }
        SvgPrimitive::Rect { x, y, width, height, rx, ry } => {
            2u8.hash(&mut hasher);
            hash_f32(*x, &mut hasher);
            hash_f32(*y, &mut hasher);
            hash_f32(*width, &mut hasher);
            hash_f32(*height, &mut hasher);
            hash_f32(*rx, &mut hasher);
            hash_f32(*ry, &mut hasher);
        }
        SvgPrimitive::Line { x1, y1, x2, y2 } => {
            3u8.hash(&mut hasher);
            hash_f32(*x1, &mut hasher);
            hash_f32(*y1, &mut hasher);
            hash_f32(*x2, &mut hasher);
            hash_f32(*y2, &mut hasher);
        }
        SvgPrimitive::Polyline { points } => {
            4u8.hash(&mut hasher);
            for (x, y) in points {
                hash_f32(*x, &mut hasher);
                hash_f32(*y, &mut hasher);
            }
        }
        SvgPrimitive::Polygon { points } => {
            5u8.hash(&mut hasher);
            for (x, y) in points {
                hash_f32(*x, &mut hasher);
                hash_f32(*y, &mut hasher);
            }
        }
        SvgPrimitive::Group => {
            6u8.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn hash_f32(v: f32, h: &mut FxHasher) {
    // Hash the raw bit pattern so NaN does not collapse everything.
    v.to_bits().hash(h);
}

fn hash_command(cmd: &PathCommand, h: &mut FxHasher) {
    match *cmd {
        PathCommand::MoveTo { x, y } => {
            1u8.hash(h);
            hash_f32(x, h);
            hash_f32(y, h);
        }
        PathCommand::LineTo { x, y } => {
            2u8.hash(h);
            hash_f32(x, h);
            hash_f32(y, h);
        }
        PathCommand::CubicTo { x1, y1, x2, y2, x, y } => {
            3u8.hash(h);
            hash_f32(x1, h);
            hash_f32(y1, h);
            hash_f32(x2, h);
            hash_f32(y2, h);
            hash_f32(x, h);
            hash_f32(y, h);
        }
        PathCommand::QuadTo { x1, y1, x, y } => {
            4u8.hash(h);
            hash_f32(x1, h);
            hash_f32(y1, h);
            hash_f32(x, h);
            hash_f32(y, h);
        }
        PathCommand::Close => {
            5u8.hash(h);
        }
    }
}

/// LRU cache from `SvgTessKey` to `Arc<SvgGeometry>`.
pub struct SvgTessCache {
    map: FxHashMap<SvgTessKey, Arc<SvgGeometry>>,
    order: VecDeque<SvgTessKey>,
    capacity: usize,
}

impl Default for SvgTessCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CACHE_CAPACITY)
    }
}

impl SvgTessCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains(&self, key: &SvgTessKey) -> bool {
        self.map.contains_key(key)
    }

    /// Get an existing entry without promoting it. Useful for tests.
    pub fn get(&self, key: &SvgTessKey) -> Option<Arc<SvgGeometry>> {
        self.map.get(key).cloned()
    }

    /// Look up by key and promote to most recently used. Returns `None` on
    /// miss so the caller can tessellate and insert.
    pub fn touch(&mut self, key: &SvgTessKey) -> Option<Arc<SvgGeometry>> {
        if !self.map.contains_key(key) {
            return None;
        }
        self.promote(key);
        self.map.get(key).cloned()
    }

    /// Insert a freshly tessellated geometry. If the cache is at capacity
    /// the least recently used entry is evicted first.
    pub fn insert(&mut self, key: SvgTessKey, geometry: Arc<SvgGeometry>) {
        use std::collections::hash_map::Entry;
        match self.map.entry(key) {
            Entry::Occupied(mut e) => {
                e.insert(geometry);
                self.promote(&key);
            }
            Entry::Vacant(slot) => {
                slot.insert(geometry);
                self.order.push_back(key);
                while self.map.len() > self.capacity && self.capacity > 0 {
                    if let Some(oldest) = self.order.pop_front() {
                        self.map.remove(&oldest);
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Resize the cache, evicting least recently used entries down to the
    /// new capacity.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.map.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            } else {
                break;
            }
        }
    }

    /// Look up by key, tessellate on miss, return the cached `Arc`. This is
    /// the one shot API used by the renderer every frame.
    pub fn get_or_tessellate(
        &mut self,
        primitive: &SvgPrimitive,
        attrs: &SvgAttrs,
        current_color: Color,
        effective_fill: Color,
        effective_stroke: Color,
    ) -> Arc<SvgGeometry> {
        let key = SvgTessKey::from_primitive(primitive, attrs, effective_fill, effective_stroke);
        if let Some(hit) = self.touch(&key) {
            return hit;
        }
        let geometry = tessellate(primitive, attrs, current_color, DEFAULT_TOLERANCE);
        self.insert(key, geometry.clone());
        geometry
    }

    fn promote(&mut self, key: &SvgTessKey) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
            self.order.push_back(*key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(r: f32) -> SvgPrimitive {
        SvgPrimitive::Circle { cx: 0.0, cy: 0.0, r }
    }

    fn fill_black() -> SvgAttrs {
        SvgAttrs {
            fill: Some(unshit_core::svg::types::SvgPaint::Solid(Color::BLACK)),
            ..Default::default()
        }
    }

    #[test]
    fn miss_then_hit_returns_same_arc() {
        let mut cache = SvgTessCache::with_capacity(8);
        let prim = circle(5.0);
        let attrs = fill_black();
        let a =
            cache.get_or_tessellate(&prim, &attrs, Color::BLACK, Color::BLACK, Color::TRANSPARENT);
        let b =
            cache.get_or_tessellate(&prim, &attrs, Color::BLACK, Color::BLACK, Color::TRANSPARENT);
        assert!(Arc::ptr_eq(&a, &b), "second lookup should return the same Arc");
    }

    #[test]
    fn inserting_over_capacity_evicts_oldest() {
        let mut cache = SvgTessCache::with_capacity(4);
        for i in 0..5 {
            let prim = circle(i as f32 + 1.0);
            cache.get_or_tessellate(
                &prim,
                &fill_black(),
                Color::BLACK,
                Color::BLACK,
                Color::TRANSPARENT,
            );
        }
        // Only 4 entries should remain.
        assert_eq!(cache.len(), 4);
        // Circle with r=1 was the oldest and should have been dropped.
        let oldest_key = SvgTessKey::from_primitive(
            &circle(1.0),
            &fill_black(),
            Color::BLACK,
            Color::TRANSPARENT,
        );
        assert!(!cache.contains(&oldest_key));
    }

    #[test]
    fn inserting_exactly_capacity_plus_one_evicts_single_entry() {
        let mut cache = SvgTessCache::with_capacity(256);
        for i in 0..257 {
            let prim = SvgPrimitive::Circle { cx: 0.0, cy: 0.0, r: i as f32 + 1.0 };
            cache.get_or_tessellate(
                &prim,
                &fill_black(),
                Color::BLACK,
                Color::BLACK,
                Color::TRANSPARENT,
            );
        }
        assert_eq!(cache.len(), 256);
    }

    #[test]
    fn set_capacity_downsizes() {
        let mut cache = SvgTessCache::with_capacity(8);
        for i in 0..8 {
            let prim = circle(i as f32 + 1.0);
            cache.get_or_tessellate(
                &prim,
                &fill_black(),
                Color::BLACK,
                Color::BLACK,
                Color::TRANSPARENT,
            );
        }
        cache.set_capacity(3);
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn same_path_with_different_stroke_widths_produces_distinct_entries() {
        let mut cache = SvgTessCache::with_capacity(8);
        let prim = circle(5.0);
        let mut a = fill_black();
        a.stroke = Some(unshit_core::svg::types::SvgPaint::Solid(Color::WHITE));
        a.stroke_width = Some(1.0);
        let mut b = a.clone();
        b.stroke_width = Some(4.0);

        cache.get_or_tessellate(&prim, &a, Color::BLACK, Color::BLACK, Color::WHITE);
        cache.get_or_tessellate(&prim, &b, Color::BLACK, Color::BLACK, Color::WHITE);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn touch_promotes_to_most_recent() {
        let mut cache = SvgTessCache::with_capacity(3);
        for i in 0..3 {
            let prim = circle(i as f32 + 1.0);
            cache.get_or_tessellate(
                &prim,
                &fill_black(),
                Color::BLACK,
                Color::BLACK,
                Color::TRANSPARENT,
            );
        }
        // Touch the oldest so it becomes most recent.
        let first_key = SvgTessKey::from_primitive(
            &circle(1.0),
            &fill_black(),
            Color::BLACK,
            Color::TRANSPARENT,
        );
        assert!(cache.touch(&first_key).is_some());

        // Insert a new entry. Radius 2 should now be evicted instead of radius 1.
        let new_prim = circle(99.0);
        cache.get_or_tessellate(
            &new_prim,
            &fill_black(),
            Color::BLACK,
            Color::BLACK,
            Color::TRANSPARENT,
        );
        assert!(cache.contains(&first_key), "recently touched entry should survive");
        let evicted_key = SvgTessKey::from_primitive(
            &circle(2.0),
            &fill_black(),
            Color::BLACK,
            Color::TRANSPARENT,
        );
        assert!(!cache.contains(&evicted_key));
    }
}
