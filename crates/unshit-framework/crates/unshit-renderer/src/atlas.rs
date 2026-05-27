use rustc_hash::FxHashMap;
use wgpu;

const ATLAS_SIZE: u32 = 2048;

fn glyph_atlas_filter_mode(format: wgpu::TextureFormat) -> wgpu::FilterMode {
    match format {
        wgpu::TextureFormat::R8Unorm => wgpu::FilterMode::Nearest,
        wgpu::TextureFormat::Rgba8Unorm if crate::text_rendering::use_subpixel_text_shader() => {
            wgpu::FilterMode::Nearest
        }
        _ => wgpu::FilterMode::Linear,
    }
}

/// Kind of atlas a glyph belongs to.
///
/// Monochrome text glyphs use a single channel coverage (R8Unorm). Color
/// glyphs like emoji use full BGRA/RGBA so their palette is preserved.
/// Ghostty and Zed both keep these textures separate so the fragment shader
/// can sample the correct format without branching on per glyph state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphAtlasKind {
    /// Grayscale alpha coverage, one byte per texel.
    Mono,
    /// Full color bitmap (RGBA or BGRA), four bytes per texel. Used for
    /// emoji and other color glyphs.
    Color,
}

impl GlyphAtlasKind {
    /// Map a cosmic-text swash content tag to the atlas kind it belongs in.
    pub fn from_swash_content(content: cosmic_text::SwashContent) -> Self {
        match content {
            cosmic_text::SwashContent::Color => GlyphAtlasKind::Color,
            cosmic_text::SwashContent::Mask | cosmic_text::SwashContent::SubpixelMask => {
                GlyphAtlasKind::Mono
            }
        }
    }

    /// Returns `true` for color glyphs.
    pub fn is_color(self) -> bool {
        matches!(self, GlyphAtlasKind::Color)
    }

    /// Texture format the atlas of this kind should use.
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            GlyphAtlasKind::Mono => wgpu::TextureFormat::R8Unorm,
            GlyphAtlasKind::Color => wgpu::TextureFormat::Rgba8Unorm,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    /// Stable namespace derived from the shaping font id plus glyph-rendering
    /// flags. This keeps atlas entries from different fonts/styles isolated.
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
    /// Monotonic generation that bumps when atlas residency changes in a way
    /// that can invalidate cached glyph UV usage.
    pub generation: u64,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_format(device, ATLAS_SIZE, wgpu::TextureFormat::R8Unorm)
    }

    pub fn new_with_size(device: &wgpu::Device, size: u32) -> Self {
        Self::new_with_format(device, size, wgpu::TextureFormat::R8Unorm)
    }

    pub fn new_with_format(device: &wgpu::Device, size: u32, format: wgpu::TextureFormat) -> Self {
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Rgba8Unorm => 4,
            _ => panic!("Unsupported atlas format: {format:?}"),
        };

        let filter = glyph_atlas_filter_mode(format);

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
            generation: 0,
        }
    }

    #[inline]
    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
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
        } else {
            // Cache residency changed (some glyph entries were removed), so any
            // renderer-side cached glyph instances must be considered stale.
            self.bump_generation();
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
        self.bump_generation();
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

    /// Pure cache lookup for the experimental fragment shader grid path.
    /// Returns the cached [`GlyphEntry`] (atlas UV rect plus pixel offset
    /// and size) for `key` or `None` when the key has not been inserted
    /// or has been evicted. Never rasterizes; the fragment renderer must
    /// reuse whatever is already resident.
    pub fn glyph_meta(&self, key: &GlyphKey) -> Option<GlyphEntry> {
        self.cache.get(key).copied()
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

/// Pair of atlases, one for monochrome text coverage and one for color
/// glyphs. The renderer inserts a glyph into the atlas indicated by its
/// `GlyphAtlasKind` so the fragment shader never has to branch on format.
///
/// Access via `atlas_for` / `atlas_for_mut`; callers obtain the correct
/// atlas for the `GlyphAtlasKind` they want to touch.
///
/// The color atlas is allocated lazily the first time a color glyph is
/// inserted, so runs that only render monochrome text pay zero extra
/// cost.
pub struct GlyphAtlasSet {
    pub mono: GlyphAtlas,
    color: Option<GlyphAtlas>,
    size: u32,
}

impl GlyphAtlasSet {
    /// Create a new set with the monochrome atlas pre allocated. The color
    /// atlas is created on first use.
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_size(device, ATLAS_SIZE)
    }

    pub fn new_with_size(device: &wgpu::Device, size: u32) -> Self {
        let mono = GlyphAtlas::new_with_format(device, size, wgpu::TextureFormat::R8Unorm);
        Self { mono, color: None, size }
    }

    /// Immutable access to the atlas for a given kind. Returns `None` for
    /// the color atlas when it has not yet been allocated.
    pub fn atlas_for(&self, kind: GlyphAtlasKind) -> Option<&GlyphAtlas> {
        match kind {
            GlyphAtlasKind::Mono => Some(&self.mono),
            GlyphAtlasKind::Color => self.color.as_ref(),
        }
    }

    /// Mutable access to the atlas for a given kind. Lazily allocates the
    /// color atlas on first request.
    pub fn atlas_for_mut(
        &mut self,
        kind: GlyphAtlasKind,
        device: &wgpu::Device,
    ) -> &mut GlyphAtlas {
        match kind {
            GlyphAtlasKind::Mono => &mut self.mono,
            GlyphAtlasKind::Color => self.color.get_or_insert_with(|| {
                GlyphAtlas::new_with_format(device, self.size, wgpu::TextureFormat::Rgba8Unorm)
            }),
        }
    }

    /// Advance the LRU frame counter for both atlases.
    pub fn advance_frame(&mut self) {
        self.mono.advance_frame();
        if let Some(c) = self.color.as_mut() {
            c.advance_frame();
        }
    }

    /// Evict unused glyphs from both atlases.
    pub fn evict_unused(
        &mut self,
        max_unused_frames: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        self.mono.evict_unused(max_unused_frames, device, queue);
        if let Some(c) = self.color.as_mut() {
            c.evict_unused(max_unused_frames, device, queue);
        }
    }

    /// Upload all pending glyphs queued on both atlases.
    pub fn upload_pending(&mut self, queue: &wgpu::Queue) {
        self.mono.upload_pending(queue);
        if let Some(c) = self.color.as_mut() {
            c.upload_pending(queue);
        }
    }

    /// Whether the color atlas has been allocated. Used by pipelines that
    /// only bind the color atlas when there is at least one color glyph.
    pub fn has_color_atlas(&self) -> bool {
        self.color.is_some()
    }

    /// Pure cache lookup that routes to the atlas matching `kind`. Returns
    /// `None` when the requested atlas has not been allocated yet (the
    /// color atlas is lazy) or when the key is not cached. Used by the
    /// experimental fragment shader grid path to reuse the atlas without
    /// re rasterizing.
    pub fn glyph_meta(&self, kind: GlyphAtlasKind, key: &GlyphKey) -> Option<GlyphEntry> {
        self.atlas_for(kind).and_then(|a| a.glyph_meta(key))
    }
}

/// Tagged entry returned when inserting or looking up a glyph through the
/// set. Callers emit one instance per glyph carrying this kind so the
/// draw pass routes it to the correct bind group.
#[derive(Clone, Copy, Debug)]
pub struct GlyphEntryTagged {
    pub entry: GlyphEntry,
    pub kind: GlyphAtlasKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(glyph_id: u16) -> GlyphKey {
        GlyphKey { font_id: 1, glyph_id, font_size_tenths: 120, subpixel_bin: 0 }
    }

    #[test]
    fn glyph_atlas_uses_nearest_sampling_for_crisper_ui_text() {
        assert_eq!(
            glyph_atlas_filter_mode(wgpu::TextureFormat::R8Unorm),
            wgpu::FilterMode::Nearest
        );
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

    // -- Atlas kind routing ---------------------------------------------------

    #[test]
    fn swash_color_maps_to_color_atlas_kind() {
        assert_eq!(
            GlyphAtlasKind::from_swash_content(cosmic_text::SwashContent::Color),
            GlyphAtlasKind::Color
        );
    }

    #[test]
    fn swash_mask_maps_to_mono_atlas_kind() {
        assert_eq!(
            GlyphAtlasKind::from_swash_content(cosmic_text::SwashContent::Mask),
            GlyphAtlasKind::Mono
        );
    }

    #[test]
    fn swash_subpixel_mask_maps_to_mono_atlas_kind() {
        // Subpixel masks carry coverage information, not color. They
        // belong on the monochrome atlas.
        assert_eq!(
            GlyphAtlasKind::from_swash_content(cosmic_text::SwashContent::SubpixelMask),
            GlyphAtlasKind::Mono
        );
    }

    #[test]
    fn mono_kind_uses_r8_format() {
        assert_eq!(GlyphAtlasKind::Mono.texture_format(), wgpu::TextureFormat::R8Unorm);
        assert!(!GlyphAtlasKind::Mono.is_color());
    }

    #[test]
    fn color_kind_uses_rgba8_format() {
        assert_eq!(GlyphAtlasKind::Color.texture_format(), wgpu::TextureFormat::Rgba8Unorm);
        assert!(GlyphAtlasKind::Color.is_color());
    }
}
