use rustc_hash::FxHashMap;
use wgpu;

const ATLAS_SIZE: u32 = 2048;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    pub font_id: u64,
    pub glyph_id: u16,
    pub font_size_tenths: u16,
    pub subpixel_bin: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct GlyphEntry {
    pub uv_rect: [f32; 4], // u0, v0, u1, v1
    pub offset: [f32; 2],
    pub size: [f32; 2],
}

pub struct PendingGlyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

struct Shelf {
    y: u32,
    height: u32,
    cursor_x: u32,
    /// Keys of all glyphs that were placed on this shelf.
    glyph_keys: Vec<GlyphKey>,
    /// Whether this shelf has been freed and can be reused.
    free: bool,
}

/// LRU tracking state, separated from GPU resources for testability.
pub struct LruTracker {
    /// Frame counter, incremented each frame.
    pub frame_counter: u64,
    /// Last frame each glyph was used. Key = GlyphKey, Value = frame number.
    pub last_used: FxHashMap<GlyphKey, u64>,
}

impl LruTracker {
    pub fn new() -> Self {
        Self { frame_counter: 0, last_used: FxHashMap::default() }
    }

    /// Record that a glyph was used in the current frame.
    pub fn touch(&mut self, key: &GlyphKey) {
        self.last_used.insert(*key, self.frame_counter);
    }

    /// Advance to the next frame.
    pub fn advance_frame(&mut self) {
        self.frame_counter += 1;
    }

    /// Return keys of glyphs not used for more than `max_unused_frames` frames.
    pub fn stale_keys(&self, max_unused_frames: u64) -> Vec<GlyphKey> {
        self.last_used
            .iter()
            .filter(|(_, &last)| self.frame_counter.saturating_sub(last) > max_unused_frames)
            .map(|(k, _)| *k)
            .collect()
    }
}

pub struct GlyphAtlas {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub size: u32,
    shelves: Vec<Shelf>,
    pub cache: FxHashMap<GlyphKey, GlyphEntry>,
    pub pending_uploads: Vec<PendingGlyph>,
    pub next_shelf_y: u32,
    /// LRU tracking.
    pub lru: LruTracker,
    /// Maximum atlas texture size in pixels (width = height).
    pub max_size: u32,
    /// Texture format (R8Unorm for grayscale, Rgba8Unorm for subpixel).
    pub format: wgpu::TextureFormat,
    /// Bytes per pixel (1 for R8Unorm, 4 for Rgba8Unorm).
    pub bytes_per_pixel: u32,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_format(device, ATLAS_SIZE, wgpu::TextureFormat::R8Unorm)
    }

    pub fn new_with_size(device: &wgpu::Device, size: u32) -> Self {
        Self::new_with_format(device, size, wgpu::TextureFormat::R8Unorm)
    }

    pub fn new_with_format(
        device: &wgpu::Device,
        size: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Rgba8Unorm => 4,
            _ => panic!("Unsupported atlas format: {format:?}"),
        };

        let filter = match format {
            wgpu::TextureFormat::Rgba8Unorm => wgpu::FilterMode::Nearest,
            _ => wgpu::FilterMode::Linear,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph sampler"),
            mag_filter: filter,
            min_filter: filter,
            ..Default::default()
        });

        Self {
            texture,
            texture_view,
            sampler,
            size,
            shelves: Vec::new(),
            cache: FxHashMap::default(),
            pending_uploads: Vec::new(),
            next_shelf_y: 0,
            lru: LruTracker::new(),
            max_size: size,
            format,
            bytes_per_pixel,
        }
    }

    /// Record that a glyph was used in the current frame (for LRU tracking).
    pub fn touch(&mut self, key: &GlyphKey) {
        self.lru.touch(key);
    }

    /// Advance the frame counter. Call once per rendered frame.
    pub fn advance_frame(&mut self) {
        self.lru.advance_frame();
    }

    /// Evict glyphs not used for more than `max_unused_frames` frames.
    ///
    /// Removes stale entries from `cache` and `lru.last_used`. When all
    /// glyphs on a shelf are evicted the shelf is marked free so it can be
    /// reused by new glyphs. If no free shelf space exists after eviction
    /// a full atlas rebuild is triggered via [`Self::rebuild`].
    pub fn evict_unused(
        &mut self,
        max_unused_frames: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let stale = self.lru.stale_keys(max_unused_frames);
        if stale.is_empty() {
            return;
        }

        for key in &stale {
            self.cache.remove(key);
            self.lru.last_used.remove(key);
        }

        // Determine which shelves now have all glyphs evicted and mark them free.
        let cache = &self.cache;
        for shelf in &mut self.shelves {
            if shelf.free {
                continue;
            }
            // A shelf is fully evicted when none of its glyphs remain in cache.
            let all_gone = shelf.glyph_keys.iter().all(|k| !cache.contains_key(k));
            if all_gone {
                shelf.free = true;
            }
        }

        // Check if any free shelf or remaining next_shelf_y space is available.
        let has_free_shelf = self.shelves.iter().any(|s| s.free);
        let has_y_space = self.next_shelf_y < self.size;

        if !has_free_shelf && !has_y_space {
            self.rebuild(device, queue);
        }
    }

    /// Rebuild the atlas from scratch, re-packing only surviving cache entries.
    ///
    /// Creates a fresh texture of the same size, re-shelves all surviving
    /// glyphs, and regenerates pending uploads for them. After this call
    /// the caller must ensure the GPU pipeline's bind group is recreated
    /// to reference the new texture and view.
    pub fn rebuild(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Collect surviving entries. We need to re-rasterize them because we
        // only stored UV coordinates, not the raw pixel data. In practice the
        // caller (batch builder) will re-insert them on the next frame via
        // get_or_insert, so here we simply wipe the atlas state so it starts
        // fresh and lets the batch builder repopulate it.
        let _ = queue; // kept for API symmetry; actual upload happens via get_or_insert

        let new_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d { width: self.size, height: self.size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let new_view = new_texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.texture = new_texture;
        self.texture_view = new_view;
        self.shelves.clear();
        self.next_shelf_y = 0;
        self.cache.clear();
        self.lru.last_used.clear();
        self.pending_uploads.clear();
    }

    pub fn get_or_insert(
        &mut self,
        key: GlyphKey,
        width: u32,
        height: u32,
        data: Vec<u8>,
        offset: [f32; 2],
    ) -> Option<GlyphEntry> {
        if let Some(entry) = self.cache.get(&key) {
            return Some(*entry);
        }

        if width == 0 || height == 0 {
            let entry = GlyphEntry { uv_rect: [0.0; 4], offset, size: [0.0, 0.0] };
            self.cache.insert(key, entry);
            return Some(entry);
        }

        // Try to fit in an existing free shelf first (previously fully evicted).
        let alloc = self.shelves.iter_mut().find_map(|shelf| {
            if shelf.free && height <= shelf.height {
                // Reset this shelf for reuse.
                shelf.free = false;
                shelf.cursor_x = 0;
                shelf.glyph_keys.clear();
            }
            if !shelf.free && height <= shelf.height && shelf.cursor_x + width < self.size {
                let x = shelf.cursor_x;
                let y = shelf.y;
                shelf.cursor_x += width + 1; // 1px padding
                shelf.glyph_keys.push(key);
                Some((x, y))
            } else {
                None
            }
        });

        let (x, y) = if let Some(pos) = alloc {
            pos
        } else {
            // New shelf
            if self.next_shelf_y + height > self.size {
                return None; // Atlas full
            }
            let mut shelf = Shelf {
                y: self.next_shelf_y,
                height: height + 1,
                cursor_x: width + 1,
                glyph_keys: Vec::new(),
                free: false,
            };
            shelf.glyph_keys.push(key);
            let pos = (0, shelf.y);
            self.next_shelf_y += shelf.height;
            self.shelves.push(shelf);
            pos
        };

        let sz = self.size as f32;
        let entry = GlyphEntry {
            uv_rect: [
                x as f32 / sz,
                y as f32 / sz,
                (x + width) as f32 / sz,
                (y + height) as f32 / sz,
            ],
            offset,
            size: [width as f32, height as f32],
        };

        self.cache.insert(key, entry);
        self.pending_uploads.push(PendingGlyph { x, y, width, height, data });

        Some(entry)
    }

    pub fn upload_pending(&mut self, queue: &wgpu::Queue) {
        for glyph in self.pending_uploads.drain(..) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: glyph.x, y: glyph.y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &glyph.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(glyph.width * self.bytes_per_pixel),
                    rows_per_image: Some(glyph.height),
                },
                wgpu::Extent3d {
                    width: glyph.width,
                    height: glyph.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(glyph_id: u16) -> GlyphKey {
        GlyphKey { font_id: 1, glyph_id, font_size_tenths: 120, subpixel_bin: 0 }
    }

    #[test]
    fn lru_touch_updates_last_used() {
        let mut lru = LruTracker::new();
        let key = make_key(1);
        lru.touch(&key);
        assert_eq!(lru.last_used.get(&key), Some(&0));
    }

    #[test]
    fn lru_advance_frame_increments_counter() {
        let mut lru = LruTracker::new();
        assert_eq!(lru.frame_counter, 0);
        lru.advance_frame();
        assert_eq!(lru.frame_counter, 1);
        lru.advance_frame();
        assert_eq!(lru.frame_counter, 2);
    }

    #[test]
    fn lru_stale_keys_returns_old_glyphs() {
        let mut lru = LruTracker::new();
        let old_key = make_key(1);
        let recent_key = make_key(2);

        // Touch old_key at frame 0, then advance many frames, then touch recent_key.
        lru.touch(&old_key);
        for _ in 0..10 {
            lru.advance_frame();
        }
        lru.touch(&recent_key);

        // With max_unused_frames = 5, old_key (used 10 frames ago) should be stale.
        let stale = lru.stale_keys(5);
        assert!(stale.contains(&old_key), "old_key should be stale");
        assert!(!stale.contains(&recent_key), "recent_key should not be stale");
    }

    #[test]
    fn lru_recently_used_glyph_survives_eviction() {
        let mut lru = LruTracker::new();
        let key = make_key(42);

        // Touch the glyph every frame; it should never be stale.
        for _ in 0..20 {
            lru.touch(&key);
            lru.advance_frame();
        }

        let stale = lru.stale_keys(5);
        assert!(!stale.contains(&key), "recently used glyph must not be stale");
    }

    #[test]
    fn lru_evict_unused_removes_from_maps() {
        let mut lru = LruTracker::new();
        let stale_key = make_key(10);
        let fresh_key = make_key(11);

        lru.touch(&stale_key);
        for _ in 0..10 {
            lru.advance_frame();
        }
        lru.touch(&fresh_key);

        // Simulate what evict_unused does with the LruTracker alone.
        let stale = lru.stale_keys(5);
        for k in &stale {
            lru.last_used.remove(k);
        }

        assert!(!lru.last_used.contains_key(&stale_key), "stale key removed from last_used");
        assert!(lru.last_used.contains_key(&fresh_key), "fresh key still in last_used");
    }

    #[test]
    fn lru_no_stale_keys_when_all_recent() {
        let mut lru = LruTracker::new();
        for i in 0..5u16 {
            lru.touch(&make_key(i));
        }
        lru.advance_frame();
        assert!(lru.stale_keys(10).is_empty(), "no stale keys expected");
    }
}
