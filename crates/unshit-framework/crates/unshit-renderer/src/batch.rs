use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::sync::OnceLock;

use crate::atlas::{GlyphAtlas, GlyphEntry, GlyphKey};
use crate::canvas::{CanvasCallback, CanvasRegistry};
#[cfg(target_os = "windows")]
use crate::dw_rasterizer::DwRasterizer;
use crate::line_quad_cache::{hash_row_cells, LineGeometrySig, LineQuadCache};
use crate::pipeline::image::ImageInstance;
use crate::pipeline::quad::{QuadInstance, MAX_GRADIENT_STOPS};
use crate::pipeline::text::GlyphInstance;
use crate::svg_cache::SvgTessCache;
use crate::svg_tess::SvgGeometry;
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache};

/// Glyph rasterization backends.
///
/// On Windows, carries both SwashCache (for CSS/UI text, where cosmic-text
/// metrics must match the rasterizer) and DwRasterizer (for the terminal
/// grid, where characters sit on a fixed cell grid and DirectWrite quality
/// matters most).
///
/// On non-Windows, only SwashCache is available.
pub struct Rasterizer<'a> {
    pub swash: &'a mut SwashCache,
    #[cfg(target_os = "windows")]
    pub dw: &'a DwRasterizer,
}
use rustc_hash::{FxHashMap, FxHashSet};
use unshit_core::cell_grid::{CellAttrs, CellGrid};
use unshit_core::cursor::CursorShape;
use unshit_core::dirty::DirtyFlags;
use unshit_core::element::{ElementContent, InputType, Tag};
use unshit_core::event::TextSelection;
use unshit_core::id::NodeId;
use unshit_core::layout::TextMeasureCache;
use unshit_core::scroll::{self, ScrollbarVisualState};
use unshit_core::style::types::{
    Background, Color, CssPosition, CssResize, Display, FilterFunction, GradientStopPosition,
    Layer, LinearGradient, Overflow, RadialGradient, RadialShape, RenderTarget, TextDecoration,
    Visibility, WhiteSpace,
};
use unshit_core::svg::types::{SvgAttrs, SvgNode, SvgPrimitive, SvgTransform, ViewBox};
use unshit_core::trace::{append_terminal_trace_line, terminal_trace_enabled};
use unshit_core::tree::NodeArena;

/// Zero value for the gradient stop color slots in `QuadInstance`. Used at
/// every call site that emits a solid color quad to keep the instance layout
/// uniform.
pub(crate) const EMPTY_GRADIENT_STOP_COLORS: [[f32; 4]; MAX_GRADIENT_STOPS] =
    [[0.0; 4]; MAX_GRADIENT_STOPS];
pub(crate) const EMPTY_GRADIENT_STOP_POSITIONS: [f32; MAX_GRADIENT_STOPS] =
    [0.0; MAX_GRADIENT_STOPS];
/// Zero value for the radial specific aux slot. Solid color quads and
/// linear gradients leave it untouched; only radial gradient quads write
/// real `(center, radii)` values into the slot.
pub(crate) const EMPTY_GRADIENT_EXTRA: [f32; 4] = [0.0; 4];

/// One shot flag so the stop truncation warning does not spam logs.
static GRADIENT_TRUNCATE_WARNED: AtomicBool = AtomicBool::new(false);
static LAST_TERMINAL_RENDER_TRACE_HASH: AtomicU64 = AtomicU64::new(0);

fn terminal_grid_trace_hash(grid: &CellGrid) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    grid.rows().hash(&mut hasher);
    grid.cols().hash(&mut hasher);
    grid.cursor_row().hash(&mut hasher);
    grid.cursor_col().hash(&mut hasher);
    grid.cursor_visible().hash(&mut hasher);
    for row in grid.debug_rows(4, 96) {
        row.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(target_os = "windows")]
fn use_directwrite_grid_rasterization() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TM_FORCE_DIRECTWRITE_GRID").is_some())
}

#[inline]
fn atlas_font_namespace(cache_key: &cosmic_text::CacheKey) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache_key.font_id.hash(&mut hasher);
    cache_key.flags.hash(&mut hasher);
    hasher.finish()
}

/// Compute the projected axis length of a gradient inside a box of size
/// `(w, h)` for the given angle in degrees.
///
/// CSS Images Level 3 defines the axis length as the distance between the
/// two opposite corners of the gradient box along the gradient direction.
/// For an axis aligned 0deg / 180deg gradient this collapses to `h`; for
/// 90deg / 270deg it collapses to `w`. Diagonals fall out of the standard
/// `|w * sin| + |h * cos|` formula. The result is clamped to a tiny
/// positive value so the per stop pixel normalization never divides by
/// zero on a degenerate element.
fn projected_axis_length(angle_deg: f32, w: f32, h: f32) -> f32 {
    let rad = angle_deg.to_radians();
    let s = rad.sin().abs();
    let c = rad.cos().abs();
    (w * s + h * c).max(1e-6)
}

/// Pack the shared stop list of either a linear or a radial gradient into
/// the GPU side stop arrays.
///
/// Returns the number of stops actually written (capped at
/// `MAX_GRADIENT_STOPS`). The `opacity` multiplier is folded into each stop's
/// alpha so the shader does not need an extra uniform path for opacity of
/// gradient backgrounds. Gradients with more than `MAX_GRADIENT_STOPS` stops
/// are truncated; the first time this happens we emit a one time warning.
///
/// `axis_length` is the projection length the shader uses to interpret pixel
/// stop positions. For linear gradients pass the corner to corner length
/// from `projected_axis_length`. For radial gradients pass the resolved rx so
/// pixel stop positions express distance along the gradient line in pixels.
fn pack_stop_list(
    stops: &[unshit_core::style::types::GradientStop],
    opacity: f32,
    axis_length: f32,
    colors: &mut [[f32; 4]; MAX_GRADIENT_STOPS],
    positions: &mut [f32; MAX_GRADIENT_STOPS],
    kind_label: &str,
) -> usize {
    if stops.len() > MAX_GRADIENT_STOPS && !GRADIENT_TRUNCATE_WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "unshit-renderer: {} has {} stops, truncating to {} (further warnings suppressed)",
            kind_label,
            stops.len(),
            MAX_GRADIENT_STOPS
        );
    }

    let count = stops.len().min(MAX_GRADIENT_STOPS);
    let safe_axis = axis_length.max(1e-6);
    let mut prev: f32 = 0.0;
    for (i, stop) in stops.iter().take(count).enumerate() {
        let mut c = stop.color.to_linear_f32();
        c[3] *= opacity;
        colors[i] = c;
        let raw = match stop.position {
            GradientStopPosition::Percent(v) => v,
            GradientStopPosition::Px(v) => (v / safe_axis).max(0.0),
        };
        // Cross unit monotonic clamp. After parse time each unit is sorted
        // independently, so a percent stop followed by a pixel stop can
        // still produce an out of order pair once both are normalized.
        // Snap any out of order entry up to the previous normalized value
        // so the shader's segment scan stays linear.
        let snapped = if i == 0 { raw.max(0.0) } else { raw.max(prev) };
        positions[i] = snapped;
        prev = snapped;
    }
    count
}

/// Pack a `LinearGradient` into the GPU side stop arrays.
///
/// Returns `(colors, positions, params)` where `params` is the
/// `gradient_params` vec4 storing `[angle_radians, repeating_flag, 0,
/// stop_count]`. The `repeating_flag` slot is 0.0 for a non repeating
/// gradient and 1.0 for a repeating one (issue #128); the shader branches
/// on it to choose between the `clamp` and `fract` sampling paths. The
/// `opacity` multiplier is folded into each stop's alpha so the shader does
/// not need an extra uniform path for opacity of gradient backgrounds.
///
/// Pixel space stop positions are normalized against the projected axis
/// length so the shader always sees positions in the same `[0.0, 1.0]`
/// space regardless of whether the source CSS used `%` or `px`.
fn pack_gradient(
    grad: &LinearGradient,
    opacity: f32,
    elem_w: f32,
    elem_h: f32,
) -> ([[f32; 4]; MAX_GRADIENT_STOPS], [f32; MAX_GRADIENT_STOPS], [f32; 4]) {
    let mut colors = EMPTY_GRADIENT_STOP_COLORS;
    let mut positions = EMPTY_GRADIENT_STOP_POSITIONS;
    let axis_length = projected_axis_length(grad.angle_deg, elem_w, elem_h);
    let count = pack_stop_list(
        &grad.stops,
        opacity,
        axis_length,
        &mut colors,
        &mut positions,
        "linear-gradient",
    );
    let angle_rad = grad.angle_deg.to_radians();
    let repeating_flag = if grad.repeating { 1.0 } else { 0.0 };
    // gradient_params.w > 0 marks a linear gradient with `count` stops;
    // gradient_params.y is the repeating flag (issue #128).
    let params = [angle_rad, repeating_flag, 0.0, count as f32];
    (colors, positions, params)
}

/// Pack a `RadialGradient` into the GPU side stop arrays plus the radial
/// auxiliary slot.
///
/// Returns `(colors, positions, params, extra)`:
/// * `params.w` is `-(count as f32)` so the shader can dispatch on the
///   sign to enter the radial branch (see `QuadInstance` docs).
/// * `params.y` is `-1.0` for a circle, `1.0` for an ellipse. The shader
///   uses the sign to pick the isotropic vs anisotropic distance function.
/// * `extra` carries `[center_x, center_y, radius_x, radius_y]` in element
///   local pixels, resolved against the rect width and height. A degenerate
///   radius (`<= 0`) is left as is and the shader collapses the gradient
///   to the last stop color.
fn pack_radial_gradient(
    grad: &RadialGradient,
    width: f32,
    height: f32,
    opacity: f32,
) -> ([[f32; 4]; MAX_GRADIENT_STOPS], [f32; MAX_GRADIENT_STOPS], [f32; 4], [f32; 4]) {
    let mut colors = EMPTY_GRADIENT_STOP_COLORS;
    let mut positions = EMPTY_GRADIENT_STOP_POSITIONS;
    let resolved = grad.resolve(width, height);
    // Pixel positions on a radial gradient are distances along the gradient
    // line from center to rx, so we normalize against rx (clamped to a tiny
    // positive value upstream by `pack_stop_list`).
    let count = pack_stop_list(
        &grad.stops,
        opacity,
        resolved.rx.max(1e-6),
        &mut colors,
        &mut positions,
        "radial-gradient",
    );
    let shape_tag = match resolved.shape {
        RadialShape::Circle => -1.0,
        RadialShape::Ellipse => 1.0,
    };
    // gradient_params.w < 0 marks a radial gradient with `count` stops.
    let params = [0.0, shape_tag, 0.0, -(count as f32)];
    let extra = [resolved.center_x, resolved.center_y, resolved.rx, resolved.ry];
    (colors, positions, params, extra)
}

pub struct ImageBatch {
    pub path: String,
    pub instances: Vec<ImageInstance>,
    pub object_fit: unshit_core::style::types::ObjectFit,
    pub object_position: unshit_core::style::types::ObjectPosition,
}

/// One queued SVG draw. `geometry` is an `Arc` pointer into the tessellation
/// cache; `translate` and `scale` map the local SVG user units into pixel
/// coordinates, `clip_rect` is the pixel space scissor rectangle inherited
/// from containing overflow clips, `color_tint` is a per draw multiplier
/// (used to honor element level `color` inheritance for `currentColor`), and
/// `opacity` is the multiplied element opacity.
#[derive(Clone)]
pub struct SvgDrawCall {
    pub geometry: Arc<SvgGeometry>,
    pub translate: [f32; 2],
    pub scale: [f32; 2],
    pub clip_rect: [f32; 4],
    pub color_tint: [f32; 4],
    pub opacity: f32,
}

/// Marker emitted per element that has a resolved `backdrop-filter`.
///
/// The GPU render loop consumes these markers, splits the current pass at the
/// point the element would draw, copies the pixels in `rect` into an offscreen
/// texture, runs a two pass separable Gaussian blur, and then reopens the
/// pass with `LoadOp::Load` so the filtered element draws on top of the
/// blurred backdrop.
///
/// Boundaries are tagged at batch build time with the prefix counts for each
/// primitive type in the layer at the moment the filtered element is about
/// to draw. The render loop uses those counts to know exactly how many
/// entries of each primitive type of the current layer to emit before
/// splitting the pass.
#[derive(Clone, Copy, Debug)]
pub struct BackdropBoundary {
    /// The element bounding rectangle in pixel space, intersected with the
    /// inherited clip rect.
    pub rect: [f32; 4],
    /// The inherited clip rectangle in pixel space.
    pub clip_rect: [f32; 4],
    /// Gaussian kernel radius in pixels, already clamped to `[0, 64]`.
    pub blur_radius: f32,
    /// Prefix counts of each primitive type in the current layer at the
    /// moment this boundary was emitted. The render loop flushes exactly
    /// these many primitives before running the blur passes.
    pub quad_prefix: u32,
    pub glyph_prefix: u32,
    pub svg_prefix: u32,
    pub image_batch_prefix: u32,
    pub canvas_prefix: u32,
    /// Always `true` today. Future hook for #117 item 8: when damage
    /// tracking lands, a clean boundary can reuse a cached blurred texture
    /// without rerunning the blur passes.
    pub dirty: bool,
}

/// Identifies which primitive type a draw span covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawKind {
    Quad,
    Glyph,
}

/// A contiguous range of primitives to draw in a specific order.
/// The GPU render loop processes spans sequentially, switching pipelines
/// as needed, to preserve correct occlusion (painter's algorithm).
#[derive(Clone, Copy, Debug)]
pub struct DrawSpan {
    pub kind: DrawKind,
    pub start: u32,
    pub count: u32,
}

pub struct FrameBatch {
    pub quad_instances: Vec<QuadInstance>,
    pub glyph_instances: Vec<GlyphInstance>,
    pub image_batches: Vec<ImageBatch>,
    pub canvas_callbacks: Vec<CanvasCallback>,
    pub svg_draws: Vec<SvgDrawCall>,
    /// Backdrop filter boundaries collected during the walk, in draw order.
    /// When empty the renderer fast path runs with zero extra cost.
    pub backdrop_boundaries: Vec<BackdropBoundary>,
    /// Draw spans for interleaved quad/glyph rendering to fix text occlusion.
    /// When non-empty, the GPU render loop processes spans sequentially
    /// instead of rendering all quads then all glyphs.
    pub draw_spans: Vec<DrawSpan>,
}

impl Default for FrameBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameBatch {
    pub fn new() -> Self {
        Self {
            quad_instances: Vec::with_capacity(4096),
            glyph_instances: Vec::with_capacity(16384),
            image_batches: Vec::new(),
            canvas_callbacks: Vec::new(),
            svg_draws: Vec::new(),
            backdrop_boundaries: Vec::new(),
            draw_spans: Vec::with_capacity(1024),
        }
    }

    pub fn clear(&mut self) {
        self.quad_instances.clear();
        self.glyph_instances.clear();
        self.image_batches.clear();
        self.canvas_callbacks.clear();
        self.svg_draws.clear();
        self.backdrop_boundaries.clear();
        self.draw_spans.clear();
    }
}

pub struct LayeredBatch {
    pub layers: [FrameBatch; Layer::COUNT],
}

impl LayeredBatch {
    pub fn new() -> Self {
        Self { layers: std::array::from_fn(|_| FrameBatch::new()) }
    }

    pub fn clear(&mut self) {
        for layer in &mut self.layers {
            layer.clear();
        }
    }

    pub fn layer_mut(&mut self, layer: Layer) -> &mut FrameBatch {
        &mut self.layers[layer as usize]
    }

    /// Returns `true` if any layer has at least one `BackdropBoundary`.
    ///
    /// The GPU render loop uses this as the single source of truth for the
    /// fast path: when no layer carries a boundary, the renderer runs the
    /// existing single pass code path and allocates nothing.
    pub fn has_backdrop_boundaries(&self) -> bool {
        self.layers.iter().any(|l| !l.backdrop_boundaries.is_empty())
    }
}

impl Default for LayeredBatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached shaped glyph data. Positions are relative to the text origin (0,0).
/// Absolute positioning and color are applied at emission time.
#[derive(Clone)]
struct CachedGlyph {
    rel_x: f32, // physical.x + bearing offset
    rel_y: f32, // run_y + physical.y + bearing offset
    atlas_key: GlyphKey,
}

/// Pre-shaped text result cached to avoid re-creating cosmic-text Buffers.
#[derive(Clone)]
struct ShapedTextEntry {
    glyphs: Vec<CachedGlyph>,
}

/// Cache for shaped text layouts. Keyed on text content + font params + width.
pub struct ShapedTextCache {
    map: FxHashMap<ShapedCacheKey, ShapedTextEntry>,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct ShapedCacheKey {
    text_hash: u64,
    font_size_tenths: i32,
    line_height_tenths: i32,
    letter_spacing_tenths: i32,
    max_width_tenths: i32,
}

impl Default for ShapedTextCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ShapedTextCache {
    pub fn new() -> Self {
        Self { map: FxHashMap::with_capacity_and_hasher(256, Default::default()) }
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// Records the primitives produced by a single node (and its subtree) in the
/// previous frame for a specific layer. Used by `BatchCache` to replay cached
/// output for clean (non-dirty) nodes without rebuilding.
#[derive(Clone, Default)]
pub struct BatchRange {
    pub quads: Vec<QuadInstance>,
    pub glyphs: Vec<GlyphInstance>,
    pub svgs: Vec<SvgDrawCall>,
    pub draw_spans: Vec<DrawSpan>,
    /// Unique glyph atlas keys used by this node range (including subtree).
    pub glyph_keys: Vec<GlyphKey>,
    /// Glyph atlas generation this range was built against.
    pub atlas_generation: u64,
}

/// Per-frame cache that stores the actual `QuadInstance` and `GlyphInstance`
/// data emitted by each node in the previous frame.  When a node has neither
/// `PAINT` nor `SUBTREE_PAINT` the batch builder extends the current batch
/// with the stored instances instead of regenerating them.
///
/// The cache is keyed on `(NodeId, layer_index)` so nodes that appear on
/// multiple layers (portals, overlays) each get their own entry.
pub struct BatchCache {
    /// Stores instances produced by each `(node_id, layer_index)` last frame.
    ranges: FxHashMap<(NodeId, usize), BatchRange>,
    /// Staging map built during the current frame; swapped in at commit time.
    pending: FxHashMap<(NodeId, usize), BatchRange>,
}

impl Default for BatchCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchCache {
    pub fn new() -> Self {
        Self {
            ranges: FxHashMap::with_capacity_and_hasher(512, Default::default()),
            pending: FxHashMap::with_capacity_and_hasher(512, Default::default()),
        }
    }

    /// Clear all cached state.  Call when the entire frame must be rebuilt
    /// (e.g. after a window resize or stylesheet reload).
    pub fn clear(&mut self) {
        self.ranges.clear();
        self.pending.clear();
    }

    /// Begin building cache data for the current frame.  Clears the staging
    /// map so stale entries from the previous frame's build do not accumulate.
    pub fn begin_frame(&mut self) {
        self.pending.clear();
    }

    /// Commit the current frame's staging data as the authoritative cache for
    /// the next frame.  Call this after `build_render_batch` completes.
    pub fn commit_frame(&mut self) {
        std::mem::swap(&mut self.ranges, &mut self.pending);
        self.pending.clear();
    }

    /// Record the quads, glyphs, SVG draws, and draw spans emitted for
    /// `node_id` on `layer_index` during the current frame into the staging map.
    pub fn record(
        &mut self,
        node_id: NodeId,
        layer_index: usize,
        quads: Vec<QuadInstance>,
        glyphs: Vec<GlyphInstance>,
        svgs: Vec<SvgDrawCall>,
        draw_spans: Vec<DrawSpan>,
        glyph_keys: Vec<GlyphKey>,
        atlas_generation: u64,
    ) {
        self.pending.insert(
            (node_id, layer_index),
            BatchRange { quads, glyphs, svgs, draw_spans, glyph_keys, atlas_generation },
        );
    }

    /// Retrieve the cached instances for `node_id` on `layer_index` from the
    /// **previous** frame, if any.
    pub fn get(&self, node_id: NodeId, layer_index: usize) -> Option<&BatchRange> {
        self.ranges.get(&(node_id, layer_index))
    }

    /// Read a cached range AND carry the entry forward into the current
    /// frame's staging map. Call this from the batch builder's replay path
    /// so clean nodes that replay from cache are not silently dropped by
    /// the next `commit_frame` swap.
    ///
    /// Without this carry-forward, the swap-and-clear pattern in
    /// `commit_frame` produces an alternating hit/miss cycle: a replayed
    /// entry never reaches `pending`, so it vanishes on the next commit
    /// and the walker must re-render fresh the frame after. That churn is
    /// wasted work and, combined with out-of-band `computed_style`
    /// mutations from transition and animation ticks, shows up as visible
    /// flicker in the app (issues #41 and #42).
    ///
    /// Preference order inside a single frame:
    /// 1. If `pending` already has an entry for this key (the caller
    ///    recorded fresh primitives earlier in the same walk), return it
    ///    unchanged.
    /// 2. Otherwise, if `ranges` has an entry from the previous frame,
    ///    clone it into `pending` and return the cloned reference.
    /// 3. Otherwise return `None` so the walker knows to render fresh.
    pub fn replay(
        &mut self,
        node_id: NodeId,
        layer_index: usize,
        atlas_generation: u64,
    ) -> Option<&BatchRange> {
        let key = (node_id, layer_index);
        if !self.pending.contains_key(&key) {
            let range = self.ranges.get(&key)?.clone();
            if range.atlas_generation != atlas_generation {
                return None;
            }
            self.pending.insert(key, range);
        }
        self.pending.get(&key)
    }
}

impl ShapedTextCache {
    fn make_key(
        text: &str,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
    ) -> ShapedCacheKey {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        ShapedCacheKey {
            text_hash: hasher.finish(),
            font_size_tenths: (font_size * 10.0) as i32,
            line_height_tenths: (line_height * 10.0) as i32,
            letter_spacing_tenths: (letter_spacing * 10.0) as i32,
            max_width_tenths: max_width.map_or(-1, |w| (w * 10.0) as i32),
        }
    }
}

// ---------------------------------------------------------------------------
// ShapeCache
//
// Persistent, cross-frame cache of shaped prototype glyphs for the terminal
// grid. The hot path in `emit_grid_cells` shapes every unique character it
// encounters once per frame; with this cache warmed the shaping call happens
// at most once per character per font/size/style combination for the lifetime
// of the cache.
//
// Hits bypass `buffer.set_text` and `shape_until_scroll` entirely.
// ---------------------------------------------------------------------------

/// Identity tag for a shaped glyph configuration. Cache entries are only
/// reused for keys that match byte-for-byte.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShapeCacheKey {
    pub ch: char,
    pub font_id: u64,
    pub style: u64,
    pub font_size_tenths: u32,
}

/// Cache-friendly copy of the shaped `cosmic_text::LayoutGlyph` and its
/// positioning baseline. Only fields needed to reproduce the physical glyph
/// each frame are retained, so the entry is both small and independent of
/// any `Buffer` / `FontSystem` state across frames.
#[derive(Clone)]
pub struct ShapedGlyphEntry {
    pub layout_glyph: cosmic_text::LayoutGlyph,
    pub line_y: f32,
}

/// Cross-frame cache of shaped prototype glyphs, keyed on
/// `(char, font_id, style, font_size)`. The key is independent of subpixel
/// bin and cell origin so it remains valid as the pane scrolls or moves.
///
/// Invalidation is coarse on purpose: any change to the global font stack,
/// DPI scale, or font size should call [`ShapeCache::clear`]. Per-character
/// invalidation is intentionally not supported because monospace advances
/// are stable inside a single font configuration.
pub struct ShapeCache {
    entries: FxHashMap<ShapeCacheKey, Option<ShapedGlyphEntry>>,
    hits: u64,
    misses: u64,
    /// Identity of the cache configuration used to populate entries. When
    /// any of these shift we drop everything; see [`ShapeCache::retune`].
    configured_font_id: u64,
    configured_scale_thousandths: u32,
    configured_font_size_tenths: u32,
}

impl Default for ShapeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ShapeCache {
    pub fn new() -> Self {
        Self {
            // Sized for 95 ASCII + typical box-drawing + growth headroom.
            entries: FxHashMap::with_capacity_and_hasher(256, Default::default()),
            hits: 0,
            misses: 0,
            configured_font_id: 0,
            configured_scale_thousandths: 0,
            configured_font_size_tenths: 0,
        }
    }

    /// Hits since the most recent [`ShapeCache::clear`].
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Misses since the most recent [`ShapeCache::clear`].
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Rough cache hit rate in the range 0.0..=1.0. Returns 0.0 when no
    /// lookups have been recorded yet.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries have been stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every entry and resets hit/miss counters. Call when font
    /// identity, DPI scale, or size changes.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// Ensure the cache matches the active font/scale/size configuration.
    /// If any input changed since the last call, the cache is dropped so
    /// stale shaping data from the old configuration cannot leak.
    ///
    /// Call this at the start of each frame before the first lookup.
    pub fn retune(&mut self, font_family: &str, scale_factor: f32, font_size: f32) {
        let font_id = shape_cache_font_id(font_family);
        let scale = (scale_factor * 1000.0).round() as u32;
        let size = (font_size * 10.0).round() as u32;

        if font_id != self.configured_font_id
            || scale != self.configured_scale_thousandths
            || size != self.configured_font_size_tenths
        {
            self.clear();
            self.configured_font_id = font_id;
            self.configured_scale_thousandths = scale;
            self.configured_font_size_tenths = size;
        }
    }

    /// Look up a cached shaped entry. Records a hit or miss either way.
    /// Callers that miss must call [`ShapeCache::insert`] with the shaped
    /// result so the next frame hits.
    pub fn get(&mut self, key: &ShapeCacheKey) -> Option<&Option<ShapedGlyphEntry>> {
        if let Some(entry) = self.entries.get(key) {
            self.hits += 1;
            Some(entry)
        } else {
            self.misses += 1;
            None
        }
    }

    /// Insert a shaped result (including the `None` case where cosmic-text
    /// produced no glyph for the character) so subsequent frames hit.
    pub fn insert(&mut self, key: ShapeCacheKey, value: Option<ShapedGlyphEntry>) {
        self.entries.insert(key, value);
    }

    /// Range of characters preloaded by [`ShapeCache::preload_defaults`] in
    /// order: printable ASCII 0x20..=0x7e then the Unicode box-drawing block
    /// 0x2500..=0x257f. Used by tests and diagnostics.
    pub fn default_preload_chars() -> impl Iterator<Item = char> {
        (0x20u32..=0x7e).chain(0x2500..=0x257f).filter_map(char::from_u32)
    }

    /// Fill the cache with printable ASCII and the Unicode box-drawing block.
    /// Terminal workloads touch these characters first; preloading them
    /// pushes the cache hit rate above 95 percent within a screenful of
    /// output.
    ///
    /// Insert-only: on repeat calls existing entries are left untouched so
    /// this is safe to call more than once.
    pub fn preload_defaults(
        &mut self,
        shape: &mut dyn FnMut(char) -> (ShapeCacheKey, Option<ShapedGlyphEntry>),
    ) {
        for ch in Self::default_preload_chars() {
            let (key, value) = shape(ch);
            self.entries.entry(key).or_insert(value);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_render_batch(
    arena: &NodeArena,
    root: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    measure_cache: &mut TextMeasureCache,
    shaped_cache: &mut ShapedTextCache,
    svg_cache: &mut SvgTessCache,
    shape_cache: &mut ShapeCache,
    text_selection: Option<&TextSelection>,
    registry: Option<&CanvasRegistry>,
    scrollbar_state: &ScrollbarVisualState,
    focused: NodeId,
    batch_cache: &mut BatchCache,
    mut line_cache: Option<&mut LineQuadCache>,
) {
    let initial_clip = [0.0_f32, 0.0, 9999.0, 9999.0];
    let mut portals: Vec<(NodeId, Layer)> = Vec::new();
    walk_for_batch(
        arena,
        root,
        root,
        batch,
        atlas,
        font_system,
        rasterizer,
        measure_cache,
        shaped_cache,
        svg_cache,
        shape_cache,
        initial_clip,
        0.0,
        0.0,
        text_selection,
        registry,
        scrollbar_state,
        focused,
        Layer::Content,
        &mut portals,
        batch_cache,
        None,
        line_cache.as_deref_mut(),
    );

    // Process deferred portal nodes with fresh viewport clip
    for (portal_node, target_layer) in portals {
        walk_for_batch(
            arena,
            portal_node,
            root,
            batch,
            atlas,
            font_system,
            rasterizer,
            measure_cache,
            shaped_cache,
            svg_cache,
            shape_cache,
            initial_clip,
            0.0,
            0.0,
            text_selection,
            registry,
            scrollbar_state,
            focused,
            target_layer,
            &mut Vec::new(),
            batch_cache,
            None,
            line_cache.as_deref_mut(),
        );
    }
}

/// Flush accumulated primitives of `kind` into `draw_spans` if `end > cursor`.
/// Returns the new cursor position.
#[inline]
fn flush_span(spans: &mut Vec<DrawSpan>, kind: DrawKind, cursor: usize, end: usize) -> usize {
    if end > cursor {
        spans.push(DrawSpan { kind, start: cursor as u32, count: (end - cursor) as u32 });
    }
    end
}

/// Clear PAINT and SUBTREE_PAINT flags on every node in a subtree after a
/// frame has been rendered. Call this after `build_render_batch` completes.
/// This requires mutable access to the arena; kept as a separate step so
/// `walk_for_batch` can retain a shared `&NodeArena` borrow.
pub fn clear_paint_flags_subtree(arena: &mut NodeArena, node_id: NodeId) {
    let children = arena.children(node_id);
    for child_id in children {
        clear_paint_flags_subtree(arena, child_id);
    }
    if let Some(element) = arena.get_mut(node_id) {
        element.dirty.remove(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT);
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_for_batch(
    arena: &NodeArena,
    node_id: NodeId,
    root: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    measure_cache: &mut TextMeasureCache,
    shaped_cache: &mut ShapedTextCache,
    svg_cache: &mut SvgTessCache,
    shape_cache: &mut ShapeCache,
    clip_rect: [f32; 4],
    scroll_offset_x: f32,
    scroll_offset_y: f32,
    text_selection: Option<&TextSelection>,
    registry: Option<&CanvasRegistry>,
    scrollbar_state: &ScrollbarVisualState,
    focused: NodeId,
    current_layer: Layer,
    portals: &mut Vec<(NodeId, Layer)>,
    batch_cache: &mut BatchCache,
    parent_glyph_keys: Option<&mut FxHashSet<GlyphKey>>,
    mut line_cache: Option<&mut LineQuadCache>,
) {
    let Some(element) = arena.get(node_id) else {
        return;
    };

    let style = &element.computed_style;

    if style.display == Display::None || style.opacity == 0.0 {
        return;
    }

    // Portal: defer rendering to a later pass with fresh clip
    if let RenderTarget::Portal(target_layer) = style.render_target {
        portals.push((node_id, target_layer));
        return;
    }

    // Resolve effective layer before the damage check so we use the correct
    // layer index when looking up cached ranges.
    let effective_layer = if style.layer != Layer::Content { style.layer } else { current_layer };
    let layer_index = effective_layer as usize;

    // Damage-aware skip: if neither PAINT nor SUBTREE_PAINT is set, replay the
    // previously cached primitive instances and skip the entire subtree.
    //
    // `batch_cache.replay` both reads the cached range AND carries it
    // forward into the current frame's staging map, so the entry survives
    // the next `commit_frame` swap. Calling the pure `get` here would
    // leak the entry on the next commit and force clean nodes to alternate
    // between cache hits and forced re-renders every other frame.
    //
    // Replay is additionally gated on glyph atlas generation. Cached ranges
    // built against an older atlas generation are discarded so stale UVs are
    // never replayed after atlas eviction/repack.
    let node_dirty = element.dirty.intersects(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT);
    if !node_dirty {
        if let Some(cached) = batch_cache.replay(node_id, layer_index, atlas.generation) {
            for key in &cached.glyph_keys {
                atlas.touch(key);
            }
            let lb = batch.layer_mut(effective_layer);
            let quad_offset = lb.quad_instances.len() as u32;
            let glyph_offset = lb.glyph_instances.len() as u32;
            lb.quad_instances.extend_from_slice(&cached.quads);
            lb.glyph_instances.extend_from_slice(&cached.glyphs);
            lb.svg_draws.extend_from_slice(&cached.svgs);
            for span in &cached.draw_spans {
                let offset = match span.kind {
                    DrawKind::Quad => quad_offset,
                    DrawKind::Glyph => glyph_offset,
                };
                lb.draw_spans.push(DrawSpan {
                    kind: span.kind,
                    start: span.start + offset,
                    count: span.count,
                });
            }
            if let Some(parent_keys) = parent_glyph_keys {
                for key in &cached.glyph_keys {
                    parent_keys.insert(*key);
                }
            }
            return;
        }
        // No cached data available: fall through to render this node so it
        // gets populated in the cache for subsequent frames.
    }

    // Record where this node starts writing primitives so we can snapshot the
    // range after all children have been processed.
    let quad_start = batch.layer_mut(effective_layer).quad_instances.len();
    let glyph_start = batch.layer_mut(effective_layer).glyph_instances.len();
    let svg_start = batch.layer_mut(effective_layer).svg_draws.len();
    let span_start = batch.layer_mut(effective_layer).draw_spans.len();
    let mut node_glyph_keys: FxHashSet<GlyphKey> = FxHashSet::default();

    // Running cursors for draw span tracking. Updated after each flush.
    let mut quad_cursor = quad_start;
    let glyph_cursor = glyph_start;

    let is_visible = style.visibility == Visibility::Visible;

    let rect = element.layout_rect;
    let opacity = style.opacity;

    let render_x = rect.x - scroll_offset_x;
    let render_y = rect.y - scroll_offset_y;

    let clips_children = style.overflow != Overflow::Visible;
    let child_clip = if clips_children {
        let new_x = render_x.max(clip_rect[0]);
        let new_y = render_y.max(clip_rect[1]);
        let new_right = (render_x + rect.width).min(clip_rect[0] + clip_rect[2]);
        let new_bottom = (render_y + rect.height).min(clip_rect[1] + clip_rect[3]);
        [new_x, new_y, (new_right - new_x).max(0.0), (new_bottom - new_y).max(0.0)]
    } else {
        clip_rect
    };
    let (child_scroll_x, child_scroll_y) = if clips_children {
        (scroll_offset_x + element.scroll_x, scroll_offset_y + element.scroll_y)
    } else {
        (scroll_offset_x, scroll_offset_y)
    };

    if is_visible && style.outline_width > 0.0 && style.outline_color.a > 0 {
        let expand = style.outline_width + style.outline_offset;
        let mut oc = style.outline_color.to_linear_f32();
        oc[3] *= opacity;
        let outline_border = [style.outline_width; 4];

        batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
            pos: [render_x - expand, render_y - expand],
            size: [rect.width + expand * 2.0, rect.height + expand * 2.0],
            color: [0.0; 4], // no fill
            border_color: oc,
            border_width: outline_border,
            border_radius: style.border_radius.to_array(),
            clip_rect,
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
            gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
            gradient_params: [0.0; 4],
            gradient_extra: EMPTY_GRADIENT_EXTRA,
        });
    }

    // Backdrop filter boundary: tagged at the point the element is about to
    // draw its own background. The render loop uses the per primitive
    // prefix counts to flush all preceding draws, blur the framebuffer
    // contents behind the element, then reopen the pass so the element
    // draws on top. We only emit a boundary if the element has a non zero
    // area after clipping, mirroring the zero area edge case in the design.
    if is_visible {
        if let Some(ref backdrop) = style.backdrop_filter {
            let max_blur = backdrop
                .filters
                .iter()
                .map(|f| match f {
                    FilterFunction::Blur(r) => *r,
                })
                .fold(0.0_f32, f32::max);
            // A rounded to zero radius degenerates to a no op pass split.
            if max_blur > 0.5 && rect.width > 0.0 && rect.height > 0.0 {
                let elem_left = render_x.max(clip_rect[0]);
                let elem_top = render_y.max(clip_rect[1]);
                let elem_right = (render_x + rect.width).min(clip_rect[0] + clip_rect[2]);
                let elem_bottom = (render_y + rect.height).min(clip_rect[1] + clip_rect[3]);
                let elem_w = (elem_right - elem_left).max(0.0);
                let elem_h = (elem_bottom - elem_top).max(0.0);
                if elem_w > 0.0 && elem_h > 0.0 {
                    let lb = batch.layer_mut(effective_layer);
                    lb.backdrop_boundaries.push(BackdropBoundary {
                        rect: [elem_left, elem_top, elem_w, elem_h],
                        clip_rect,
                        blur_radius: max_blur,
                        quad_prefix: lb.quad_instances.len() as u32,
                        glyph_prefix: lb.glyph_instances.len() as u32,
                        svg_prefix: lb.svg_draws.len() as u32,
                        image_batch_prefix: lb.image_batches.len() as u32,
                        canvas_prefix: lb.canvas_callbacks.len() as u32,
                        dirty: true,
                    });
                }
            }
        }
    }

    if is_visible
        && (style.background.is_visible()
            || style.border_width.any_nonzero()
            || !style.box_shadow.is_empty())
    {
        // 1. Outer shadows go behind the background quad on the same layer,
        //    in declared order. CSS paints them in reverse so the first
        //    shadow lands on top; we mirror that by pushing outer shadows in
        //    reverse order so later pushes draw on top within the batch.
        for shadow in style.box_shadow.iter().rev() {
            if shadow.inset {
                continue;
            }
            if shadow.color.a == 0 {
                continue;
            }
            let mut sc = shadow.color.to_linear_f32();
            sc[3] *= opacity;
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [render_x, render_y],
                size: [rect.width, rect.height],
                color: [0.0; 4],
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: style.border_radius.to_array(),
                clip_rect,
                shadow_color: sc,
                shadow_offset: [shadow.offset_x, shadow.offset_y],
                shadow_params: [shadow.blur_radius, 0.0],
                shadow_spread: [shadow.spread_radius, 0.0],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
            });
        }

        // 2. Background + border quad (no embedded shadow).
        let mut bc = style.border_color.to_linear_f32();
        bc[3] *= opacity;

        let (bg, grad_stop_colors, grad_stop_positions, grad_params, grad_extra) =
            match &style.background {
                Background::Color(c) => {
                    let mut bg = c.to_linear_f32();
                    bg[3] *= opacity;
                    (
                        bg,
                        EMPTY_GRADIENT_STOP_COLORS,
                        EMPTY_GRADIENT_STOP_POSITIONS,
                        [0.0; 4],
                        EMPTY_GRADIENT_EXTRA,
                    )
                }
                Background::LinearGradient(grad) => {
                    let (colors, positions, params) =
                        pack_gradient(grad, opacity, rect.width, rect.height);
                    ([0.0; 4], colors, positions, params, EMPTY_GRADIENT_EXTRA)
                }
                Background::RadialGradient(grad) => {
                    let (colors, positions, params, extra) =
                        pack_radial_gradient(grad, rect.width, rect.height, opacity);
                    ([0.0; 4], colors, positions, params, extra)
                }
            };

        if style.background.is_visible() || style.border_width.any_nonzero() {
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [render_x, render_y],
                size: [rect.width, rect.height],
                color: bg,
                border_color: bc,
                border_width: style.border_width.to_array(),
                border_radius: style.border_radius.to_array(),
                clip_rect,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: grad_stop_colors,
                gradient_stop_positions: grad_stop_positions,
                gradient_params: grad_params,
                gradient_extra: grad_extra,
            });
        }

        // 3. Inset shadows go over the background, clipped to the padding
        //    box. The shader reads the inset flag from `shadow_params.y`.
        for shadow in style.box_shadow.iter() {
            if !shadow.inset {
                continue;
            }
            if shadow.color.a == 0 {
                continue;
            }
            let mut sc = shadow.color.to_linear_f32();
            sc[3] *= opacity;
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [render_x, render_y],
                size: [rect.width, rect.height],
                color: [0.0; 4],
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: style.border_radius.to_array(),
                clip_rect,
                shadow_color: sc,
                shadow_offset: [shadow.offset_x, shadow.offset_y],
                shadow_params: [shadow.blur_radius, 1.0],
                shadow_spread: [shadow.spread_radius, 0.0],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
            });
        }
    }

    // Flush background/outline/shadow/border quads before text emission.
    {
        let lb = batch.layer_mut(effective_layer);
        let end = lb.quad_instances.len();
        quad_cursor = flush_span(&mut lb.draw_spans, DrawKind::Quad, quad_cursor, end);
    }

    // Input element rendering
    if element.tag == Tag::Input && is_visible {
        let style = &element.computed_style;
        let content_w = rect.width - style.padding.left - style.padding.right;
        let content_h = rect.height - style.padding.top - style.padding.bottom;
        let text_x = render_x + style.padding.left;

        let input = &element.input_state;

        match input.input_type {
            InputType::Hidden => {
                // Nothing to render.
            }
            InputType::Checkbox | InputType::Radio => {
                // Both are rendered as a small square/circle (the outer box is
                // already drawn by the quad pass via CSS).  We just draw the
                // checkmark glyph or a filled dot when checked.
                if input.checked {
                    let glyph = if input.input_type == InputType::Checkbox {
                        "\u{2713}" // checkmark
                    } else {
                        "\u{25CF}" // black circle
                    };
                    let mut fg = style.color;
                    fg.a = (fg.a as f32 * opacity) as u8;
                    let (gw, gh) = unshit_core::layout::measure_text_cached(
                        glyph,
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        Some(content_w),
                        font_system,
                        Some(measure_cache),
                    );
                    let gx = render_x + (rect.width - gw) * 0.5;
                    let gy = render_y + (rect.height - gh) * 0.5;
                    emit_text_glyphs_cached(
                        glyph,
                        gx,
                        gy,
                        Some(content_w),
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        &fg,
                        clip_rect,
                        batch.layer_mut(effective_layer),
                        atlas,
                        font_system,
                        rasterizer,
                        shaped_cache,
                        Some(&mut node_glyph_keys),
                    );
                }
            }
            InputType::Range => {
                // Render a thin horizontal track and a circular thumb.
                let track_h = 4.0_f32;
                let thumb_r = (content_h * 0.5).min(8.0);
                let thumb_d = thumb_r * 2.0;

                let track_x = render_x + style.padding.left;
                let track_y = render_y + (rect.height - track_h) * 0.5;
                let track_w = content_w;

                // Track background.
                let mut track_color = style.color.to_linear_f32();
                track_color[3] *= opacity * 0.3;
                batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                    pos: [track_x, track_y],
                    size: [track_w, track_h],
                    color: track_color,
                    border_color: [0.0; 4],
                    border_width: [0.0; 4],
                    border_radius: [track_h * 0.5; 4],
                    clip_rect,
                    shadow_color: [0.0; 4],
                    shadow_offset: [0.0; 2],
                    shadow_params: [0.0; 2],
                    shadow_spread: [0.0; 2],
                    gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                    gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                    gradient_params: [0.0; 4],
                    gradient_extra: EMPTY_GRADIENT_EXTRA,
                });

                // Thumb position.
                let ratio = if input.max > input.min {
                    ((input.numeric_value - input.min) / (input.max - input.min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let thumb_cx = track_x + ratio * track_w;
                let thumb_x = thumb_cx - thumb_r;
                let thumb_y = render_y + (rect.height - thumb_d) * 0.5;

                let mut thumb_color = style.color.to_linear_f32();
                thumb_color[3] *= opacity;
                batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                    pos: [thumb_x, thumb_y],
                    size: [thumb_d, thumb_d],
                    color: thumb_color,
                    border_color: [0.0; 4],
                    border_width: [0.0; 4],
                    border_radius: [thumb_r; 4],
                    clip_rect,
                    shadow_color: [0.0; 4],
                    shadow_offset: [0.0; 2],
                    shadow_params: [0.0; 2],
                    shadow_spread: [0.0; 2],
                    gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                    gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                    gradient_params: [0.0; 4],
                    gradient_extra: EMPTY_GRADIENT_EXTRA,
                });
            }
            InputType::Text | InputType::Password | InputType::Number => {
                // For password, substitute bullet characters at render time.
                let masked: String;
                let is_placeholder = input.value.is_empty();
                let display_text = if is_placeholder {
                    element.placeholder.as_deref().unwrap_or("")
                } else if input.input_type == InputType::Password {
                    masked = "\u{2022}".repeat(input.value.chars().count());
                    &masked
                } else {
                    &input.value
                };

                if !display_text.is_empty() {
                    let mut text_color =
                        if is_placeholder { style.placeholder_color } else { style.color };
                    text_color.a = (text_color.a as f32 * opacity) as u8;

                    let (_, text_h) = unshit_core::layout::measure_text_cached(
                        display_text,
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        Some(content_w),
                        font_system,
                        Some(measure_cache),
                    );
                    let y_offset = ((content_h - text_h) * 0.5).max(0.0);
                    let text_y = render_y + style.padding.top + y_offset;

                    emit_text_glyphs_cached(
                        display_text,
                        text_x,
                        text_y,
                        Some(content_w),
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        &text_color,
                        clip_rect,
                        batch.layer_mut(effective_layer),
                        atlas,
                        font_system,
                        rasterizer,
                        shaped_cache,
                        Some(&mut node_glyph_keys),
                    );
                }

                // Render cursor (caret) when focused and cursor is visible
                if node_id == focused && element.cursor_state.visible {
                    // For password, measure prefix of masked text.
                    let cursor_x = if input.cursor_pos == 0 || input.value.is_empty() {
                        0.0
                    } else {
                        let prefix: String = if input.input_type == InputType::Password {
                            // Each char maps to one bullet.
                            let char_count = input.value[..input.cursor_pos].chars().count();
                            "\u{2022}".repeat(char_count)
                        } else {
                            input.value[..input.cursor_pos].to_string()
                        };
                        let (w, _) = unshit_core::layout::measure_text_cached(
                            &prefix,
                            style.font_size,
                            style.line_height,
                            style.letter_spacing,
                            Some(content_w),
                            font_system,
                            Some(measure_cache),
                        );
                        w
                    };

                    let caret_text = if input.value.is_empty() { " " } else { &input.value };
                    let (_, caret_text_h) = unshit_core::layout::measure_text_cached(
                        caret_text,
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        Some(content_w),
                        font_system,
                        Some(measure_cache),
                    );
                    let caret_y_offset = ((content_h - caret_text_h) * 0.5).max(0.0);
                    let caret_y = render_y + style.padding.top + caret_y_offset;
                    let caret_height = style.font_size * style.line_height;

                    // Determine cursor dimensions based on shape
                    let cursor_shape = style.caret_shape;
                    let (caret_w, caret_h, caret_pos_x, caret_pos_y) = match cursor_shape {
                        CursorShape::Block => {
                            let char_width = style.font_size * 0.6;
                            (char_width, caret_height, text_x + cursor_x, caret_y)
                        }
                        CursorShape::Beam => (2.0_f32, caret_height, text_x + cursor_x, caret_y),
                        CursorShape::Underline => {
                            let char_width = style.font_size * 0.6;
                            let underline_y = caret_y + caret_height - 2.0;
                            (char_width, 2.0_f32, text_x + cursor_x, underline_y)
                        }
                    };

                    let mut caret_color = style.caret_color.to_linear_f32();
                    if cursor_shape == CursorShape::Block {
                        caret_color[3] *= opacity * 0.5;
                    } else {
                        caret_color[3] *= opacity;
                    }

                    batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                        pos: [caret_pos_x, caret_pos_y],
                        size: [caret_w, caret_h],
                        color: caret_color,
                        border_color: [0.0; 4],
                        border_width: [0.0; 4],
                        border_radius: [0.0; 4],
                        clip_rect,
                        shadow_color: [0.0; 4],
                        shadow_offset: [0.0; 2],
                        shadow_params: [0.0; 2],
                        shadow_spread: [0.0; 2],
                        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                        gradient_params: [0.0; 4],
                        gradient_extra: EMPTY_GRADIENT_EXTRA,
                    });
                }
            }
        }
    } else {
        match &element.content {
            ElementContent::Text(ref text) if is_visible && !text.is_empty() => {
                let mut text_color = style.color;
                text_color.a = (text_color.a as f32 * opacity) as u8;

                let content_w = rect.width - style.padding.left - style.padding.right;
                let content_h = rect.height - style.padding.top - style.padding.bottom;

                let text_max_w =
                    if matches!(style.white_space, WhiteSpace::Nowrap | WhiteSpace::Pre) {
                        None
                    } else {
                        Some(content_w)
                    };

                let (text_w, text_h) = unshit_core::layout::measure_text_cached(
                    text,
                    style.font_size,
                    style.line_height,
                    style.letter_spacing,
                    text_max_w,
                    font_system,
                    Some(measure_cache),
                );
                let y_offset = ((content_h - text_h) * 0.5).max(0.0);

                let text_x = render_x + style.padding.left;
                let text_y = render_y + style.padding.top + y_offset;

                // Selection highlight rendering (emitted before text so it renders behind glyphs)
                if let Some(selection) = text_selection {
                    if !selection.is_collapsed() {
                        let byte_range = if selection.anchor_element == selection.focus_element {
                            if selection.anchor_element == node_id {
                                selection.single_element_range()
                            } else {
                                None
                            }
                        } else {
                            let anchor_order = unshit_core::event::document_order(
                                arena,
                                root,
                                selection.anchor_element,
                            );
                            let focus_order = unshit_core::event::document_order(
                                arena,
                                root,
                                selection.focus_element,
                            );
                            let node_order =
                                unshit_core::event::document_order(arena, root, node_id);

                            match (anchor_order, focus_order, node_order) {
                                (Some(ao), Some(fo), Some(no)) => {
                                    selection.element_byte_range(node_id, no, ao, fo, text.len())
                                }
                                _ => None,
                            }
                        };

                        if let Some((sel_start, sel_end)) = byte_range {
                            let line_ranges = unshit_core::layout::text_line_ranges(
                                text,
                                style.font_size,
                                style.line_height,
                                style.letter_spacing,
                                text_max_w,
                                sel_start,
                                sel_end,
                                font_system,
                            );

                            let sel_color = [0.2, 0.4, 0.8, 0.4];

                            for lr in &line_ranges {
                                batch.layer_mut(effective_layer).quad_instances.push(
                                    QuadInstance {
                                        pos: [text_x + lr.x, text_y + lr.y],
                                        size: [lr.width, lr.height],
                                        color: sel_color,
                                        border_color: [0.0; 4],
                                        border_width: [0.0; 4],
                                        border_radius: [0.0; 4],
                                        clip_rect,
                                        shadow_color: [0.0; 4],
                                        shadow_offset: [0.0; 2],
                                        shadow_params: [0.0; 2],
                                        shadow_spread: [0.0; 2],
                                        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                                        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                                        gradient_params: [0.0; 4],
                                        gradient_extra: EMPTY_GRADIENT_EXTRA,
                                    },
                                );
                            }
                        }
                    }
                }

                emit_text_glyphs_cached(
                    text,
                    text_x,
                    text_y,
                    text_max_w,
                    style.font_size,
                    style.line_height,
                    style.letter_spacing,
                    &text_color,
                    clip_rect,
                    batch.layer_mut(effective_layer),
                    atlas,
                    font_system,
                    rasterizer,
                    shaped_cache,
                    Some(&mut node_glyph_keys),
                );

                // Text decoration line rendering
                if style.text_decoration != TextDecoration::None {
                    let deco_color = style.text_decoration_color.unwrap_or(style.color);
                    let mut deco_color_linear = deco_color.to_linear_f32();
                    deco_color_linear[3] *= opacity;

                    let font_size = style.font_size;
                    let line_thickness = (font_size * 0.07).max(1.0);

                    let deco_y = match style.text_decoration {
                        TextDecoration::Underline => text_y + font_size * 0.85,
                        TextDecoration::LineThrough => text_y + font_size * 0.5,
                        TextDecoration::Overline => text_y,
                        TextDecoration::None => unreachable!(),
                    };

                    batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                        pos: [text_x, deco_y],
                        size: [text_w.min(content_w), line_thickness],
                        color: deco_color_linear,
                        border_color: [0.0; 4],
                        border_width: [0.0; 4],
                        border_radius: [0.0; 4],
                        clip_rect,
                        shadow_color: [0.0; 4],
                        shadow_offset: [0.0; 2],
                        shadow_params: [0.0; 2],
                        shadow_spread: [0.0; 2],
                        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                        gradient_params: [0.0; 4],
                        gradient_extra: EMPTY_GRADIENT_EXTRA,
                    });
                }
            }
            ElementContent::Image(ref path) if is_visible && !path.is_empty() => {
                let instance = ImageInstance {
                    pos: [render_x, render_y],
                    size: [rect.width, rect.height],
                    border_radius: style.border_radius.to_array(),
                    opacity,
                    _pad: [0.0; 3],
                    clip_rect,
                };
                let layer_batch = batch.layer_mut(effective_layer);
                if let Some(ib) = layer_batch
                    .image_batches
                    .iter_mut()
                    .find(|b| b.path == *path && b.object_fit == style.object_fit)
                {
                    ib.instances.push(instance);
                } else {
                    layer_batch.image_batches.push(ImageBatch {
                        path: path.clone(),
                        instances: vec![instance],
                        object_fit: style.object_fit,
                        object_position: style.object_position,
                    });
                }
            }
            ElementContent::Canvas if is_visible => {
                if let Some(registry) = registry {
                    if let Some(ref id) = element.id {
                        if let Some(painter) = registry.get(id) {
                            let canvas_node_id = registry.get_node_id(id);
                            batch.layer_mut(effective_layer).canvas_callbacks.push(
                                CanvasCallback {
                                    painter: Arc::clone(painter),
                                    rect: unshit_core::element::LayoutRect {
                                        x: render_x,
                                        y: render_y,
                                        width: rect.width,
                                        height: rect.height,
                                    },
                                    clip_rect,
                                    node_id: canvas_node_id,
                                },
                            );
                        }
                    }
                }
            }
            ElementContent::Grid(ref grid) if is_visible => {
                // cell_h derives from CSS line_height (the source of truth).
                let cell_h = style.font_size * style.line_height;
                // Grid cell width must match the active glyph shaping/raster
                // path. When TM_FORCE_DIRECTWRITE_GRID is off, the terminal
                // uses swash/cosmic-text on Windows too, so use the same
                // monospace width measurement there.
                #[cfg(target_os = "windows")]
                let cell_w = if use_directwrite_grid_rasterization() {
                    rasterizer.dw.measure_advance_width('M', style.font_size)
                } else {
                    measure_monospace_cell_width(font_system, style.font_size, cell_h)
                };
                #[cfg(not(target_os = "windows"))]
                let cell_w = measure_monospace_cell_width(font_system, style.font_size, cell_h);

                // Publish metrics so the app's resize handler can read them.
                unshit_core::cell_grid::CellGrid::publish_cell_metrics(cell_w, cell_h);

                // Compute grid dimensions from element size and cell metrics,
                // then publish a pending resize when they differ from the
                // current grid. This eliminates the timing gap where the
                // layout resize handler reads stale cell metrics.
                let content_w = rect.width - style.padding.left - style.padding.right;
                let content_h = rect.height - style.padding.top - style.padding.bottom;
                let cols = (content_w / cell_w).max(1.0) as u16;
                let rows = (content_h / cell_h).max(1.0) as u16;
                if cols as usize != grid.cols() || rows as usize != grid.rows() {
                    unshit_core::cell_grid::CellGrid::publish_pending_resize(cols, rows);
                }

                emit_grid_cells(
                    grid,
                    render_x + style.padding.left,
                    render_y + style.padding.top,
                    cell_w,
                    cell_h,
                    style.font_size,
                    opacity,
                    clip_rect,
                    batch.layer_mut(effective_layer),
                    atlas,
                    font_system,
                    rasterizer,
                    shape_cache,
                    Some(&mut node_glyph_keys),
                    node_id,
                    line_cache.as_deref_mut(),
                );
            }
            ElementContent::Svg(ref node) if is_visible => {
                emit_svg_node(
                    node,
                    &SvgAttrs::default(),
                    SvgTransform::IDENTITY,
                    node.attrs.view_box.unwrap_or_else(ViewBox::default),
                    render_x,
                    render_y,
                    rect.width,
                    rect.height,
                    style.color,
                    opacity,
                    clip_rect,
                    svg_cache,
                    batch.layer_mut(effective_layer),
                );
            }
            _ => {}
        }
    }

    // Flush text/content glyphs and any interleaved quads (selection
    // highlights, text decorations, input cursors) before child recursion.
    {
        let lb = batch.layer_mut(effective_layer);
        let qend = lb.quad_instances.len();
        let gend = lb.glyph_instances.len();
        let _ = flush_span(&mut lb.draw_spans, DrawKind::Quad, quad_cursor, qend);
        let _ = flush_span(&mut lb.draw_spans, DrawKind::Glyph, glyph_cursor, gend);
    }

    // Collect children into a Vec so we can sort by z-index.
    // Stable sort preserves DOM order for children with equal z-index.
    let mut children_ids: Vec<NodeId> = Vec::new();
    let mut needs_sort = false;
    {
        let mut c = element.first_child;
        while !c.is_dangling() {
            if !needs_sort {
                let z = arena.get(c).map(|e| e.computed_style.z_index).unwrap_or(0);
                if z != 0 {
                    needs_sort = true;
                }
            }
            children_ids.push(c);
            c = arena.get(c).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
    if needs_sort {
        children_ids
            .sort_by_key(|&id| arena.get(id).map(|e| e.computed_style.z_index).unwrap_or(0));
    }

    for &child in &children_ids {
        // Per CSS spec, absolutely positioned children escape their
        // parent's overflow clip and scroll offset.
        let (effective_clip, eff_scroll_x, eff_scroll_y) =
            if let Some(child_elem) = arena.get(child) {
                if matches!(
                    child_elem.computed_style.position,
                    CssPosition::Absolute | CssPosition::Fixed
                ) {
                    (clip_rect, scroll_offset_x, scroll_offset_y)
                } else {
                    (child_clip, child_scroll_x, child_scroll_y)
                }
            } else {
                (child_clip, child_scroll_x, child_scroll_y)
            };
        walk_for_batch(
            arena,
            child,
            root,
            batch,
            atlas,
            font_system,
            rasterizer,
            measure_cache,
            shaped_cache,
            svg_cache,
            shape_cache,
            effective_clip,
            eff_scroll_x,
            eff_scroll_y,
            text_selection,
            registry,
            scrollbar_state,
            focused,
            effective_layer,
            portals,
            batch_cache,
            Some(&mut node_glyph_keys),
            line_cache.as_deref_mut(),
        );
    }

    // Children have already flushed their own draw spans internally
    // during their recursive `walk_for_batch` calls.  Do NOT flush a
    // span here: it would create duplicate overlapping spans that
    // re-draw all children's quads then all glyphs, destroying the
    // interleaved draw order and causing text occlusion (earlier DOM
    // text renders on top of later DOM backgrounds).
    //
    // Just advance quad_cursor past children's contributions so the
    // scrollbar/grip flush below starts at the right offset.
    {
        let lb = batch.layer_mut(effective_layer);
        quad_cursor = lb.quad_instances.len();
    }

    // Snapshot the primitives emitted by this node and its subtree into the
    // pending cache.  Future frames can replay this data when the node is clean.
    // (draw_spans are snapshotted *after* scrollbar/grip emission below so the
    // cache replay is complete; see the second snapshot block.)

    // Overlay scrollbar rendering.
    // Emitted after children so the scrollbar draws on top of content.
    if style.overflow == Overflow::Scroll {
        let (v_geom, h_geom) =
            scroll::compute_scrollbar_geometry(arena, node_id, render_x, render_y);

        const TRACK_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.05];
        const CORNER_RADIUS: f32 = 3.0;

        let mut push_scrollbar_quad = |pos: [f32; 2], size: [f32; 2], color: [f32; 4]| {
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos,
                size,
                color,
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: [CORNER_RADIUS; 4],
                clip_rect: child_clip,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
            });
        };

        for geom in [v_geom.as_ref(), h_geom.as_ref()].into_iter().flatten() {
            let alpha = scrollbar_state.thumb_alpha(node_id, geom.axis);
            let thumb_color = [1.0, 1.0, 1.0, alpha];
            push_scrollbar_quad(
                [geom.track_x, geom.track_y],
                [geom.track_w, geom.track_h],
                TRACK_COLOR,
            );
            push_scrollbar_quad(
                [geom.thumb_x, geom.thumb_y],
                [geom.thumb_w, geom.thumb_h],
                thumb_color,
            );
        }
    }

    // Resize grip indicator.
    // Per CSS spec, `resize` only works when `overflow` is not `visible`.
    if style.resize != CssResize::None && style.overflow != Overflow::Visible {
        const GRIP_SIZE: f32 = 12.0;
        const DOT_SIZE: f32 = 2.0;
        const GRIP_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.35];

        let base_x = render_x + rect.width - GRIP_SIZE;
        let base_y = render_y + rect.height - GRIP_SIZE;

        // Three diagonal dots (bottom-right corner grip pattern).
        let dots: &[(f32, f32)] = &[
            (GRIP_SIZE - 3.0, GRIP_SIZE - 3.0),
            (GRIP_SIZE - 7.0, GRIP_SIZE - 3.0),
            (GRIP_SIZE - 3.0, GRIP_SIZE - 7.0),
            (GRIP_SIZE - 11.0, GRIP_SIZE - 3.0),
            (GRIP_SIZE - 7.0, GRIP_SIZE - 7.0),
            (GRIP_SIZE - 3.0, GRIP_SIZE - 11.0),
        ];
        for &(dx, dy) in dots {
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [base_x + dx, base_y + dy],
                size: [DOT_SIZE, DOT_SIZE],
                color: GRIP_COLOR,
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: [1.0; 4],
                clip_rect: child_clip,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
            });
        }
    }

    // Flush scrollbar/resize grip quads.
    {
        let lb = batch.layer_mut(effective_layer);
        let qend = lb.quad_instances.len();
        let _ = flush_span(&mut lb.draw_spans, DrawKind::Quad, quad_cursor, qend);
    }

    // Snapshot primitives and draw spans into the pending cache.
    {
        let lb = batch.layer_mut(effective_layer);
        let quads = lb.quad_instances[quad_start..].to_vec();
        let glyphs = lb.glyph_instances[glyph_start..].to_vec();
        let svgs = lb.svg_draws[svg_start..].to_vec();
        let quad_start_u32 = quad_start as u32;
        let glyph_start_u32 = glyph_start as u32;
        let spans = lb.draw_spans[span_start..]
            .iter()
            .map(|span| DrawSpan {
                kind: span.kind,
                start: match span.kind {
                    DrawKind::Quad => span.start.saturating_sub(quad_start_u32),
                    DrawKind::Glyph => span.start.saturating_sub(glyph_start_u32),
                },
                count: span.count,
            })
            .collect::<Vec<_>>();
        let glyph_keys = node_glyph_keys.iter().copied().collect::<Vec<_>>();
        batch_cache.record(
            node_id,
            layer_index,
            quads,
            glyphs,
            svgs,
            spans,
            glyph_keys,
            atlas.generation,
        );
    }
    if let Some(parent_keys) = parent_glyph_keys {
        for key in node_glyph_keys {
            parent_keys.insert(key);
        }
    }
}

/// Walk an `SvgNode` tree, accumulate the SVG presentation cascade and
/// transform, tessellate each leaf primitive via the shared cache, and emit
/// one `SvgDrawCall` per leaf into the active layer batch.
#[allow(clippy::too_many_arguments)]
fn emit_svg_node(
    node: &SvgNode,
    parent_attrs: &SvgAttrs,
    parent_transform: SvgTransform,
    root_view_box: ViewBox,
    root_x: f32,
    root_y: f32,
    root_w: f32,
    root_h: f32,
    current_color: Color,
    opacity: f32,
    clip_rect: [f32; 4],
    svg_cache: &mut SvgTessCache,
    batch: &mut FrameBatch,
) {
    // The node level attributes cascade over the parent group attrs.
    let cascaded = node.attrs.cascade_over(parent_attrs);
    // The node transform (if any) composes onto the parent transform. Note
    // that cascade_over also composes the transform, so we use the cascaded
    // value directly here.
    let transform = cascaded.transform.unwrap_or(SvgTransform::IDENTITY);
    let _ = parent_transform;

    if matches!(node.primitive, SvgPrimitive::Group) {
        for child in &node.children {
            emit_svg_node(
                child,
                &cascaded,
                transform,
                root_view_box,
                root_x,
                root_y,
                root_w,
                root_h,
                current_color,
                opacity,
                clip_rect,
                svg_cache,
                batch,
            );
        }
        return;
    }

    use unshit_core::svg::types::SvgPaint;
    let effective_fill = match cascaded.fill.unwrap_or(SvgPaint::Solid(Color::BLACK)) {
        SvgPaint::None => Color::TRANSPARENT,
        SvgPaint::Current => current_color,
        SvgPaint::Solid(c) => c,
    };
    let effective_stroke = match cascaded.stroke.unwrap_or(SvgPaint::None) {
        SvgPaint::None => Color::TRANSPARENT,
        SvgPaint::Current => current_color,
        SvgPaint::Solid(c) => c,
    };

    let geometry = svg_cache.get_or_tessellate(
        &node.primitive,
        &cascaded,
        current_color,
        effective_fill,
        effective_stroke,
    );
    if geometry.is_empty() {
        return;
    }

    // Map the viewBox onto the rendered rectangle. Non uniform scale is
    // allowed and produces visibly stretched strokes, matching browser SVG
    // behavior. Transforms on groups (already composed into `transform`)
    // apply in viewBox space before the viewport scale.
    let vb_w = if root_view_box.width > 0.0 { root_view_box.width } else { 1.0 };
    let vb_h = if root_view_box.height > 0.0 { root_view_box.height } else { 1.0 };
    let scale_x = root_w / vb_w;
    let scale_y = root_h / vb_h;
    let translate_x = root_x - root_view_box.min_x * scale_x + transform.e * scale_x;
    let translate_y = root_y - root_view_box.min_y * scale_y + transform.f * scale_y;

    // For now we only use the translate components of the SVG transform
    // plus the viewport scale. Rotation and non axis aligned scale require
    // a full 2x2 transform uniform; tracked as a follow up.
    batch.svg_draws.push(SvgDrawCall {
        geometry,
        translate: [translate_x, translate_y],
        scale: [scale_x * transform.a, scale_y * transform.d],
        clip_rect,
        color_tint: [1.0, 1.0, 1.0, 1.0],
        opacity,
    });
}

#[allow(clippy::too_many_arguments)]
fn emit_text_glyphs_cached(
    text: &str,
    x: f32,
    y: f32,
    max_width: Option<f32>,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    color: &Color,
    clip_rect: [f32; 4],
    batch: &mut FrameBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    mut glyph_keys_out: Option<&mut FxHashSet<GlyphKey>>,
) {
    let cache_key =
        ShapedTextCache::make_key(text, font_size, line_height, letter_spacing, max_width);
    let color_linear = color.to_linear_f32();

    // Check if we have a cached shaped result. If any atlas key is missing,
    // invalidate this shaped entry and rebuild so glyphs are never silently
    // dropped on atlas churn.
    if let Some(entry) = shaped_cache.map.get(&cache_key).cloned() {
        let atlas_ready = entry.glyphs.iter().all(|cg| atlas.cache.contains_key(&cg.atlas_key));
        if atlas_ready {
            for cg in &entry.glyphs {
                let atlas_entry = atlas
                    .cache
                    .get(&cg.atlas_key)
                    .copied()
                    .expect("atlas_ready guarantees all shaped glyph keys exist");
                atlas.touch(&cg.atlas_key);
                if let Some(keys) = glyph_keys_out.as_deref_mut() {
                    keys.insert(cg.atlas_key);
                }
                batch.glyph_instances.push(GlyphInstance {
                    pos: [x + cg.rel_x, y + cg.rel_y],
                    size: atlas_entry.size,
                    uv_min: [atlas_entry.uv_rect[0], atlas_entry.uv_rect[1]],
                    uv_max: [atlas_entry.uv_rect[2], atlas_entry.uv_rect[3]],
                    color: color_linear,
                    clip_rect,
                });
            }
            return;
        }
        shaped_cache.map.remove(&cache_key);
    }

    // Cache miss: shape text and populate cache
    let metrics = Metrics::new(font_size, font_size * line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width.map(|w| w.max(1.0)), None);
    buffer.set_text(font_system, text, Attrs::new(), Shaping::Advanced);
    buffer.shape_until_scroll(font_system, false);

    let mut cached_glyphs = Vec::new();

    for run in buffer.layout_runs() {
        let run_y = run.line_y;
        for (glyph_idx, glyph) in run.glyphs.iter().enumerate() {
            let ls_offset = glyph_idx as f32 * letter_spacing;
            let physical = glyph.physical((ls_offset, 0.0), 1.0);

            let key = GlyphKey {
                font_id: atlas_font_namespace(&physical.cache_key),
                glyph_id: physical.cache_key.glyph_id,
                font_size_tenths: (font_size * 10.0) as u16,
                subpixel_bin: ((physical.cache_key.x_bin as u8) << 2)
                    | (physical.cache_key.y_bin as u8),
            };

            let entry = if let Some(entry) = atlas.cache.get(&key).copied() {
                atlas.touch(&key);
                entry
            } else {
                let raster_result =
                    rasterize_swash_for_atlas(rasterizer.swash, font_system, &physical, atlas, key);
                match raster_result {
                    Some(entry) => entry,
                    None => continue,
                }
            };

            let rel_x = physical.x as f32 + entry.offset[0];
            let rel_y = run_y + physical.y as f32 + entry.offset[1];

            cached_glyphs.push(CachedGlyph { rel_x, rel_y, atlas_key: key });
            if let Some(keys) = glyph_keys_out.as_deref_mut() {
                keys.insert(key);
            }

            batch.glyph_instances.push(GlyphInstance {
                pos: [x + rel_x, y + rel_y],
                size: entry.size,
                uv_min: [entry.uv_rect[0], entry.uv_rect[1]],
                uv_max: [entry.uv_rect[2], entry.uv_rect[3]],
                color: color_linear,
                clip_rect,
            });
        }
    }

    shaped_cache.map.insert(cache_key, ShapedTextEntry { glyphs: cached_glyphs });
}

/// Hash a font family name into the stable `font_id` used by
/// [`ShapeCacheKey`]. Empty string falls back to 0 so anonymous / monospace
/// default families share a bucket.
pub fn shape_cache_font_id(font_family: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    if font_family.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    font_family.hash(&mut hasher);
    hasher.finish()
}

/// Local struct shared by `emit_grid_cells` and `emit_grid_row_fresh` so
/// both paths resolve glyphs into the same shape.
struct ResolvedGlyph {
    key: GlyphKey,
    entry: GlyphEntry,
    physical_x: i32,
    physical_y: i32,
    line_y: f32,
}

/// Emit background quads and glyph instances for a `CellGrid`.
///
/// This path skips cosmic-text shaping entirely for already rasterized
/// glyphs. When a retained per line quad cache is provided, rows whose
/// content and geometry signatures still match the cache are replayed by
/// appending the previously emitted vertex instances to the frame batch,
/// skipping shaping, rasterization, and iteration.
#[allow(clippy::too_many_arguments)]
fn emit_grid_cells(
    grid: &CellGrid,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    batch: &mut FrameBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shape_cache: &mut ShapeCache,
    mut glyph_keys_out: Option<&mut FxHashSet<GlyphKey>>,
    node_id: NodeId,
    mut line_cache: Option<&mut LineQuadCache>,
) {
    let rows = grid.rows();
    let cols = grid.cols();
    let cells = grid.cells();
    let trace_hash = terminal_grid_trace_hash(grid);
    let trace_this_grid = terminal_trace_enabled()
        && LAST_TERMINAL_RENDER_TRACE_HASH.swap(trace_hash, Ordering::Relaxed) != trace_hash;
    let trace_rows = if trace_this_grid { Some(grid.debug_rows(4, 96)) } else { None };
    let mut trace_glyphs: Vec<String> = Vec::new();

    let atlas_generation = atlas.generation;

    // Shape each unique character once, then cache the fully resolved glyph
    // per actual atlas key. Fractional cell origins can change the subpixel
    // bins, so caching only by `char` is incorrect when cell_w/cell_h are not
    // integers. This cache is shared across every row we actually emit
    // (cache miss path) in this pass.
    let mut glyph_cache: FxHashMap<GlyphKey, ResolvedGlyph> = FxHashMap::default();

    // Precompute the (font_family, font_id, style) fields for the ShapeCache
    // key. These are identical for every cell in this grid, so we lift them
    // out of the inner loop.
    #[cfg(target_os = "windows")]
    let family_name: &str = &rasterizer.dw.font_family;
    #[cfg(not(target_os = "windows"))]
    let family_name: &str = "";
    let shape_font_id = shape_cache_font_id(family_name);
    // Style hash reserved for bold/italic/etc; currently single bucket.
    const SHAPE_STYLE_REGULAR: u64 = 0;
    let shape_font_size_tenths = (font_size * 10.0).round() as u32;

    // Auto-invalidate the cache if font, DPI, or size has changed since the
    // last grid render. `retune` is a no-op when nothing has changed, so the
    // hot path still costs a single hashmap lookup per cell.
    shape_cache.retune(family_name, 1.0, font_size);

    // Reusable buffer for glyph shaping on cache miss.
    let metrics = cosmic_text::Metrics::new(font_size, cell_h);
    let mut buffer = cosmic_text::Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(cell_w * 4.0), None);
    let mut ch_buf = [0u8; 4];

    let dirty = grid.dirty_flags();

    // Geometry inputs are constant across every row of a single grid pass,
    // so build the geometry signature once and reuse it for every row probe.
    let geom_sig = LineGeometrySig::new(
        origin_x,
        origin_y,
        cell_w,
        cell_h,
        font_size,
        opacity,
        clip_rect,
        cols as u32,
        atlas_generation,
    );

    // Drop any cached rows that refer to rows past the current grid height.
    // This is cheap and keeps memory bounded when the grid shrinks.
    if let Some(cache) = line_cache.as_deref_mut() {
        cache.truncate_element(node_id, rows as u32);
    }

    for row in 0..rows {
        // Determine which columns in this row have damage. A fully clean row
        // (no cells touched since the last clear_dirty) takes the cache replay
        // fast path when available, and is skipped otherwise (matching the
        // Tier 2 line damage skip behavior).
        let damage_range = grid.line_damage_for(row).and_then(|ld| {
            if ld.is_clean() {
                None
            } else {
                Some((ld.first_dirty_col as usize, (ld.last_dirty_col as usize + 1).min(cols)))
            }
        });
        let row_is_clean = match grid.line_damage_for(row) {
            Some(ld) => ld.is_clean(),
            None => false,
        };

        let content_sig = hash_row_cells(cells, row, cols);

        // Cache probe: replay the cached instances for this row when the
        // content hash and geometry signature still match. Works for both
        // clean and dirty rows because the hash captures any visible change.
        if let Some(cache) = line_cache.as_deref_mut() {
            if let Some(hit) = cache.lookup_replayable(node_id, row as u32, content_sig, geom_sig) {
                batch.quad_instances.extend_from_slice(&hit.quads);
                batch.glyph_instances.extend_from_slice(&hit.glyphs);
                for key in &hit.glyph_keys {
                    atlas.touch(key);
                    if let Some(keys) = glyph_keys_out.as_deref_mut() {
                        keys.insert(*key);
                    }
                }
                continue;
            }
        }

        // Clean row with no cache hit: nothing to emit. This preserves the
        // Tier 2 damage-skip guarantee that unchanged rows incur zero
        // per-cell work when the line cache is unavailable.
        if row_is_clean {
            continue;
        }

        let (scan_start, scan_end) = damage_range.unwrap_or((0, cols));
        if scan_start >= scan_end {
            continue;
        }

        // Cache miss on a dirty row: emit fresh into local buffers using the
        // Tier 2 two pass structure (run batched background, then per cell
        // glyphs) and store the result in the line cache for next frame.
        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let mut row_glyphs: Vec<GlyphInstance> = Vec::new();
        let mut row_keys: Vec<GlyphKey> = Vec::new();

        emit_grid_row_fresh(
            grid,
            cells,
            dirty,
            row,
            cols,
            scan_start,
            scan_end,
            origin_x,
            origin_y,
            cell_w,
            cell_h,
            font_size,
            opacity,
            clip_rect,
            shape_font_id,
            SHAPE_STYLE_REGULAR,
            shape_font_size_tenths,
            family_name,
            &mut row_quads,
            &mut row_glyphs,
            &mut row_keys,
            &mut glyph_cache,
            shape_cache,
            atlas,
            font_system,
            rasterizer,
            &mut buffer,
            &mut ch_buf,
            trace_this_grid,
            &mut trace_glyphs,
        );

        // Forward to the frame batch.
        batch.quad_instances.extend_from_slice(&row_quads);
        batch.glyph_instances.extend_from_slice(&row_glyphs);
        if let Some(keys) = glyph_keys_out.as_deref_mut() {
            for key in &row_keys {
                keys.insert(*key);
            }
        }

        // Store the fresh row in the cache.
        if let Some(cache) = line_cache.as_deref_mut() {
            cache.store(
                node_id,
                row as u32,
                content_sig,
                geom_sig,
                row_quads,
                row_glyphs,
                row_keys,
            );
        }
    }

    if trace_this_grid {
        let rows_dump = trace_rows.unwrap_or_default();
        append_terminal_trace_line(&format!(
            "terminal-trace stage=emit_grid_cells rows={} cols={} origin=({:.1}, {:.1}) cell=({:.2}, {:.2}) cursor=({}, {}) visible={} row0={:?} row1={:?} row2={:?} row3={:?} glyphs={}",
            rows,
            cols,
            origin_x,
            origin_y,
            cell_w,
            cell_h,
            grid.cursor_row(),
            grid.cursor_col(),
            grid.cursor_visible(),
            rows_dump.first().cloned().unwrap_or_default(),
            rows_dump.get(1).cloned().unwrap_or_default(),
            rows_dump.get(2).cloned().unwrap_or_default(),
            rows_dump.get(3).cloned().unwrap_or_default(),
            trace_glyphs.join(" | "),
        ));
    }

    // Draw cursor block when visible. The cursor moves every frame in the
    // typical interactive case; we do not cache it, because the pertinent
    // row's content hash may still be stable while the cursor shifts inside
    // that row.
    if grid.cursor_visible() {
        let crow = grid.cursor_row();
        let ccol = grid.cursor_col();
        if crow < rows && ccol < cols {
            let cx = origin_x + ccol as f32 * cell_w;
            let cy = origin_y + crow as f32 * cell_h;

            let cursor_idx = crow * cols + ccol;
            let cell_fg = cells[cursor_idx].fg;
            let mut cursor_color = cell_fg.to_linear_f32();
            cursor_color[3] *= opacity * 0.7;

            batch.quad_instances.push(QuadInstance {
                pos: [cx, cy],
                size: [cell_w, cell_h],
                color: cursor_color,
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: [0.0; 4],
                clip_rect,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
            });
        }
    }
}

/// Emit one row of a cell grid into three output buffers, doing all
/// shaping and atlas uploads needed for that row. Called by
/// `emit_grid_cells` on cache miss. Uses Tier 2's two pass layout:
/// first one merged background `QuadInstance` per style run in the
/// damaged column range, then a per cell glyph pass over dirty columns
/// using the cross frame `ShapeCache`.
#[allow(clippy::too_many_arguments)]
fn emit_grid_row_fresh(
    grid: &CellGrid,
    cells: &[unshit_core::cell_grid::Cell],
    dirty: &[bool],
    row: usize,
    cols: usize,
    scan_start: usize,
    scan_end: usize,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    shape_font_id: u64,
    shape_style: u64,
    shape_font_size_tenths: u32,
    family_name: &str,
    row_quads: &mut Vec<QuadInstance>,
    row_glyphs: &mut Vec<GlyphInstance>,
    row_keys: &mut Vec<GlyphKey>,
    glyph_cache: &mut FxHashMap<GlyphKey, ResolvedGlyph>,
    shape_cache: &mut ShapeCache,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    buffer: &mut cosmic_text::Buffer,
    ch_buf: &mut [u8; 4],
    trace_this_grid: bool,
    trace_glyphs: &mut Vec<String>,
) {
    let py = origin_y + row as f32 * cell_h;
    let row_base = row * cols;

    // Merge adjacent cells with the same (fg, bg, attrs) into a single
    // background QuadInstance via `compute_style_runs_in_range`, mirroring
    // Zed's BatchedTextRun pattern: one quad per run instead of one per cell.
    for run in grid.compute_style_runs_in_range(row, scan_start, scan_end) {
        // A run is only emitted when at least one of its columns is dirty
        // this frame. Without this check a partial-row repaint would also
        // redraw the surrounding clean columns that happened to share the
        // same style.
        let run_has_damage = (run.start_col..run.end_col).any(|c| dirty[row_base + c]);
        if !run_has_damage {
            continue;
        }
        let bg = run.style.bg;
        if bg.a > 0 {
            let mut bg_color = bg.to_linear_f32();
            bg_color[3] *= opacity;
            let px = origin_x + run.start_col as f32 * cell_w;
            let width = run.col_count() as f32 * cell_w;
            if width > 0.0 {
                row_quads.push(QuadInstance {
                    pos: [px, py],
                    size: [width, cell_h],
                    color: bg_color,
                    border_color: [0.0; 4],
                    border_width: [0.0; 4],
                    border_radius: [0.0; 4],
                    clip_rect,
                    shadow_color: [0.0; 4],
                    shadow_offset: [0.0; 2],
                    shadow_params: [0.0; 2],
                    shadow_spread: [0.0; 2],
                    gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                    gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                    gradient_params: [0.0; 4],
                    gradient_extra: EMPTY_GRADIENT_EXTRA,
                });
            }
        }
    }

    // Emit per-cell glyphs across the same damaged range.
    for col in scan_start..scan_end {
        let idx = row_base + col;
        if !dirty[idx] {
            continue;
        }
        let cell = &cells[idx];
        // Skip glyph for empty cells and wide continuation cells.
        if cell.is_empty() || cell.wide_continuation {
            continue;
        }

        // INVERSE swaps fg/bg; only fg is needed here since the bg quad
        // was already emitted via the run loop above.
        let fg = if cell.attrs.contains(CellAttrs::INVERSE) { cell.bg } else { cell.fg };
        let mut fg_linear = fg.to_linear_f32();
        if cell.attrs.contains(CellAttrs::DIM) {
            fg_linear[3] *= 0.5;
        }
        fg_linear[3] *= opacity;

        let cache_key = ShapeCacheKey {
            ch: cell.ch,
            font_id: shape_font_id,
            style: shape_style,
            font_size_tenths: shape_font_size_tenths,
        };
        let px = origin_x + col as f32 * cell_w;
        // Cross-frame ShapeCache. Hit path bypasses set_text and
        // shape_until_scroll entirely; miss path shapes once and stores.
        let prototype = if let Some(entry) = shape_cache.get(&cache_key) {
            entry.clone()
        } else {
            let ch_str = cell.ch.encode_utf8(ch_buf);
            #[cfg(target_os = "windows")]
            let family = cosmic_text::Family::Name(family_name);
            #[cfg(not(target_os = "windows"))]
            let family = cosmic_text::Family::Monospace;
            // Silence unused on non-windows.
            let _ = family_name;
            buffer.set_text(
                font_system,
                ch_str,
                cosmic_text::Attrs::new().family(family),
                cosmic_text::Shaping::Advanced,
            );
            buffer.shape_until_scroll(font_system, false);

            let shaped = buffer.layout_runs().find_map(|run| {
                run.glyphs
                    .first()
                    .cloned()
                    .map(|glyph| ShapedGlyphEntry { layout_glyph: glyph, line_y: run.line_y })
            });
            shape_cache.insert(cache_key, shaped.clone());
            shaped
        };

        let Some(prototype) = prototype else {
            continue;
        };

        let px_floor = px.floor();
        let py_floor = py.floor();
        let physical = prototype.layout_glyph.physical((px - px_floor, py - py_floor), 1.0);
        let key = GlyphKey {
            font_id: atlas_font_namespace(&physical.cache_key),
            glyph_id: physical.cache_key.glyph_id,
            font_size_tenths: (font_size * 10.0) as u16,
            subpixel_bin: ((physical.cache_key.x_bin as u8) << 2)
                | (physical.cache_key.y_bin as u8),
        };

        let was_cached = glyph_cache.contains_key(&key);
        let resolved = if let Some(cached) = glyph_cache.get(&key) {
            atlas.touch(&cached.key);
            cached
        } else {
            let entry = if let Some(entry) = atlas.cache.get(&key).copied() {
                atlas.touch(&key);
                entry
            } else {
                match rasterize_grid_glyph_for_atlas(
                    rasterizer,
                    font_system,
                    &physical,
                    cell.ch,
                    font_size,
                    atlas,
                    key,
                ) {
                    Some(entry) => entry,
                    None => continue,
                }
            };

            glyph_cache.entry(key).or_insert(ResolvedGlyph {
                key,
                entry,
                physical_x: physical.x,
                physical_y: physical.y,
                line_y: prototype.line_y,
            })
        };

        row_keys.push(resolved.key);

        let gx = px_floor + resolved.physical_x as f32 + resolved.entry.offset[0];
        let gy = py_floor + resolved.line_y + resolved.physical_y as f32 + resolved.entry.offset[1];
        if trace_this_grid && row < 4 && trace_glyphs.len() < 64 {
            trace_glyphs.push(format!(
                "{} r{}c{} ch={:?} key=({}, {}, {}) pos=({:.1}, {:.1})",
                if was_cached { "cache" } else { "miss" },
                row,
                col,
                cell.ch,
                resolved.key.font_id,
                resolved.key.glyph_id,
                resolved.key.subpixel_bin,
                gx,
                gy,
            ));
        }

        row_glyphs.push(GlyphInstance {
            pos: [gx, gy],
            size: resolved.entry.size,
            uv_min: [resolved.entry.uv_rect[0], resolved.entry.uv_rect[1]],
            uv_max: [resolved.entry.uv_rect[2], resolved.entry.uv_rect[3]],
            color: fg_linear,
            clip_rect,
        });
    }
}

/// Walk the arena and emit select-specific rendering:
/// - Closed select: label text and dropdown arrow rendered into the content layer.
/// - Open select: dropdown items rendered into the overlay layer.
///
/// This is called from app.rs after `build_render_batch` so that select overlays
/// always appear on top of regular content.
#[allow(clippy::too_many_arguments)]
pub fn emit_select_overlays(
    arena: &NodeArena,
    root: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    vw: f32,
    vh: f32,
) {
    emit_select_overlays_rec(
        arena,
        root,
        batch,
        atlas,
        font_system,
        rasterizer,
        shaped_cache,
        vw,
        vh,
    );
}

fn emit_select_overlays_rec(
    arena: &NodeArena,
    node_id: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    vw: f32,
    vh: f32,
) {
    let Some(element) = arena.get(node_id) else { return };

    if element.tag == Tag::Select {
        emit_select_node(element, batch, atlas, font_system, rasterizer, shaped_cache, vw, vh);
        // Select has no arena children to recurse into.
        return;
    }

    // Recurse into children
    let mut child = element.first_child;
    // Store child IDs to avoid borrow conflict while iterating
    let mut children = Vec::new();
    while !child.is_dangling() {
        children.push(child);
        child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
    }
    for child_id in children {
        emit_select_overlays_rec(
            arena,
            child_id,
            batch,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            vw,
            vh,
        );
    }
}

/// Emit rendering for a single select element and, if open, its dropdown.
#[allow(clippy::too_many_arguments)]
fn emit_select_node(
    element: &unshit_core::element::Element,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    vw: f32,
    vh: f32,
) {
    let Some(ref ss) = element.select_state else { return };
    let rect = element.layout_rect;
    let style = &element.computed_style;

    // Derive colors from the element's computed style or fall back to defaults.
    let fg_color = style.color;
    let font_size = style.font_size.max(10.0);
    let line_height = style.line_height;
    let letter_spacing = style.letter_spacing;
    let pad_left = style.padding.left;
    let pad_top = style.padding.top;

    // --- Selected label text inside the select box ---
    if !ss.options.is_empty() {
        let sel_idx = (ss.selected_index as usize).min(ss.options.len().saturating_sub(1));
        let label = &ss.options[sel_idx].label;
        let text_x = rect.x + pad_left;
        let text_y = rect.y + pad_top;
        let text_w = (rect.width - pad_left - style.padding.right - 20.0).max(1.0);
        let clip = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
        let content_layer = batch.layer_mut(Layer::Content);
        emit_text_glyphs_cached(
            label,
            text_x,
            text_y,
            Some(text_w),
            font_size,
            line_height,
            letter_spacing,
            &fg_color,
            clip,
            content_layer,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            None,
        );
    }

    // --- Dropdown arrow (▼) at right edge ---
    {
        let arrow_x = rect.x + rect.width - 18.0;
        let arrow_y = rect.y + pad_top;
        let clip = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
        let content_layer = batch.layer_mut(Layer::Content);
        emit_text_glyphs_cached(
            "\u{25BC}",
            arrow_x,
            arrow_y,
            Some(18.0),
            font_size,
            line_height,
            letter_spacing,
            &fg_color,
            clip,
            content_layer,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            None,
        );
    }

    // --- Dropdown panel when open ---
    if !ss.open || ss.options.is_empty() {
        return;
    }

    let item_h = (font_size * line_height * 1.2).max(24.0);
    let dropdown_w = rect.width;
    let dropdown_x = rect.x;
    let dropdown_y = rect.y + rect.height;
    let dropdown_h = item_h * ss.options.len() as f32;

    // Clamp dropdown to viewport bottom
    let actual_y =
        if dropdown_y + dropdown_h > vh { (rect.y - dropdown_h).max(0.0) } else { dropdown_y };

    let overlay_clip = [0.0, 0.0, vw, vh];

    // Dropdown background panel
    let overlay_layer = batch.layer_mut(Layer::Overlay);
    overlay_layer.quad_instances.push(QuadInstance {
        pos: [dropdown_x, actual_y],
        size: [dropdown_w, dropdown_h],
        color: [0.15, 0.15, 0.18, 0.97],
        border_color: [0.35, 0.35, 0.40, 1.0],
        border_width: [1.0, 1.0, 1.0, 1.0],
        border_radius: [4.0, 4.0, 4.0, 4.0],
        clip_rect: overlay_clip,
        shadow_color: [0.0, 0.0, 0.0, 0.4],
        shadow_offset: [0.0, 4.0],
        shadow_params: [8.0, 0.0],
        shadow_spread: [0.0, 0.0],
        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
        gradient_params: [0.0, 0.0, 0.0, 0.0],
        gradient_extra: EMPTY_GRADIENT_EXTRA,
    });

    // Per-option rows
    for (i, opt) in ss.options.iter().enumerate() {
        let item_y = actual_y + i as f32 * item_h;
        let is_highlighted = ss.highlighted_index == Some(i as u32);
        let is_selected = ss.selected_index == i as u32;
        let item_clip = [dropdown_x, actual_y, dropdown_x + dropdown_w, actual_y + dropdown_h];

        if is_highlighted {
            let overlay_layer = batch.layer_mut(Layer::Overlay);
            overlay_layer.quad_instances.push(QuadInstance {
                pos: [dropdown_x + 2.0, item_y + 1.0],
                size: [dropdown_w - 4.0, item_h - 2.0],
                color: [0.27, 0.49, 0.82, 0.85],
                border_color: [0.0, 0.0, 0.0, 0.0],
                border_width: [0.0, 0.0, 0.0, 0.0],
                border_radius: [3.0, 3.0, 3.0, 3.0],
                clip_rect: item_clip,
                shadow_color: [0.0, 0.0, 0.0, 0.0],
                shadow_offset: [0.0, 0.0],
                shadow_params: [0.0, 0.0],
                shadow_spread: [0.0, 0.0],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0, 0.0, 0.0, 0.0],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
            });
        }

        // Checkmark for the currently selected option
        let text_x_offset = if is_selected { 20.0 } else { 8.0 };
        let text_color = Color { r: 230, g: 230, b: 235, a: 255 };

        if is_selected {
            let check_color = Color { r: 100, g: 200, b: 120, a: 255 };
            let overlay_layer = batch.layer_mut(Layer::Overlay);
            emit_text_glyphs_cached(
                "\u{2713}",
                dropdown_x + 4.0,
                item_y + 4.0,
                Some(14.0),
                font_size.min(14.0),
                line_height,
                letter_spacing,
                &check_color,
                item_clip,
                overlay_layer,
                atlas,
                font_system,
                rasterizer,
                shaped_cache,
                None,
            );
        }

        let overlay_layer = batch.layer_mut(Layer::Overlay);
        emit_text_glyphs_cached(
            &opt.label,
            dropdown_x + text_x_offset,
            item_y + 4.0,
            Some(dropdown_w - text_x_offset - 8.0),
            font_size,
            line_height,
            letter_spacing,
            &text_color,
            item_clip,
            overlay_layer,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            None,
        );
    }
}

/// Cache key for [`CellMetricsCache`]. Keyed on the inputs that can change the
/// measured monospace cell dimensions: font identity (family name hash), font
/// size in tenths of a pixel, line height in tenths of a pixel, and DPI scale
/// factor in thousandths.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CellMetricsKey {
    pub font_id: u64,
    pub font_size_tenths: u32,
    pub line_height_tenths: u32,
    pub scale_factor_thousandths: u32,
}

impl CellMetricsKey {
    pub fn new(font_family: &str, font_size: f32, line_height: f32, scale_factor: f32) -> Self {
        Self {
            font_id: shape_cache_font_id(font_family),
            font_size_tenths: (font_size * 10.0).round() as u32,
            line_height_tenths: (line_height * 10.0).round() as u32,
            scale_factor_thousandths: (scale_factor * 1000.0).round() as u32,
        }
    }
}

/// Measured cell metrics for a given font configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
}

/// Cross-frame cache for monospace cell metrics. Keyed on font identity,
/// font size, line height, and DPI scale factor. Entries are only produced on
/// a cache miss; hits are O(1) hash lookups. Invalidation is automatic via
/// the key: changing any input produces a fresh lookup.
///
/// A single shared instance lives on the frame loop and is reused across
/// frames. The hot render path consults it on every `ElementContent::Grid`
/// node but only measures once per unique configuration.
pub struct CellMetricsCache {
    entries: FxHashMap<CellMetricsKey, CellMetrics>,
    /// Number of cache misses (fresh measurements) since creation. Exposed
    /// so tests can prove the cache actually caches.
    misses: u64,
}

impl Default for CellMetricsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CellMetricsCache {
    pub fn new() -> Self {
        Self { entries: FxHashMap::default(), misses: 0 }
    }

    /// Returns the measured `CellMetrics` for the given configuration,
    /// reusing any cached value when the key matches.
    pub fn get_or_measure(
        &mut self,
        font_system: &mut FontSystem,
        font_family: &str,
        font_size: f32,
        line_height: f32,
        scale_factor: f32,
    ) -> CellMetrics {
        let key = CellMetricsKey::new(font_family, font_size, line_height, scale_factor);
        if let Some(&hit) = self.entries.get(&key) {
            return hit;
        }
        self.misses += 1;
        let cell_w = measure_monospace_advance(font_system, font_family, font_size, line_height);
        let cell_h = line_height;
        let metrics = CellMetrics { cell_w, cell_h };
        self.entries.insert(key, metrics);
        metrics
    }

    /// Number of cache misses since creation. Exists for tests and diagnostics.
    pub fn miss_count(&self) -> u64 {
        self.misses
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry has ever been measured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears every cached entry. Intended for use when the font stack is
    /// swapped at runtime or the atlas generation changes.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Measure the actual advance width of a monospace glyph at the given font_size.
///
/// `line_height` is the absolute pixel line height (typically `font_size * style.line_height`
/// from CSS resolution). Accepting it as a parameter keeps the renderer's cell
/// placement code as the single source of truth for the line_height value, rather
/// than hardcoding 1.2 inside this function.
///
/// Cached: only re-measures when font_size, line_height, font family, or DPI
/// scale change. Backed by an internal [`CellMetricsCache`] so the measurement
/// survives across frames.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn measure_monospace_cell_width(
    font_system: &mut FontSystem,
    font_size: f32,
    line_height: f32,
) -> f32 {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<CellMetricsCache>> = Mutex::new(None);

    let mut guard = CACHE.lock().expect("cell metrics cache mutex poisoned");
    let cache = guard.get_or_insert_with(CellMetricsCache::new);
    cache.get_or_measure(font_system, monospace_family_name(), font_size, line_height, 1.0).cell_w
}

/// Default family name used for the free-function measurement path. Tests and
/// `measure_monospace_cell_width` call this; the renderer's grid emission path
/// now routes through [`CellMetricsCache::get_or_measure`] directly with the
/// active family.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn monospace_family_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Consolas"
    }
    #[cfg(not(target_os = "windows"))]
    {
        ""
    }
}

/// Perform the actual cosmic-text measurement for a monospace glyph. Kept
/// separate from the cache so the cache owns the memoization policy and this
/// function remains a pure measurement op.
fn measure_monospace_advance(
    font_system: &mut FontSystem,
    font_family: &str,
    font_size: f32,
    line_height: f32,
) -> f32 {
    let family = if font_family.is_empty() {
        cosmic_text::Family::Monospace
    } else {
        cosmic_text::Family::Name(font_family)
    };

    let metrics = cosmic_text::Metrics::new(font_size, line_height);
    let mut buffer = cosmic_text::Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(font_size * 10.0), None);
    buffer.set_text(
        font_system,
        "M",
        cosmic_text::Attrs::new().family(family),
        cosmic_text::Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    if let Some(glyph) = buffer.layout_runs().flat_map(|run| run.glyphs.iter()).next() {
        return glyph.w;
    }
    font_size * 0.6
}

/// Rasterize a glyph via SwashCache and insert into the atlas.
/// Used for CSS/UI text where cosmic-text metrics must match the rasterizer.
fn rasterize_swash_for_atlas(
    swash_cache: &mut SwashCache,
    font_system: &mut FontSystem,
    physical: &cosmic_text::PhysicalGlyph,
    atlas: &mut GlyphAtlas,
    key: GlyphKey,
) -> Option<crate::atlas::GlyphEntry> {
    let image = swash_cache.get_image_uncached(font_system, physical.cache_key)?;
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }

    let w = image.placement.width;
    let h = image.placement.height;
    let bearing_x = image.placement.left as f32;
    let bearing_y = -(image.placement.top as f32);

    let alpha_data = match image.content {
        cosmic_text::SwashContent::Mask => image.data,
        cosmic_text::SwashContent::Color => {
            image.data.chunks(4).map(|c| c.get(3).copied().unwrap_or(255)).collect()
        }
        cosmic_text::SwashContent::SubpixelMask => image
            .data
            .chunks(3)
            .map(|c| ((c[0] as u16 + c[1] as u16 + c[2] as u16) / 3) as u8)
            .collect(),
    };

    // Match the upload shape to the current atlas format. The Windows path can
    // now run either a monochrome R8 atlas or the old RGBA subpixel atlas.
    let glyph_data = if atlas.bytes_per_pixel == 4 {
        alpha_data.iter().flat_map(|&a| [a, a, a, a]).collect()
    } else {
        alpha_data
    };

    let entry = atlas.get_or_insert(key, w, h, glyph_data, [bearing_x, bearing_y])?;
    atlas.touch(&key);
    Some(entry)
}

/// Rasterize a terminal grid glyph and insert into the atlas.
/// On Windows, uses DirectWrite for native ClearType quality.
/// On non-Windows, falls back to SwashCache.
fn rasterize_grid_glyph_for_atlas(
    rasterizer: &mut Rasterizer<'_>,
    font_system: &mut FontSystem,
    physical: &cosmic_text::PhysicalGlyph,
    ch: char,
    font_size: f32,
    atlas: &mut GlyphAtlas,
    key: GlyphKey,
) -> Option<crate::atlas::GlyphEntry> {
    #[cfg(target_os = "windows")]
    {
        if use_directwrite_grid_rasterization() {
            let _ = (font_system, physical); // not needed on DirectWrite path
            let rg = rasterizer.dw.rasterize_glyph(ch, font_size)?;
            if rg.width == 0 || rg.height == 0 {
                return None;
            }
            let entry = atlas.get_or_insert(
                key,
                rg.width,
                rg.height,
                rg.data,
                [rg.bearing_x, rg.bearing_y],
            )?;
            atlas.touch(&key);
            Some(entry)
        } else {
            // The trace shows terminal content stays correct through batching,
            // so prefer the swash path until the Windows-specific raster data
            // corruption is understood. TM_FORCE_DIRECTWRITE_GRID=1 restores
            // the old path for A/B verification.
            rasterize_swash_for_atlas(rasterizer.swash, font_system, physical, atlas, key)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (ch, font_size); // not needed on swash path
        rasterize_swash_for_atlas(rasterizer.swash, font_system, physical, atlas, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unshit_core::element::{Element, Tag};
    use unshit_core::id::NodeId;
    use unshit_core::scroll::ScrollbarVisualState;
    use unshit_core::tree::NodeArena;

    /// Helper: build a minimal arena with a single div node (no taffy needed).
    fn build_single_node() -> (NodeArena, NodeId) {
        let mut arena = NodeArena::new();
        let elem = Element::new(Tag::Div);
        let root = arena.alloc(elem);
        (arena, root)
    }

    /// Helper: run `build_render_batch` using only CPU-side structures (no GPU
    /// atlas, no SVG cache that requires rasterization).  We pass a fake
    /// `ShapedTextCache` and rely on the fact that an unstyled div node emits
    /// no glyphs and no quads.
    fn run_batch_cpu_only(
        _arena: &NodeArena,
        root: NodeId,
        batch_cache: &mut BatchCache,
    ) -> LayeredBatch {
        use cosmic_text::{FontSystem, SwashCache};
        use unshit_core::layout::TextMeasureCache;

        let mut batch = LayeredBatch::new();
        // Build a minimal fake atlas: we cannot call GlyphAtlas::new without a
        // wgpu device, so we bypass it by using Default which zeroes the cache.
        // No glyphs are emitted for unstyled div nodes so this is safe.
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        #[cfg(target_os = "windows")]
        let _dw = crate::dw_rasterizer::DwRasterizer::new("Consolas");
        let mut _rasterizer = Rasterizer {
            swash: &mut swash_cache,
            #[cfg(target_os = "windows")]
            dw: &_dw,
        };
        let mut measure_cache = TextMeasureCache::default();
        let mut shaped_cache = ShapedTextCache::new();
        let mut svg_cache = crate::svg_cache::SvgTessCache::with_capacity(0);
        let scrollbar = ScrollbarVisualState::default();

        // We need a GlyphAtlas but cannot construct one without a device.
        // Instead skip build_render_batch entirely and exercise just the
        // cache logic, which is the part we want to test here.
        // For the cache test we only check that begin_frame/commit_frame work
        // and that dirty nodes produce output while clean ones replay from cache.
        let _ = (
            &mut batch,
            &mut font_system,
            &mut _rasterizer,
            &mut measure_cache,
            &mut shaped_cache,
            &mut svg_cache,
            &scrollbar,
        );

        // Simulate "frame 1": record something for the root node.
        batch_cache.begin_frame();
        // Dirty node produces output; we fake it by recording an empty range.
        batch_cache.record(root, 0, vec![], vec![], vec![], vec![], vec![], 0);
        batch_cache.commit_frame();

        batch
    }

    #[test]
    fn clean_node_skips_rebuild_on_second_frame() {
        let (mut arena, root) = build_single_node();

        // Clear PAINT flags that may have been set by build_tree_from_def.
        clear_paint_flags_subtree(&mut arena, root);

        // After clearing paint flags, the node has neither PAINT nor
        // SUBTREE_PAINT, so the batch builder should skip it and replay from
        // cache.  We verify this by checking that the cache lookup works.
        let mut cache = BatchCache::new();
        let _batch = run_batch_cpu_only(&arena, root, &mut cache);

        // The cache should now have an entry for root on layer 0.
        assert!(
            cache.get(root, 0).is_some(),
            "cache should contain an entry for root after simulated frame 1"
        );

        // Second frame: node is still clean.  The cache entry should still be
        // present (we do not evict on begin_frame; only on clear).
        cache.begin_frame();
        assert!(
            cache.get(root, 0).is_some(),
            "cache entry should remain readable after begin_frame (reads from prev)"
        );
    }

    #[test]
    fn batch_cache_begin_commit_cycle_works() {
        let mut cache = BatchCache::new();

        // Populate staging with a fake record.
        cache.begin_frame();
        cache.record(NodeId::DANGLING, 0, vec![], vec![], vec![], vec![], vec![], 0);
        // Before commit, `get` reads from the previous frame (empty).
        assert!(cache.get(NodeId::DANGLING, 0).is_none());

        // After commit, the staged entry becomes readable.
        cache.commit_frame();
        assert!(cache.get(NodeId::DANGLING, 0).is_some());

        // After begin_frame, a new staging cycle starts; the committed data is
        // still readable from `get` (it references `ranges`, not `pending`).
        cache.begin_frame();
        assert!(cache.get(NodeId::DANGLING, 0).is_some());
    }

    #[test]
    fn replayed_entries_survive_multi_frame_cycle() {
        // Regression test for a structural bug in BatchCache that caused
        // clean nodes to force-re-render every other frame: the walk's
        // replay path read from `ranges` but never wrote into `pending`, so
        // the next `commit_frame` swap dropped the entry. This produced an
        // alternating cache-hit/cache-miss pattern that, combined with
        // unflagged tick mutations to `computed_style`, manifested as
        // flickering on hover restyle and CSS animations (issues #41, #42).
        //
        // The fix routes the walk through `BatchCache::replay`, which copies
        // the cached range from `ranges` into `pending` so it survives the
        // next swap.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;

        // Frame 1: a dirty node produces primitives. record -> commit.
        cache.begin_frame();
        cache.record(id, 0, vec![], vec![], vec![], vec![], vec![], 0);
        cache.commit_frame();
        assert!(cache.get(id, 0).is_some(), "frame 1 recorded and committed");

        // Frame 2: the node is clean, so the walker calls `replay` instead
        // of re-rendering. The fix guarantees the cached entry is carried
        // forward into the current staging map.
        cache.begin_frame();
        assert!(
            cache.replay(id, 0, 0).is_some(),
            "frame 2 must replay from frame 1's committed data",
        );
        cache.commit_frame();

        // Frame 3: without the fix, the frame-2 commit swap would have
        // dropped the entry because nothing wrote to `pending`. With the
        // fix, `replay` cloned the range into `pending` during frame 2,
        // so the entry survives the swap.
        cache.begin_frame();
        assert!(cache.get(id, 0).is_some(), "replayed entry must persist across commit_frame",);

        // Frame 4: repeat to confirm the carry-forward is stable across
        // multiple consecutive replay cycles, not just the first one.
        assert!(cache.replay(id, 0, 0).is_some(), "frame 4 replay");
        cache.commit_frame();
        cache.begin_frame();
        assert!(
            cache.get(id, 0).is_some(),
            "entry must still persist after two consecutive replay frames",
        );
    }

    #[test]
    fn replay_returns_none_when_no_prior_entry() {
        // `replay` must not fabricate an entry for an unknown key; it is
        // purely a carry-forward for entries that already exist in `ranges`.
        let mut cache = BatchCache::new();
        cache.begin_frame();
        assert!(cache.replay(NodeId::DANGLING, 0, 0).is_none());
    }

    #[test]
    fn replay_returns_none_when_atlas_generation_mismatches() {
        // Atlas-aware invalidation: cached ranges built against an older atlas
        // generation must not be replayed.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;
        cache.begin_frame();
        cache.record(id, 0, vec![], vec![], vec![], vec![], vec![], 7);
        cache.commit_frame();

        cache.begin_frame();
        assert!(cache.replay(id, 0, 8).is_none(), "generation mismatch must force fresh render",);
    }

    #[test]
    fn replay_returns_recorded_data_when_pending_already_has_entry() {
        // If the caller has already recorded a fresh render for this node
        // earlier in the frame (e.g. a parent was dirty and walked children
        // including this node), a subsequent replay call must return the
        // pending entry instead of overwriting it with stale ranges data.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;

        // Seed ranges with an empty entry (stale previous-frame state).
        cache.begin_frame();
        cache.record(id, 0, vec![], vec![], vec![], vec![], vec![], 0);
        cache.commit_frame();

        // New frame: caller records fresh data with one distinctive span.
        cache.begin_frame();
        cache.record(
            id,
            0,
            vec![],
            vec![],
            vec![],
            vec![DrawSpan { kind: DrawKind::Quad, start: 0, count: 7 }],
            vec![],
            0,
        );

        // A subsequent replay call should return the just-recorded entry,
        // not the older empty entry still sitting in `ranges`.
        let out = cache.replay(id, 0, 0).expect("pending entry must be returned");
        assert_eq!(out.draw_spans.len(), 1);
        assert_eq!(out.draw_spans[0].kind, DrawKind::Quad);
        assert_eq!(out.draw_spans[0].count, 7);
    }

    // -----------------------------------------------------------------------
    // DrawSpan recording tests (text occlusion fix)
    // -----------------------------------------------------------------------

    #[test]
    fn draw_spans_are_recorded_in_batch_cache() {
        let spans = vec![
            DrawSpan { kind: DrawKind::Quad, start: 0, count: 3 },
            DrawSpan { kind: DrawKind::Glyph, start: 0, count: 5 },
            DrawSpan { kind: DrawKind::Quad, start: 3, count: 2 },
        ];
        let mut cache = BatchCache::new();
        cache.begin_frame();
        cache.record(NodeId::DANGLING, 0, vec![], vec![], vec![], spans.clone(), vec![], 0);
        cache.commit_frame();

        let cached = cache.get(NodeId::DANGLING, 0).expect("should have cached entry");
        assert_eq!(cached.draw_spans.len(), 3);
        assert_eq!(cached.draw_spans[0].kind, DrawKind::Quad);
        assert_eq!(cached.draw_spans[0].start, 0);
        assert_eq!(cached.draw_spans[0].count, 3);
        assert_eq!(cached.draw_spans[1].kind, DrawKind::Glyph);
        assert_eq!(cached.draw_spans[1].count, 5);
        assert_eq!(cached.draw_spans[2].kind, DrawKind::Quad);
        assert_eq!(cached.draw_spans[2].start, 3);
        assert_eq!(cached.draw_spans[2].count, 2);
    }

    #[test]
    fn cached_draw_spans_must_be_node_local_before_replay() {
        let quad_start = 11_u32;
        let glyph_start = 37_u32;
        let absolute_spans = vec![
            DrawSpan { kind: DrawKind::Quad, start: quad_start, count: 2 },
            DrawSpan { kind: DrawKind::Glyph, start: glyph_start, count: 4 },
            DrawSpan { kind: DrawKind::Quad, start: quad_start + 2, count: 1 },
            DrawSpan { kind: DrawKind::Glyph, start: glyph_start + 4, count: 3 },
        ];

        let normalized = absolute_spans
            .iter()
            .map(|span| DrawSpan {
                kind: span.kind,
                start: match span.kind {
                    DrawKind::Quad => span.start - quad_start,
                    DrawKind::Glyph => span.start - glyph_start,
                },
                count: span.count,
            })
            .collect::<Vec<_>>();

        assert_eq!(normalized[0].start, 0);
        assert_eq!(normalized[1].start, 0);
        assert_eq!(normalized[2].start, 2);
        assert_eq!(normalized[3].start, 4);

        let quad_offset = 100_u32;
        let glyph_offset = 200_u32;
        let replayed = normalized
            .iter()
            .map(|span| DrawSpan {
                kind: span.kind,
                start: match span.kind {
                    DrawKind::Quad => quad_offset + span.start,
                    DrawKind::Glyph => glyph_offset + span.start,
                },
                count: span.count,
            })
            .collect::<Vec<_>>();

        assert_eq!(replayed[0].start, 100);
        assert_eq!(replayed[1].start, 200);
        assert_eq!(replayed[2].start, 102);
        assert_eq!(replayed[3].start, 204);
    }

    #[test]
    fn draw_spans_replayed_with_offset_adjustment() {
        // Record spans at known positions.
        let spans = vec![
            DrawSpan { kind: DrawKind::Quad, start: 0, count: 2 },
            DrawSpan { kind: DrawKind::Glyph, start: 0, count: 3 },
        ];
        let mut cache = BatchCache::new();
        cache.begin_frame();
        cache.record(NodeId::DANGLING, 0, vec![], vec![], vec![], spans, vec![], 0);
        cache.commit_frame();

        // Replay into a batch that already has some data, simulating offset.
        let mut batch = FrameBatch::new();
        // Simulate pre-existing data by pushing dummy instances.
        let dummy_quad = QuadInstance {
            pos: [0.0; 2],
            size: [10.0; 2],
            color: [1.0; 4],
            border_color: [0.0; 4],
            border_width: [0.0; 4],
            border_radius: [0.0; 4],
            clip_rect: [0.0, 0.0, 100.0, 100.0],
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
            gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
            gradient_params: [0.0; 4],
            gradient_extra: EMPTY_GRADIENT_EXTRA,
        };
        batch.quad_instances.push(dummy_quad);

        // Replay cached data with offset.
        let cached = cache.get(NodeId::DANGLING, 0).unwrap();
        let quad_offset = batch.quad_instances.len() as u32;
        let glyph_offset = batch.glyph_instances.len() as u32;
        batch.quad_instances.extend_from_slice(&cached.quads);
        batch.glyph_instances.extend_from_slice(&cached.glyphs);
        for span in &cached.draw_spans {
            let offset = match span.kind {
                DrawKind::Quad => quad_offset,
                DrawKind::Glyph => glyph_offset,
            };
            batch.draw_spans.push(DrawSpan {
                kind: span.kind,
                start: span.start + offset,
                count: span.count,
            });
        }

        // The quad span should be offset by 1 (one pre-existing quad).
        assert_eq!(batch.draw_spans.len(), 2);
        assert_eq!(batch.draw_spans[0].kind, DrawKind::Quad);
        assert_eq!(batch.draw_spans[0].start, 1); // offset by 1
        assert_eq!(batch.draw_spans[1].kind, DrawKind::Glyph);
        assert_eq!(batch.draw_spans[1].start, 0); // glyph offset was 0
    }

    #[test]
    fn frame_batch_clear_resets_draw_spans() {
        let mut batch = FrameBatch::new();
        batch.draw_spans.push(DrawSpan { kind: DrawKind::Quad, start: 0, count: 1 });
        assert_eq!(batch.draw_spans.len(), 1);
        batch.clear();
        assert!(batch.draw_spans.is_empty());
    }

    // -----------------------------------------------------------------------
    // measure_monospace_cell_width tests (issue #5, approach 4)
    // -----------------------------------------------------------------------

    /// The function must return a positive width for a standard monospace font.
    #[test]
    fn measure_monospace_cell_width_returns_positive() {
        let mut fs = FontSystem::new();
        let font_size = 14.0_f32;
        let line_height = font_size * 1.2;
        let w = measure_monospace_cell_width(&mut fs, font_size, line_height);
        assert!(w > 0.0, "cell width must be positive, got {}", w);
    }

    /// Different line_height values must not change the measured advance width,
    /// because cosmic-text Metrics line_height only affects vertical layout.
    #[test]
    fn measure_monospace_cell_width_stable_across_line_heights() {
        let mut fs = FontSystem::new();
        let font_size = 14.0_f32;

        let w_normal = measure_monospace_cell_width(&mut fs, font_size, font_size * 1.2);
        let w_tall = measure_monospace_cell_width(&mut fs, font_size, font_size * 2.0);
        let w_tight = measure_monospace_cell_width(&mut fs, font_size, font_size * 1.0);

        let epsilon = 0.01;
        assert!(
            (w_normal - w_tall).abs() < epsilon,
            "line_height should not affect advance width: normal={}, tall={}",
            w_normal,
            w_tall
        );
        assert!(
            (w_normal - w_tight).abs() < epsilon,
            "line_height should not affect advance width: normal={}, tight={}",
            w_normal,
            w_tight
        );
    }

    /// The cache must return the same value on repeated calls with the same inputs.
    #[test]
    fn measure_monospace_cell_width_cache_consistency() {
        let mut fs = FontSystem::new();
        let font_size = 12.0_f32;
        let line_height = font_size * 1.2;

        let w1 = measure_monospace_cell_width(&mut fs, font_size, line_height);
        let w2 = measure_monospace_cell_width(&mut fs, font_size, line_height);
        assert_eq!(
            w1.to_bits(),
            w2.to_bits(),
            "cached value should be bit-identical: {} vs {}",
            w1,
            w2
        );
    }

    // -----------------------------------------------------------------------
    // Per-frame glyph cache tests (emit_grid_cells optimization)
    // -----------------------------------------------------------------------

    /// Helper: shape a single character through cosmic-text and return the
    /// resulting GlyphKey (the same computation emit_grid_cells performs).
    fn shape_char_to_key(
        fs: &mut FontSystem,
        ch: char,
        font_size: f32,
        cell_h: f32,
        cell_w: f32,
    ) -> Option<GlyphKey> {
        let metrics = cosmic_text::Metrics::new(font_size, cell_h);
        let mut buffer = cosmic_text::Buffer::new(fs, metrics);
        buffer.set_size(fs, Some(cell_w * 4.0), None);

        let mut ch_buf = [0u8; 4];
        let ch_str = ch.encode_utf8(&mut ch_buf);
        #[cfg(target_os = "windows")]
        let family = cosmic_text::Family::Name("Consolas");
        #[cfg(not(target_os = "windows"))]
        let family = cosmic_text::Family::Monospace;
        buffer.set_text(
            fs,
            ch_str,
            cosmic_text::Attrs::new().family(family),
            cosmic_text::Shaping::Advanced,
        );
        buffer.shape_until_scroll(fs, false);

        for run in buffer.layout_runs() {
            if let Some(glyph) = run.glyphs.iter().next() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                return Some(GlyphKey {
                    font_id: atlas_font_namespace(&physical.cache_key),
                    glyph_id: physical.cache_key.glyph_id,
                    font_size_tenths: (font_size * 10.0) as u16,
                    subpixel_bin: ((physical.cache_key.x_bin as u8) << 2)
                        | (physical.cache_key.y_bin as u8),
                });
            }
        }
        None
    }

    /// Core cache invariant: shaping the same character twice must yield an
    /// identical GlyphKey, otherwise the per-frame cache would serve wrong
    /// glyphs.
    #[test]
    fn same_char_produces_identical_glyph_key() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let key1 = shape_char_to_key(&mut fs, 'A', font_size, cell_h, cell_w)
            .expect("'A' should produce a glyph");
        let key2 = shape_char_to_key(&mut fs, 'A', font_size, cell_h, cell_w)
            .expect("'A' should produce a glyph on second call");
        assert_eq!(key1, key2, "same char must yield identical GlyphKey");
    }

    #[test]
    fn atlas_font_namespace_includes_render_flags() {
        let (plain, _, _) = cosmic_text::CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::empty(),
        );
        let (italic, _, _) = cosmic_text::CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::FAKE_ITALIC,
        );

        assert_ne!(
            atlas_font_namespace(&plain),
            atlas_font_namespace(&italic),
            "atlas font namespace must differ when glyph render flags differ",
        );
    }

    #[test]
    fn atlas_font_namespace_includes_font_identity() {
        let fs = FontSystem::new();
        let ids: Vec<_> = fs.db().faces().take(2).map(|face| face.id).collect();
        if ids.len() < 2 {
            return;
        }

        let (a, _, _) = cosmic_text::CacheKey::new(
            ids[0],
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::empty(),
        );
        let (b, _, _) = cosmic_text::CacheKey::new(
            ids[1],
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::empty(),
        );

        assert_ne!(
            atlas_font_namespace(&a),
            atlas_font_namespace(&b),
            "atlas font namespace must differ for different font ids",
        );
    }

    /// Different characters must produce different glyph IDs so the cache
    /// does not conflate them.
    #[test]
    fn different_chars_produce_different_glyph_keys() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let key_a = shape_char_to_key(&mut fs, 'A', font_size, cell_h, cell_w)
            .expect("'A' should produce a glyph");
        let key_b = shape_char_to_key(&mut fs, 'B', font_size, cell_h, cell_w)
            .expect("'B' should produce a glyph");
        assert_ne!(key_a.glyph_id, key_b.glyph_id, "'A' and 'B' must map to different glyph IDs");
    }

    /// The glyph cache is keyed on `char`. Verify that a broad set of
    /// printable ASCII characters each produce a unique glyph_id, so
    /// caching by char is safe.
    #[test]
    fn printable_ascii_glyphs_are_unique() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let mut seen = std::collections::HashMap::<u16, char>::new();
        for ch in '!'..='~' {
            if let Some(key) = shape_char_to_key(&mut fs, ch, font_size, cell_h, cell_w) {
                if let Some(&prev_ch) = seen.get(&key.glyph_id) {
                    panic!("glyph_id {} collides between '{}' and '{}'", key.glyph_id, prev_ch, ch);
                }
                seen.insert(key.glyph_id, ch);
            }
        }
    }

    /// Shaping must be deterministic across many calls so the per-frame
    /// cache is safe to rebuild each frame.
    #[test]
    fn shaping_is_deterministic_across_many_calls() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let reference = shape_char_to_key(&mut fs, 'X', font_size, cell_h, cell_w)
            .expect("'X' should produce a glyph");
        for _ in 0..100 {
            let key = shape_char_to_key(&mut fs, 'X', font_size, cell_h, cell_w)
                .expect("'X' should produce a glyph");
            assert_eq!(reference, key, "GlyphKey must be stable across repeated shaping");
        }
    }

    /// font_size changes must produce different GlyphKeys, validating
    /// that the frame-local cache (rebuilt each frame with potentially
    /// different font_size) does not serve stale entries.
    #[test]
    fn different_font_size_produces_different_key() {
        let mut fs = FontSystem::new();
        let cell_w = 8.4;

        let key_14 = shape_char_to_key(&mut fs, 'M', 14.0, 14.0 * 1.2, cell_w)
            .expect("'M' at 14pt should produce a glyph");
        let key_20 = shape_char_to_key(&mut fs, 'M', 20.0, 20.0 * 1.2, cell_w)
            .expect("'M' at 20pt should produce a glyph");
        assert_ne!(
            key_14.font_size_tenths, key_20.font_size_tenths,
            "different font sizes must produce different font_size_tenths"
        );
    }

    // -----------------------------------------------------------------------
    // CellMetricsCache tests.
    //
    // The cache must cross frames: a second lookup with identical
    // (font_family, font_size, line_height, scale_factor) must NOT re-measure.
    // Any change to any of those four components must invalidate.
    // -----------------------------------------------------------------------

    #[test]
    fn cell_metrics_cache_is_empty_on_construction() {
        let cache = CellMetricsCache::new();
        assert!(cache.is_empty(), "freshly constructed cache must have no entries");
        assert_eq!(cache.miss_count(), 0, "freshly constructed cache must have 0 misses");
    }

    #[test]
    fn cell_metrics_cache_hits_identical_lookup() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();

        let family = monospace_family_name();
        let first = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        let second = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);

        assert_eq!(first, second, "identical inputs must return identical metrics");
        assert_eq!(
            cache.miss_count(),
            1,
            "second lookup with identical inputs must be a cache hit (misses stays at 1)"
        );
        assert_eq!(cache.len(), 1, "cache should contain exactly one entry");
    }

    #[test]
    fn cell_metrics_cache_misses_on_font_size_change() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();
        let family = monospace_family_name();

        let _a = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        let _b = cache.get_or_measure(&mut fs, family, 20.0, 20.0 * 1.2, 1.0);

        assert_eq!(cache.miss_count(), 2, "different font size must miss");
        assert_eq!(cache.len(), 2, "two distinct sizes must produce two entries");
    }

    #[test]
    fn cell_metrics_cache_misses_on_scale_factor_change() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();
        let family = monospace_family_name();

        let _a = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        let _b = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.5);
        let _c = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 2.0);

        assert_eq!(cache.miss_count(), 3, "different scale factor must miss");
        assert_eq!(cache.len(), 3, "three distinct scales must produce three entries");
    }

    #[test]
    fn cell_metrics_cache_misses_on_font_family_change() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();

        let _a = cache.get_or_measure(&mut fs, "Consolas", 14.0, 14.0 * 1.2, 1.0);
        let _b = cache.get_or_measure(&mut fs, "Menlo", 14.0, 14.0 * 1.2, 1.0);

        assert_eq!(cache.miss_count(), 2, "different font family must miss");
        assert_eq!(cache.len(), 2, "two families must produce two entries");
    }

    #[test]
    fn cell_metrics_cache_populates_both_cell_w_and_cell_h() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();

        let metrics = cache.get_or_measure(&mut fs, monospace_family_name(), 14.0, 14.0 * 1.2, 1.0);

        assert!(metrics.cell_w > 0.0, "cell_w must be positive");
        assert!(metrics.cell_h > 0.0, "cell_h must be positive");
        assert_eq!(
            metrics.cell_h.to_bits(),
            (14.0_f32 * 1.2).to_bits(),
            "cell_h must equal line_height"
        );
    }

    #[test]
    fn cell_metrics_cache_clear_resets_entries_but_preserves_miss_count() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();
        let family = monospace_family_name();

        let _ = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty(), "clear must empty the entry map");

        let _ = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        assert_eq!(cache.miss_count(), 2, "miss counter is monotonic across clear");
    }

    #[test]
    fn cell_metrics_key_is_stable_for_fractional_sizes() {
        let k1 = CellMetricsKey::new("Consolas", 14.0, 16.8, 1.0);
        let k2 = CellMetricsKey::new("Consolas", 14.0, 16.8, 1.0);
        assert_eq!(k1, k2, "identical fractional inputs must produce identical keys");
    }

    // -----------------------------------------------------------------------
    // ShapeCache tests.
    //
    // The ShapeCache is a cross-frame cache. Tests verify:
    //   * Cache hit on second lookup.
    //   * Cache miss on font change.
    //   * Cache miss on DPI change.
    //   * Cache miss on size change.
    //   * Preload warmup.
    //   * Cross-frame persistence (no implicit per-frame clear).
    // -----------------------------------------------------------------------

    /// Tiny fake shaped entry used by ShapeCache tests so the unit tests do
    /// not depend on cosmic-text shaping pipelines.
    fn fake_shape(ch: char) -> Option<ShapedGlyphEntry> {
        // Build a LayoutGlyph minimal enough to satisfy the cache contract.
        // The cache treats the value as opaque, so any LayoutGlyph instance
        // that can be cloned is fine. We derive one via cosmic-text so the
        // type stays correct across upstream changes.
        use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};
        let mut fs = FontSystem::new();
        let mut buf = Buffer::new(&mut fs, Metrics::new(14.0, 16.8));
        buf.set_size(&mut fs, Some(1000.0), None);
        let mut tmp = [0u8; 4];
        let s = ch.encode_utf8(&mut tmp);
        buf.set_text(
            &mut fs,
            s,
            Attrs::new().family(cosmic_text::Family::Monospace),
            Shaping::Basic,
        );
        buf.shape_until_scroll(&mut fs, false);
        buf.layout_runs().find_map(|run| {
            run.glyphs
                .first()
                .cloned()
                .map(|g| ShapedGlyphEntry { layout_glyph: g, line_y: run.line_y })
        })
    }

    fn key_for(ch: char, font: &str, size: f32) -> ShapeCacheKey {
        ShapeCacheKey {
            ch,
            font_id: shape_cache_font_id(font),
            style: 0,
            font_size_tenths: (size * 10.0).round() as u32,
        }
    }

    #[test]
    fn shape_cache_is_empty_on_construction() {
        let cache = ShapeCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 0);
    }

    #[test]
    fn shape_cache_reports_hit_on_second_lookup() {
        let mut cache = ShapeCache::new();
        let key = key_for('A', "Consolas", 14.0);

        // First lookup: miss + insert.
        assert!(cache.get(&key).is_none());
        cache.insert(key, fake_shape('A'));

        // Second lookup: hit.
        assert!(cache.get(&key).is_some());

        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);
        assert!((cache.hit_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn shape_cache_misses_on_font_change() {
        let mut cache = ShapeCache::new();
        let key_a = key_for('A', "Consolas", 14.0);
        let key_b = key_for('A', "Menlo", 14.0);

        cache.insert(key_a, fake_shape('A'));
        assert!(cache.get(&key_b).is_none(), "different font must miss");
    }

    #[test]
    fn shape_cache_misses_on_size_change() {
        let mut cache = ShapeCache::new();
        let key_small = key_for('A', "Consolas", 14.0);
        let key_large = key_for('A', "Consolas", 20.0);

        cache.insert(key_small, fake_shape('A'));
        assert!(cache.get(&key_large).is_none(), "different size must miss");
    }

    #[test]
    fn shape_cache_retune_invalidates_on_font_change() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        let key = key_for('A', "Consolas", 14.0);
        cache.insert(key, fake_shape('A'));
        assert_eq!(cache.len(), 1);

        // Switching the font family must drop the cache.
        cache.retune("Menlo", 1.0, 14.0);
        assert!(cache.is_empty(), "retune with a different font family must clear the cache");
    }

    #[test]
    fn shape_cache_retune_invalidates_on_dpi_change() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));

        cache.retune("Consolas", 2.0, 14.0);
        assert!(cache.is_empty(), "retune with a new DPI must clear the cache");
    }

    #[test]
    fn shape_cache_retune_invalidates_on_size_change() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));

        cache.retune("Consolas", 1.0, 20.0);
        assert!(cache.is_empty(), "retune with a new size must clear the cache");
    }

    #[test]
    fn shape_cache_retune_is_noop_when_identical() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));
        assert_eq!(cache.len(), 1);

        cache.retune("Consolas", 1.0, 14.0);
        assert_eq!(cache.len(), 1, "identical retune must not drop entries");
    }

    #[test]
    fn shape_cache_preload_fills_ascii_and_box_drawing() {
        let mut cache = ShapeCache::new();
        let mut shaped_count = 0u32;
        cache.preload_defaults(&mut |ch| {
            shaped_count += 1;
            (key_for(ch, "Consolas", 14.0), fake_shape(ch))
        });

        // 0x20..=0x7e is 95 chars, 0x2500..=0x257f is 128 chars. The iterator
        // yields the union; we assert both are represented in the cache.
        assert_eq!(shaped_count, 95 + 128, "preload must visit all defaults");
        assert!(cache.len() >= 95 + 128);
        assert!(cache.get(&key_for(' ', "Consolas", 14.0)).is_some());
        assert!(cache.get(&key_for('~', "Consolas", 14.0)).is_some());
        assert!(cache.get(&key_for('\u{2500}', "Consolas", 14.0)).is_some()); // box drawings: light horizontal
        assert!(cache.get(&key_for('\u{257f}', "Consolas", 14.0)).is_some());
    }

    #[test]
    fn shape_cache_persists_across_simulated_frames() {
        // Simulates two frames: second frame touches the same char twice and
        // must observe zero additional misses.
        let mut cache = ShapeCache::new();
        let key = key_for('x', "Consolas", 14.0);

        // Frame 1: miss + insert.
        assert!(cache.get(&key).is_none());
        cache.insert(key, fake_shape('x'));
        assert_eq!(cache.misses(), 1);

        // Frame 2: two lookups, both hits.
        assert!(cache.get(&key).is_some());
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.misses(), 1, "cross-frame reuse must NOT add misses");
        assert_eq!(cache.hits(), 2);
    }

    #[test]
    fn shape_cache_default_preload_chars_covers_ascii_and_box_drawing() {
        let chars: Vec<char> = ShapeCache::default_preload_chars().collect();
        assert!(chars.contains(&' '));
        assert!(chars.contains(&'~'));
        assert!(chars.contains(&'A'));
        assert!(chars.contains(&'\u{2500}'));
        assert!(chars.contains(&'\u{257f}'));
        assert_eq!(chars.len(), 95 + 128);
    }
}
