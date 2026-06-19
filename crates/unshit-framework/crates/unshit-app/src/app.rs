use crate::animation_waker::AnimationWaker;
use crate::clipboard::ClipboardContext;
use crate::event_sink::{EventSink, ExternalEvent};
use crate::notification::{AttentionUrgency, BellConfig, BellState};
use crate::scroll_motion::{
    browser_like_initial_slope, browser_like_wheel_duration, dominant_delta, ScrollMotion,
    SMOOTH_SCROLL_EPSILON,
};
use crate::shortcut::{key_combo_from_winit, ShortcutResolver};
use crate::window;
use cosmic_text::{FontSystem, SwashCache};
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use unshit_core::build::{
    build_tree_from_def, dispatch_resize_callbacks, mark_layout_dirty, mark_node_paint_dirty,
    mark_paint_dirty, resolve_all_styles, resolve_all_styles_with_transitions, run_layout_pipeline,
    scale_all_styles, sync_all_animations, tick_all_animations, tick_all_transitions,
};
use unshit_core::dirty::DirtyFlags;
use unshit_core::element::*;
use unshit_core::event::*;
use unshit_core::frame_arena::FrameArena;
use unshit_core::id::NodeId;
use unshit_core::layout::{self, TextMeasureCache, TextMeasureCtx};
use unshit_core::scroll::{self, ScrollbarAxis, ScrollbarPart, ScrollbarVisualState};
use unshit_core::style::animation::AnimationDriver;
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::style::theme::Theme;
use unshit_core::style::transition::ActiveTransitions;
use unshit_core::style::types::Layer;
use unshit_core::tree::NodeArena;
use unshit_renderer::batch::{self, BatchCache, ShapeCache, ShapedTextCache};
use unshit_renderer::batch::{Rasterizer, SubpixelSwashCache};
use unshit_renderer::canvas::{CanvasRegistry, CustomPainter};
#[cfg(target_os = "windows")]
use unshit_renderer::dw_rasterizer::DwRasterizer;
use unshit_renderer::gpu::GpuContext;
use unshit_renderer::pipeline::quad::QuadInstance;
use winit::application::ApplicationHandler;
use winit::cursor::CursorIcon;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{ResizeDirection, Window, WindowId};

pub const DEFAULT_WHEEL_LINE_SCROLL_PX: f32 = 100.0;
pub const DEFAULT_SMOOTH_SCROLL_DURATION_MS: u64 = 180;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollTuning {
    pub line_scroll_px: f32,
    pub smooth_scroll_duration_ms: u64,
}

impl Default for ScrollTuning {
    fn default() -> Self {
        Self {
            line_scroll_px: DEFAULT_WHEEL_LINE_SCROLL_PX,
            smooth_scroll_duration_ms: DEFAULT_SMOOTH_SCROLL_DURATION_MS,
        }
    }
}

impl ScrollTuning {
    pub fn sanitized(self) -> Self {
        let line_scroll_px = if self.line_scroll_px.is_finite() {
            self.line_scroll_px.clamp(8.0, 320.0)
        } else {
            DEFAULT_WHEEL_LINE_SCROLL_PX
        };
        Self {
            line_scroll_px,
            smooth_scroll_duration_ms: self.smooth_scroll_duration_ms.clamp(16, 500),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollTelemetryPhase {
    Started,
    Frame,
    Completed,
    Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollTelemetry {
    pub phase: ScrollTelemetryPhase,
    pub node_id: NodeId,
    pub elapsed_ms: f32,
    pub duration_ms: f32,
    pub start_x: f32,
    pub start_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub progress_y: f32,
}

pub type ScrollTelemetryCallback = dyn Fn(&ScrollTelemetry) + Send;

pub struct AppConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub decorations: bool,
    pub css: String,
    pub keybindings_path: Option<String>,
    /// Callback invoked for [`ExternalEvent::Custom`] payloads.
    /// Called on the UI thread inside `proxy_wake_up`.
    #[allow(clippy::type_complexity)]
    pub on_external_event: Option<Box<dyn Fn(Box<dyn std::any::Any + Send>) + Send>>,
    /// Callback invoked for [`ExternalEvent::Bytes`] payloads.
    /// Called on the UI thread inside `proxy_wake_up`.
    pub on_bytes: Option<Box<dyn Fn(std::sync::Arc<[u8]>) + Send>>,
    /// Additional keyboard shortcuts registered at startup. Each entry
    /// pairs a shortcut string (parsed via [`Shortcut::parse`]) with a
    /// command name that will be passed to [`Self::on_command`] when the
    /// shortcut fires. Example: `("Ctrl+K".to_string(), "palette.open".to_string())`.
    pub user_shortcuts: Vec<(String, String)>,
    /// Callback invoked when a non-default command is dispatched (either
    /// from [`Self::user_shortcuts`] or any other shortcut the framework
    /// does not recognize). The handler is called on the UI thread and
    /// returns `true` if the application should rebuild its tree.
    #[allow(clippy::type_complexity)]
    pub on_command: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    /// Callback invoked before the shortcut resolver runs, receiving the
    /// parsed [`KeyCombo`] for every key press. Returning `true` consumes
    /// the event: the resolver is skipped, no command fires, and the
    /// framework requests a rebuild. Returning `false` passes through to
    /// normal shortcut resolution.
    ///
    /// Intended for features that need to capture arbitrary key combos
    /// (settings UIs recording a new binding, text-entry modes that
    /// temporarily suppress hotkeys). The hook runs in both normal and
    /// capture-mode key flows so it behaves the same whether a terminal
    /// pane holds keyboard focus or not.
    #[allow(clippy::type_complexity)]
    pub on_raw_key: Option<Arc<dyn Fn(&unshit_core::shortcut::KeyCombo) -> bool + Send + Sync>>,
    /// Additional fonts registered exactly once at startup.
    ///
    /// Entries are loaded into the cosmic-text `FontSystem` inside
    /// `can_create_surfaces`, right after OS font discovery. Any
    /// `@font-face` rules found in `css` are loaded next, in source order.
    ///
    /// Relative [`FontSource::Path`] entries resolve against the current
    /// working directory at load time. [`FontSource::System`] entries are
    /// recorded but not loaded: they exist for the future fallback chain.
    ///
    /// Failed entries (missing files, corrupt bytes) are logged at warn
    /// level and skipped. A single bad entry never aborts startup.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use unshit_app::{AppConfig, FontSource};
    /// use std::path::PathBuf;
    ///
    /// let config = AppConfig {
    ///     css: "body { font-family: \"Inter\"; }".into(),
    ///     fonts: vec![FontSource::Path(PathBuf::from("assets/Inter.ttf"))],
    ///     ..AppConfig::default()
    /// };
    /// # let _ = config;
    /// ```
    pub fonts: Vec<crate::font::FontSource>,
    /// Priority-ordered font family names used for glyph fallback.
    pub fallback_chain: crate::font::FallbackChain,
    pub theme: Theme,
    /// Maximum atlas memory in bytes.
    pub max_atlas_bytes: Option<usize>,
    /// Path to a CSS file for hot-reload. When set with the `hot-reload`
    /// feature enabled, the framework watches this file and re-parses on change.
    pub css_path: Option<std::path::PathBuf>,
    /// Callback invoked after each rendered frame with the collected metrics.
    /// Called on the UI thread immediately after the frame is complete.
    pub on_frame_metrics: Option<Box<dyn Fn(&FrameMetrics) + Send>>,
    /// Callback read on wheel input to tune line-wheel scroll distance and
    /// animation duration without rebuilding the app.
    pub scroll_tuning: Option<Arc<dyn Fn() -> ScrollTuning + Send + Sync>>,
    /// Callback invoked when smooth scrolling starts and on each animation
    /// sample. Intended for diagnostics and regression metrics.
    pub on_scroll_telemetry: Option<Box<ScrollTelemetryCallback>>,
    /// Callback invoked when the OS reports the DPI scale factor.
    /// Fires once at startup and again whenever the window moves between
    /// monitors with different scale factors.
    pub on_scale_factor: Option<Arc<dyn Fn(f32) + Send + Sync>>,
    /// Callback invoked when the window enters or leaves the maximized state.
    /// Fires once at startup with the initial state, after framework-driven
    /// maximize toggles, and when OS window events report a state change.
    pub on_window_maximized: Option<Arc<dyn Fn(bool) + Send + Sync>>,
    /// Callback invoked when the window's close button is clicked.
    /// Returning `true` lets the framework proceed with exit; returning
    /// `false` vetoes the close so the application can show a confirm
    /// prompt and either issue its own `process::exit` once the user
    /// confirms, or leave the window alive if the user cancels. When the
    /// callback is unset the framework exits unconditionally.
    pub on_close: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// One-shot callback invoked once the renderer publishes valid cell
    /// metrics (cell width and height in pixels). Fires after the first
    /// render pass that produces non-zero values, giving the application a
    /// reliable point to compute initial PTY dimensions.
    pub on_cell_metrics: Option<Arc<dyn Fn(f32, f32) + Send + Sync>>,
    /// Callback invoked on every rendered frame right after
    /// `record_frame_presented` runs, with a fresh snapshot of the
    /// input latency histograms.
    ///
    /// Only present when the `input-latency-histogram` cargo feature is
    /// enabled. Lets callers pipe snapshots into bench aggregation or a
    /// debug HUD without polling.
    #[cfg(feature = "input-latency-histogram")]
    #[allow(clippy::type_complexity)]
    pub on_input_latency: Option<Box<dyn Fn(&crate::input_latency::InputLatencySnapshot) + Send>>,
    /// Optional arena-aware tree function. When set, the frame loop
    /// allocates the tree into the per-frame [`FrameArena`] and routes
    /// through the bump reconcile path instead of the owned
    /// [`unshit_core::element::ElementTree`] returned by
    /// [`App::new`]'s `tree_fn`.
    ///
    /// This is ADDITIVE: leaving it `None` keeps the owned path intact.
    pub tree_fn_bump: Option<TreeFnBump>,
}

/// Type alias for the bump-aware tree builder. The closure must be valid
/// for any arbitrary arena lifetime, which is why the signature uses an
/// HRTB `for<'a>`.
pub type TreeFnBump =
    Box<dyn for<'a> Fn(&'a FrameArena) -> unshit_core::element::ElementTreeBump<'a>>;

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "unshit".to_string(),
            width: 800,
            height: 600,
            decorations: true,
            css: String::new(),
            keybindings_path: None,
            on_external_event: None,
            on_bytes: None,
            user_shortcuts: Vec::new(),
            on_command: None,
            on_raw_key: None,
            fonts: Vec::new(),
            fallback_chain: crate::font::FallbackChain::default_chain(),
            theme: Theme::dark(),
            max_atlas_bytes: None,
            css_path: None,
            on_frame_metrics: None,
            scroll_tuning: None,
            on_scale_factor: None,
            on_window_maximized: None,
            on_close: None,
            on_cell_metrics: None,
            on_scroll_telemetry: None,
            #[cfg(feature = "input-latency-histogram")]
            on_input_latency: None,
            tree_fn_bump: None,
        }
    }
}

pub struct App {
    config: AppConfig,
    tree_fn: Box<dyn Fn() -> ElementTree>,
    state: Option<AppState>,
    event_tx: flume::Sender<ExternalEvent>,
    event_rx: flume::Receiver<ExternalEvent>,
    proxy_cell: Arc<OnceLock<EventLoopProxy>>,
    /// The single persistent animation waker shared by every animation
    /// producer (container smooth scroll, grid-animation hooks). Replaces
    /// the per-wheel-notch waker threads; see [`crate::animation_waker`].
    animation_waker: AnimationWaker,
    canvas_registry: CanvasRegistry,
    clipboard: Arc<ClipboardContext>,
    #[cfg(feature = "async")]
    runtime: crate::runtime::AsyncRuntime,
    #[cfg(feature = "async")]
    subscription_manager: Option<crate::subscription::SubscriptionManager>,
}

/// Per-frame performance metrics.
#[derive(Clone, Debug, Default)]
pub struct FrameMetrics {
    pub tree_build_us: u64,
    pub style_resolve_us: u64,
    pub scale_us: u64,
    pub layout_us: u64,
    pub batch_build_us: u64,
    /// CPU time spent encoding and submitting the frame's render passes
    /// (wall time of `gpu.render()` on the render thread, minus the
    /// swapchain acquire wait reported in [`Self::present_wait_us`]).
    /// Not GPU execution time: submission is non-blocking and no
    /// timestamp queries exist yet, so actual GPU cost is invisible
    /// here. The overlay surfaces this as "encode".
    pub gpu_render_us: u64,
    pub total_us: u64,
    pub node_count: usize,
    pub rss_bytes: usize,
    pub nodes_visited: u32,
    pub nodes_skipped: u32,
    pub quad_count: u32,
    pub glyph_count: u32,
    pub atlas_fill_ratio: f32,
    pub gpu_upload_bytes: u64,
    pub damage_area_px: u64,
    /// Current frame pacer coalescing interval in nanoseconds, derived
    /// from the active monitor's refresh rate (see [`crate::frame_pacer`]).
    /// Constant across frames on a stationary window; changes when the
    /// window crosses monitor boundaries. Used by the bench harness to
    /// report the effective frame-rate ceiling alongside measured fps.
    pub pacer_min_interval_ns: u64,
    /// Wall-clock microseconds between the completion of this paint and
    /// the completion of the previous paint, measured where `total_us`
    /// is finalized. Captures the real presentation cadence rather than
    /// per-frame CPU work. 0 when there is no cadence to measure: the
    /// first painted frame, which has no predecessor, and paints after
    /// an idle gap of `ACTIVITY_WINDOW` or longer (e.g. the 500ms
    /// cursor-blink repaint of an otherwise idle session), which
    /// represent intentional idling rather than missed refreshes.
    pub present_interval_us: u64,
    /// Active monitor's refresh period in nanoseconds
    /// (`1e12 / refresh_mhz`). 0 when the platform cannot report the
    /// monitor's refresh rate; consumers should fall back to
    /// [`Self::pacer_min_interval_ns`] in that case.
    pub display_period_ns: u64,
    /// CPU time `gpu.render()` spent blocked acquiring the swapchain
    /// image — up to one display period under a vsync-paced present
    /// mode, ~0 otherwise. Already subtracted out of
    /// [`Self::gpu_render_us`] and [`Self::total_us`] at the source so
    /// every work-time consumer (bench percentiles, overlay rows, slow
    /// frame logs) keeps measuring work; reported separately so the
    /// vblank wait stays observable.
    pub present_wait_us: u64,
}

impl std::fmt::Display for FrameMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frame {:.1}ms | tree {:.1}ms  style {:.1}ms  scale {:.1}ms  layout {:.1}ms  batch {:.1}ms  gpu {:.1}ms | nodes {} | quads {} glyphs {} | rss {:.1}MB",
            self.total_us as f64 / 1000.0,
            self.tree_build_us as f64 / 1000.0,
            self.style_resolve_us as f64 / 1000.0,
            self.scale_us as f64 / 1000.0,
            self.layout_us as f64 / 1000.0,
            self.batch_build_us as f64 / 1000.0,
            self.gpu_render_us as f64 / 1000.0,
            self.node_count,
            self.quad_count,
            self.glyph_count,
            self.rss_bytes as f64 / (1024.0 * 1024.0),
        )
    }
}

struct AppState {
    window: Arc<dyn Window>,
    gpu: GpuContext,
    arena: NodeArena,
    taffy: taffy::TaffyTree<TextMeasureCtx>,
    root: NodeId,
    stylesheet: CompiledStylesheet,
    font_system: FontSystem,
    swash_cache: SwashCache,
    subpixel_swash_cache: SubpixelSwashCache,
    #[cfg(target_os = "windows")]
    dw_rasterizer: DwRasterizer,
    interaction: InteractionState,
    needs_rebuild: bool,
    needs_restyle: bool,
    needs_relayout: bool,
    /// When `Some`, the next `needs_restyle` pass cascades from this node
    /// instead of the document root. Set by hover / focus / active state
    /// changes to the lowest common ancestor of the leaving and entering
    /// node, narrowing the cascade to the smallest subtree that could
    /// contain a re-evaluating pseudo-class selector. Cleared by `take()`
    /// when the restyle pass consumes it. A full rebuild ignores it.
    restyle_root: Option<NodeId>,
    scale_factor: f32,
    window_maximized: bool,
    zoom_factor: f32,
    ctrl_held: bool,
    shift_held: bool,
    modifiers_state: ModifiersState,
    shortcut_resolver: ShortcutResolver,
    measure_cache: TextMeasureCache,
    shaped_cache: ShapedTextCache,
    batch_cache: BatchCache,
    /// Cross-frame cache of shaped prototype glyphs for the terminal grid.
    /// Populated lazily as characters appear; preloaded with ASCII and box
    /// drawing at startup. See [`ShapeCache`] for invalidation semantics.
    shape_cache: ShapeCache,
    line_quad_cache: unshit_renderer::line_quad_cache::LineQuadCache,
    canvas_registry: CanvasRegistry,
    last_metrics: FrameMetrics,
    frame_count: u64,
    fps_timer: Instant,
    current_fps: f32,
    app_title: String,
    event_log: Option<Vec<String>>,
    event_log_start: Instant,
    scrollbar_visual: ScrollbarVisualState,
    smooth_scroll: Option<SmoothScroll>,
    /// Framework-driven per-node grid animations, registered by Scroll
    /// handlers through [`ScrollGridPatch::animation`] and ticked at the
    /// shared animation cadence. See [`tick_grid_animations`].
    grid_animations: HashMap<NodeId, GridAnimationHook>,
    /// Timer-fallback due-gate shared by all animation sources (container
    /// smooth scroll and grid animations): the earliest instant the next
    /// animation frame may paint. `None` when no animation is active.
    /// Unused in [`PacingMode::VsyncBlocking`], where the blocking
    /// swapchain acquire paces the chain instead.
    animation_next_frame: Option<Instant>,
    /// Set by a Timer-fallback [`AnimationWaker`] tick so the next
    /// `RedrawRequested` bypasses the due-gate and the pacer gate once.
    /// Never set in [`PacingMode::VsyncBlocking`] (the waker never runs).
    force_animation_paint: bool,
    active_transitions: ActiveTransitions,
    animation_driver: AnimationDriver,
    pseudo_table: unshit_core::style::pseudo::PseudoSideTable,
    release_chord: unshit_core::shortcut::KeyCombo,
    bell_state: BellState,
    // Stored for future CSS variable resolution and runtime theme switching.
    #[allow(dead_code)]
    theme: Theme,
    /// Whether the one-shot on_cell_metrics callback has already fired.
    cell_metrics_fired: bool,
    /// How this session paces paints, fixed at startup from the surface's
    /// present mode. See [`PacingMode`].
    pacing_mode: PacingMode,
    /// In [`PacingMode::Timer`], coalesces redraw requests into at most
    /// one paint per `FramePacer::min_interval` (the display's true
    /// period), preventing per-PTY-chunk rebuild storms from dominating
    /// the event loop. In [`PacingMode::VsyncBlocking`] the gate is
    /// skipped — the blocking acquire is the coalescer — and the pacer
    /// survives as the metrics floor feeding
    /// [`FrameMetrics::pacer_min_interval_ns`]. See [`crate::frame_pacer`].
    frame_pacer: crate::frame_pacer::FramePacer,
    /// Timestamp of the most recent external-event-driven wake (proxy wake,
    /// keyboard input, pointer input, resize, etc.). Used by `about_to_wait`
    /// to decide whether to schedule a speculative repaint at the pacer
    /// deadline. While this value is within [`ACTIVITY_WINDOW`] of now, the
    /// event loop repaints at the pacer rhythm even without a dirty flag so
    /// that a PTY chunk or keystroke landing mid-interval reaches the screen
    /// on the very next frame. After [`ACTIVITY_WINDOW`] of silence the loop
    /// falls back to `ControlFlow::Wait` and idle CPU returns to ~zero.
    last_activity: Instant,
    /// Rolling window of per-frame durations. Always present. Emits
    /// p50/p95/p99 quantiles once per second via `log::info!` only while
    /// the runtime enable flag is set; debug builds default to enabled
    /// to preserve the previous behavior, release builds default to
    /// disabled and rely on the in-app FPS overlay (or other callers)
    /// flipping the flag on. See [`crate::frame_probe`].
    frame_probe: crate::frame_probe::FrameProbe,
    /// Rolling window of paint-to-paint wall-clock intervals: the
    /// presentation-cadence counterpart of [`Self::frame_probe`]'s CPU
    /// work times. Emits `[FRAME-INTERVAL]` quantiles once per second
    /// under the same runtime gating. See [`crate::frame_probe`].
    interval_probe: crate::frame_probe::FrameProbe,
    /// Completion timestamp of the most recent paint, taken once per
    /// painted frame right after `total_us` is finalized. Source of
    /// [`FrameMetrics::present_interval_us`]. `None` until the first
    /// frame has painted.
    last_paint_completed_at: Option<Instant>,
    /// Active monitor's refresh rate in millihertz, as last reported by
    /// the platform; 0 when unknown. Set at startup and kept in sync
    /// with the frame pacer in [`refresh_pacer_from_window`]. Source of
    /// [`FrameMetrics::display_period_ns`].
    display_refresh_mhz: u32,
    /// Nanosecond-grained input latency histograms. See
    /// [`crate::input_latency`]. Only present when the
    /// `input-latency-histogram` cargo feature is enabled; the field
    /// disappears entirely from the struct layout otherwise.
    #[cfg(feature = "input-latency-histogram")]
    pub(crate) input_latency: crate::input_latency::InputLatencyTracker,
    /// Timestamp of the last time the pacer re-read the current
    /// monitor's refresh rate. `WindowEvent::Moved` fires once per mouse
    /// move during a drag; we debounce monitor probes to at most once per
    /// [`ACTIVITY_WINDOW`] to avoid hitting the compositor on every
    /// pixel of motion. See [`refresh_pacer_from_window`].
    last_refresh_probe: Instant,
    /// Per-frame bump allocator for the transient [`ElementDefBump`]
    /// tree. Reset at the end of each rendered frame; preserves chunk
    /// capacity across resets so steady-state allocation work drops to
    /// zero. Only used when [`AppConfig::tree_fn_bump`] is set; left
    /// unused otherwise.
    frame_arena: FrameArena,
}

const WINDOW_RESIZE_GRIP_SIZE: f32 = 14.0;

fn window_resize_direction(
    surface_size: PhysicalSize<u32>,
    cursor: (f32, f32),
    scale_factor: f32,
) -> Option<ResizeDirection> {
    if surface_size.width == 0 || surface_size.height == 0 {
        return None;
    }

    let (x, y) = cursor;
    let width = surface_size.width as f32;
    let height = surface_size.height as f32;
    let grip = WINDOW_RESIZE_GRIP_SIZE * scale_factor.max(1.0);

    let near_left = x <= grip && x <= width * 0.5;
    let near_right = x >= width - grip && x > width * 0.5;
    let near_top = y <= grip && y <= height * 0.5;
    let near_bottom = y >= height - grip && y > height * 0.5;

    match (near_left, near_right, near_top, near_bottom) {
        (true, false, true, false) => Some(ResizeDirection::NorthWest),
        (false, true, true, false) => Some(ResizeDirection::NorthEast),
        (true, false, false, true) => Some(ResizeDirection::SouthWest),
        (false, true, false, true) => Some(ResizeDirection::SouthEast),
        (true, false, false, false) => Some(ResizeDirection::West),
        (false, true, false, false) => Some(ResizeDirection::East),
        (false, false, true, false) => Some(ResizeDirection::North),
        (false, false, false, true) => Some(ResizeDirection::South),
        _ => None,
    }
}

fn custom_window_resize_direction(
    decorations: bool,
    surface_size: PhysicalSize<u32>,
    cursor: (f32, f32),
    scale_factor: f32,
) -> Option<ResizeDirection> {
    if decorations {
        None
    } else {
        window_resize_direction(surface_size, cursor, scale_factor)
    }
}

fn resize_direction_cursor_icon(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
        ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
    }
}

/// How long after the last external event the loop keeps scheduling
/// speculative repaints. 250ms is long enough to coalesce bursts of
/// keystrokes or PTY chunks without keeping the CPU warm after activity
/// stops. Matches Ghostty's active-renderer-window concept.
pub(crate) const ACTIVITY_WINDOW: Duration = Duration::from_millis(250);
const SMOOTH_SCROLL_WAKE_GRACE: Duration = Duration::from_millis(48);

/// How the frame loop paces paints, chosen once at startup from the
/// surface's present mode (see [`pacing_mode_for_surface`]). The mode
/// cannot change at runtime because surface reconfigures reuse the stored
/// present mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PacingMode {
    /// The surface's present mode blocks in the swapchain acquire
    /// (Fifo family): a saturated paint loop runs at exactly the display
    /// refresh rate, so painting self-paces. Animations run the classic
    /// vsync loop — paint, `request_redraw`, block at the next acquire —
    /// with no timers, no due-gates, and no pacer gate.
    VsyncBlocking,
    /// The surface never blocks on vblank (Mailbox / Immediate, exotic
    /// surfaces only). The legacy timer machinery — the [`FramePacer`]
    /// gate, the speculative deadline, and the [`AnimationWaker`] —
    /// paces paints at the display's true period instead.
    Timer,
}

/// Pure mapping from the renderer's pacing capability
/// ([`GpuContext::is_vsync_paced`]) to the app's [`PacingMode`].
fn pacing_mode_for_surface(is_vsync_paced: bool) -> PacingMode {
    if is_vsync_paced {
        PacingMode::VsyncBlocking
    } else {
        PacingMode::Timer
    }
}

/// Display period to assume when the platform cannot report a refresh
/// rate: one 120Hz frame. Only used to schedule skipped-frame resumes,
/// where a slightly wrong period costs at most one frame of animation
/// idle time.
const FALLBACK_DISPLAY_PERIOD: Duration = Duration::from_nanos(8_333_333);

/// When an animation tick produced no visual change, nothing was
/// presented and no acquire will block, so chaining a `request_redraw`
/// would tick at CPU speed. Park the loop until the next display period
/// instead, anchored to the skipping frame's own start so consecutive
/// skips always schedule a strictly future wake (a stale
/// `last_paint_completed_at` would collapse the deadline into the past
/// and spin the loop through zero-timeout waits).
fn skipped_frame_resume_at(frame_start: Instant, display_period_ns: u64) -> Instant {
    let period = if display_period_ns == 0 {
        FALLBACK_DISPLAY_PERIOD
    } else {
        Duration::from_nanos(display_period_ns)
    };
    frame_start + period
}

/// The timestamp animation positions are sampled at: the predicted
/// present time of the frame being built, i.e. one display period past
/// the previous paint's completion. Sampling at wake time instead turns
/// scheduler jitter into spatial velocity noise (finding 16 of the
/// scroll-smoothness spec). Falls back to `frame_start` when prediction
/// is meaningless: no previous paint, unknown display period, or an idle
/// cadence break of [`ACTIVITY_WINDOW`] or longer. The result is clamped
/// to never precede `frame_start`, so sampled timestamps stay monotone
/// across frames even after a missed vblank.
pub(crate) fn predicted_present_ts(
    last_paint_completed_at: Option<Instant>,
    display_period_ns: u64,
    frame_start: Instant,
) -> Instant {
    let Some(last) = last_paint_completed_at else {
        return frame_start;
    };
    if display_period_ns == 0 || frame_start.saturating_duration_since(last) >= ACTIVITY_WINDOW {
        return frame_start;
    }
    (last + Duration::from_nanos(display_period_ns)).max(frame_start)
}
/// Fallback lines-per-notch when the OS setting cannot be queried. Matches
/// the Windows default of 3 wheel-scroll lines per detent.
const WHEEL_LINE_DELTA_PER_NOTCH: f32 = 3.0;

fn should_check_shortcut_during_keyboard_capture(combo: &unshit_core::shortcut::KeyCombo) -> bool {
    combo.modifiers.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META)
        || matches!(combo.key, Key::F(_))
}

fn consume_raw_key_hook(config: &AppConfig, combo: &unshit_core::shortcut::KeyCombo) -> bool {
    config.on_raw_key.as_ref().is_some_and(|f| f(combo))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SmoothScroll {
    node_id: NodeId,
    start_x: f32,
    start_y: f32,
    target_x: f32,
    target_y: f32,
    started_at: Instant,
    duration: Duration,
    initial_slope: f32,
}

impl SmoothScroll {
    fn position_at(self, now: Instant) -> ((f32, f32), bool) {
        let (position, _, _, complete) = self.position_velocity_at(now);
        (position, complete)
    }

    /// The horizontal axis as a shared 1-D [`ScrollMotion`].
    fn motion_x(self) -> ScrollMotion {
        ScrollMotion {
            start: self.start_x,
            target: self.target_x,
            started_at: self.started_at,
            duration: self.duration,
            initial_slope: self.initial_slope,
        }
    }

    /// The vertical axis as a shared 1-D [`ScrollMotion`].
    fn motion_y(self) -> ScrollMotion {
        ScrollMotion {
            start: self.start_y,
            target: self.target_y,
            started_at: self.started_at,
            duration: self.duration,
            initial_slope: self.initial_slope,
        }
    }

    /// Sample both axes by delegating to the shared
    /// [`crate::scroll_motion::ScrollMotion`] sampler, so container smooth
    /// scrolling and app-managed grid scrolling run one curve
    /// implementation. Both axes share `started_at`/`duration`, so either
    /// axis' completion flag describes the whole scroll.
    fn position_velocity_at(self, now: Instant) -> ((f32, f32), f32, f32, bool) {
        let (x, vx, complete) = self.motion_x().sample(now);
        let (y, vy, _) = self.motion_y().sample(now);
        ((x, y), vx, vy, complete)
    }
}

/// Opt-in return payload for element `Scroll` handlers (boxed as the
/// handler's `Box<dyn Any>` result): fresh grid content for the handling
/// node.
///
/// The wheel dispatch downcasts a consumed Scroll handler's return value to
/// this type. On a match it writes the grid straight into the retained arena
/// and paint-dirties the node instead of setting the global `needs_rebuild`,
/// so app-managed scroll surfaces (e.g. terminal scrollback) repaint at
/// content-change cost and a concurrently animating smooth scroll stays on
/// the fast-paint path. `grid: None` means "event consumed, nothing visible
/// changed" and schedules no work at all. Handlers returning any other value
/// (or `None`) keep the legacy behavior: a full rebuild on the next frame.
///
/// `animation` additionally registers (or replaces, extending the shared
/// waker's deadline) a framework-driven [`GridAnimationHook`] on the
/// handling node, so the surface keeps repainting at the animation cadence
/// after the event without owning a timer.
#[derive(Default)]
pub struct ScrollGridPatch {
    pub grid: Option<unshit_core::cell_grid::CellGrid>,
    /// Registers (or replaces) a framework-driven animation on the handling
    /// node. `None` leaves any in-flight animation untouched.
    pub animation: Option<GridAnimationHook>,
}

/// A node-scoped grid animation driven by the framework's animation tick.
///
/// The framework owns *when* to sample (the shared animation cadence, the
/// same one container smooth scrolling uses, so a future vblank-anchored
/// clock upgrades both at once); the app owns *what* a sample means: `tick`
/// receives the frame's injected timestamp and returns fresh grid content
/// or [`GridTick::Idle`]. Hooks are keyed by node — re-registering on the
/// same node replaces the previous hook, which is exactly what a new wheel
/// notch retargeting an in-flight animation wants.
pub struct GridAnimationHook {
    /// Drop-dead time for the hook (animation end plus grace). Past this
    /// instant the framework gives the sampler one final settling tick
    /// (so the underlying animation can land on its target rather than
    /// strand mid-flight) and then unregisters it unconditionally, so a
    /// buggy or stuck sampler cannot keep the cadence hot forever.
    pub deadline: Instant,
    /// Timestamp-injectable sampler, called once per animation frame on
    /// the UI thread. Must be cheap and must not re-enter the framework.
    pub tick: Arc<dyn Fn(Instant) -> GridTick + Send + Sync>,
}

/// One animation-frame step of a [`GridAnimationHook`].
pub enum GridTick {
    /// The animation advanced: repaint the node with this grid and keep
    /// the hook registered.
    Continue(unshit_core::cell_grid::CellGrid),
    /// Final frame: repaint the node with this grid and unregister the
    /// hook. Hooks whose underlying animation was cancelled out from
    /// under them (a snap-to-live, a takeover by another input device)
    /// should also return `Done` so the framework quiesces immediately
    /// rather than idling against the deadline.
    Done(unshit_core::cell_grid::CellGrid),
    /// Nothing changed visually this tick (e.g. the sampled position
    /// rounded to the same sub-row fraction); keep the hook registered
    /// and schedule no paint work.
    Idle,
}

/// Write a Scroll handler's freshly snapshotted grid onto the handling node,
/// mirroring the reconciler's `grid_content_paint_only` classification: a
/// same-dimensions content swap sets PAINT on the node plus SUBTREE_PAINT on
/// its ancestors and skips style and layout entirely. Returns `false` when
/// the node is gone, its content is not a grid, or the dimensions differ;
/// those cases can affect layout and must go through a full rebuild.
fn apply_scroll_grid_patch(
    arena: &mut NodeArena,
    node_id: NodeId,
    grid: unshit_core::cell_grid::CellGrid,
) -> bool {
    let dims_match = matches!(
        arena.get(node_id).map(|element| &element.content),
        Some(ElementContent::Grid(old)) if old.rows() == grid.rows() && old.cols() == grid.cols()
    );
    if !dims_match {
        return false;
    }
    if let Some(element) = arena.get_mut(node_id) {
        element.content = ElementContent::Grid(grid);
    }
    mark_node_paint_dirty(arena, node_id);
    true
}

fn wheel_scroll_delta_pixels(
    delta: winit::event::MouseScrollDelta,
    scale_factor: f32,
    zoom_factor: f32,
    tuning: ScrollTuning,
    chars_per_notch: f32,
    lines_per_notch: f32,
) -> (f32, f32, bool) {
    match delta {
        winit::event::MouseScrollDelta::LineDelta(x, y) => {
            let line_px = tuning.sanitized().line_scroll_px * scale_factor * zoom_factor;
            (
                normalize_wheel_line_delta(x, chars_per_notch) * line_px,
                normalize_wheel_line_delta(y, lines_per_notch) * line_px,
                true,
            )
        }
        winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32, false),
    }
}

/// Converts a winit `LineDelta` value into wheel notches by dividing
/// unconditionally by the platform's per-axis units-per-notch (lines for the
/// vertical axis, chars for the horizontal axis), so a full detent maps to
/// 1.0 and sub-notch fragments from high-resolution wheels scale
/// proportionally instead of being amplified. Non-finite or non-positive
/// divisors fall back to `WHEEL_LINE_DELTA_PER_NOTCH`.
fn normalize_wheel_line_delta(value: f32, units_per_notch: f32) -> f32 {
    let units = if units_per_notch.is_finite() && units_per_notch > 0.0 {
        units_per_notch
    } else {
        WHEEL_LINE_DELTA_PER_NOTCH
    };
    value / units
}

/// Lines-per-notch the OS applies to a single vertical wheel detent,
/// re-queried on every event.
///
/// Windows scales `WM_MOUSEWHEEL` into `LineDelta` lines using the user's
/// "wheel scroll lines" setting (`SPI_GETWHEELSCROLLLINES`, default 3), so
/// one detent arrives as e.g. `LineDelta(0.0, 3.0)`. The vendored winit
/// backend re-queries this setting on every `WM_MOUSEWHEEL`, so the divisor
/// must be re-queried per event too or a mid-session settings change would
/// desync multiplier and divisor. Falls back to `WHEEL_LINE_DELTA_PER_NOTCH`
/// when the query fails, reports 0, or reports the `WHEEL_PAGESCROLL`
/// page-scroll sentinel. Non-Windows winit backends already deliver ~1 line
/// per notch.
#[cfg(windows)]
fn lines_per_notch() -> f32 {
    use winapi::um::winuser::{SystemParametersInfoW, SPI_GETWHEELSCROLLLINES, WHEEL_PAGESCROLL};
    let mut lines: u32 = 0;
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&mut lines as *mut u32).cast(), 0)
    };
    if ok == 0 || lines == 0 || lines == WHEEL_PAGESCROLL {
        WHEEL_LINE_DELTA_PER_NOTCH
    } else {
        lines as f32
    }
}

#[cfg(not(windows))]
fn lines_per_notch() -> f32 {
    1.0
}

/// Chars-per-notch the OS applies to a single horizontal wheel detent,
/// re-queried on every event.
///
/// Windows scales `WM_MOUSEHWHEEL` into horizontal `LineDelta` values using
/// the "wheel scroll chars" setting (`SPI_GETWHEELSCROLLCHARS`, default 3),
/// not the lines setting, so the horizontal axis needs its own divisor.
/// Same fallbacks as [`lines_per_notch`]: query failure, 0, or the
/// `WHEEL_PAGESCROLL` sentinel map to `WHEEL_LINE_DELTA_PER_NOTCH`.
/// Non-Windows winit backends already deliver ~1 unit per notch.
#[cfg(windows)]
fn chars_per_notch() -> f32 {
    use winapi::um::winuser::{SystemParametersInfoW, SPI_GETWHEELSCROLLCHARS, WHEEL_PAGESCROLL};
    let mut chars: u32 = 0;
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWHEELSCROLLCHARS, 0, (&mut chars as *mut u32).cast(), 0)
    };
    if ok == 0 || chars == 0 || chars == WHEEL_PAGESCROLL {
        WHEEL_LINE_DELTA_PER_NOTCH
    } else {
        chars as f32
    }
}

#[cfg(not(windows))]
fn chars_per_notch() -> f32 {
    1.0
}

fn unscaled_scroll_delta(delta: (f32, f32), scale_factor: f32, zoom_factor: f32) -> (f32, f32) {
    let factor = (scale_factor * zoom_factor).max(0.01);
    (delta.0 / factor, delta.1 / factor)
}

fn next_smooth_scroll(
    current: (f32, f32),
    max_scroll: (f32, f32),
    active: Option<SmoothScroll>,
    node_id: NodeId,
    delta: (f32, f32),
    now: Instant,
    duration: Duration,
    initial_slope: f32,
) -> Option<SmoothScroll> {
    let base = active
        .filter(|scroll| scroll.node_id == node_id)
        .map(|scroll| (scroll.target_x, scroll.target_y))
        .unwrap_or(current);
    let target_x = (base.0 - delta.0).clamp(0.0, max_scroll.0);
    let target_y = (base.1 - delta.1).clamp(0.0, max_scroll.1);

    if (target_x - current.0).abs() < SMOOTH_SCROLL_EPSILON
        && (target_y - current.1).abs() < SMOOTH_SCROLL_EPSILON
    {
        return None;
    }

    let continuity_slope = active.filter(|scroll| scroll.node_id == node_id).and_then(|scroll| {
        let (_, vx, vy, complete) = scroll.position_velocity_at(now);
        if complete || duration.is_zero() {
            return None;
        }
        let velocity = dominant_delta((vx, vy));
        let new_delta = dominant_delta((target_x - current.0, target_y - current.1));
        if new_delta.abs() < SMOOTH_SCROLL_EPSILON {
            None
        } else {
            Some(velocity * duration.as_secs_f32() / new_delta)
        }
    });
    let initial_slope =
        continuity_slope.map(|slope| slope.max(initial_slope)).unwrap_or(initial_slope);

    Some(SmoothScroll {
        node_id,
        start_x: current.0,
        start_y: current.1,
        target_x,
        target_y,
        started_at: now,
        duration,
        initial_slope,
    })
}

fn scroll_telemetry(
    scroll: SmoothScroll,
    phase: ScrollTelemetryPhase,
    now: Instant,
) -> ScrollTelemetry {
    let ((scroll_x, scroll_y), velocity_x, velocity_y, _) = scroll.position_velocity_at(now);
    let elapsed_ms = now.saturating_duration_since(scroll.started_at).as_secs_f32() * 1000.0;
    let distance_y = scroll.target_y - scroll.start_y;
    let progress_y = if distance_y.abs() < SMOOTH_SCROLL_EPSILON {
        1.0
    } else {
        ((scroll_y - scroll.start_y) / distance_y).clamp(0.0, 1.0)
    };

    ScrollTelemetry {
        phase,
        node_id: scroll.node_id,
        elapsed_ms,
        duration_ms: scroll.duration.as_secs_f32() * 1000.0,
        start_x: scroll.start_x,
        start_y: scroll.start_y,
        scroll_x,
        scroll_y,
        target_x: scroll.target_x,
        target_y: scroll.target_y,
        velocity_x,
        velocity_y,
        progress_y,
    }
}

fn emit_scroll_telemetry(callback: Option<&ScrollTelemetryCallback>, telemetry: ScrollTelemetry) {
    if let Some(callback) = callback {
        callback(&telemetry);
    }
}

fn tick_smooth_scroll(
    state: &mut AppState,
    now: Instant,
    decorations: bool,
    on_scroll_telemetry: Option<&ScrollTelemetryCallback>,
) {
    let Some(scroll_state) = state.smooth_scroll else {
        return;
    };

    if state.arena.get(scroll_state.node_id).is_none() {
        state.smooth_scroll = None;
        return;
    }

    let ((next_x, next_y), complete) = scroll_state.position_at(now);
    scroll::set_scroll_position(&mut state.arena, scroll_state.node_id, next_x, next_y);
    emit_scroll_telemetry(
        on_scroll_telemetry,
        scroll_telemetry(
            scroll_state,
            if complete { ScrollTelemetryPhase::Completed } else { ScrollTelemetryPhase::Frame },
            now,
        ),
    );

    if complete {
        let pos = state.interaction.last_cursor_pos;
        handle_normal_hover(state, pos, decorations);
        state.smooth_scroll = None;
        // The frame gate is shared with grid animations; only drop it when
        // no animation source remains.
        if state.grid_animations.is_empty() {
            state.animation_next_frame = None;
        }
    }
}

/// Advance every registered grid animation by one tick, apply the fresh
/// grids as paint-only patches, and unregister hooks that reported `Done`
/// or whose deadline passed.
///
/// `apply_patches` is `false` on frames that are about to run a full tree
/// rebuild: the rebuilt snapshot reflects the advanced animation state
/// anyway, so applying the patch would only duplicate compose work. Hook
/// lifecycle (deadline expiry, `Done` unregistration) still advances.
///
/// Returns whether any hook reported a visual change. Kept as a thin
/// wrapper over [`tick_grid_animations_core`] so the lifecycle rules are
/// unit-testable without an [`AppState`].
fn tick_grid_animations(state: &mut AppState, now: Instant, apply_patches: bool) -> bool {
    if state.grid_animations.is_empty() {
        return false;
    }
    let (visual_change, rebuild_required) =
        tick_grid_animations_core(&mut state.arena, &mut state.grid_animations, now, apply_patches);
    if rebuild_required {
        state.needs_rebuild = true;
    }
    if state.smooth_scroll.is_none() && state.grid_animations.is_empty() {
        state.animation_next_frame = None;
    }
    visual_change
}

/// Core of [`tick_grid_animations`]: returns
/// `(visual_change, rebuild_required)`.
fn tick_grid_animations_core(
    arena: &mut NodeArena,
    grid_animations: &mut HashMap<NodeId, GridAnimationHook>,
    now: Instant,
    apply_patches: bool,
) -> (bool, bool) {
    let mut visual_change = false;
    let mut rebuild_required = false;
    let mut finished: Vec<NodeId> = Vec::new();
    // Snapshot the hooks before ticking: the samplers run app code and the
    // patch application needs the arena, so the map cannot stay borrowed.
    #[allow(clippy::type_complexity)]
    let ticks: Vec<(NodeId, Instant, Arc<dyn Fn(Instant) -> GridTick + Send + Sync>)> =
        grid_animations
            .iter()
            .map(|(node_id, hook)| (*node_id, hook.deadline, Arc::clone(&hook.tick)))
            .collect();
    for (node_id, deadline, tick) in ticks {
        let node_alive = arena.get(node_id).is_some();
        if now > deadline {
            // Deadline expiry (a frame gap longer than the grace at the
            // motion's end, or a sampler that never reported `Done`).
            // The hook still gets one final sample so the app-side
            // animation settles on its target instead of stranding
            // mid-flight with the last sampled position on screen; the
            // settled frame is painted when the node still exists.
            if let GridTick::Continue(grid) | GridTick::Done(grid) = tick(now) {
                if node_alive {
                    visual_change = true;
                    if apply_patches && !apply_scroll_grid_patch(arena, node_id, grid) {
                        rebuild_required = true;
                    }
                }
            }
            finished.push(node_id);
            continue;
        }
        let (grid, done) = match tick(now) {
            GridTick::Idle => continue,
            GridTick::Continue(grid) => (grid, false),
            GridTick::Done(grid) => (grid, true),
        };
        if done {
            finished.push(node_id);
        }
        if node_alive {
            visual_change = true;
            if apply_patches && !apply_scroll_grid_patch(arena, node_id, grid) {
                // The grid's dimensions changed underneath the animation
                // (or the node no longer holds grid content): fall back to
                // a full rebuild. Live hooks stay registered so the
                // animation resumes against the rebuilt tree.
                rebuild_required = true;
            }
        } else {
            // The node vanished mid-flight (reconcile re-keyed or removed
            // it, e.g. a tab switch while a wheel animation runs). The
            // hook keeps ticking blind so the app-side animation advances
            // to completion instead of stranding, and each advanced
            // sample requests a rebuild: if the surface is (or becomes)
            // visible again under a fresh node, the rebuilt snapshot
            // carries the up-to-date position. The deadline still bounds
            // the hook's lifetime.
            rebuild_required = true;
        }
    }
    for node_id in finished {
        grid_animations.remove(&node_id);
    }
    (visual_change, rebuild_required)
}

/// Whether any framework-driven animation is in flight: a container smooth
/// scroll or at least one registered grid-animation hook. Every tick-site
/// and control-flow predicate that used to test `smooth_scroll.is_some()`
/// goes through this so both animation kinds share the fast-paint path,
/// the pacer bypass, and the wake cadence.
fn animations_active(state: &AppState) -> bool {
    state.smooth_scroll.is_some() || !state.grid_animations.is_empty()
}

fn can_fast_paint_animations(state: &AppState) -> bool {
    animations_active(state)
        && !state.needs_rebuild
        && !state.needs_restyle
        && !state.needs_relayout
}

/// Start (or retarget) the animation frame chain after a producer
/// registered new animation work (a wheel notch starting a container
/// smooth scroll, a Scroll handler registering a grid hook).
///
/// `VsyncBlocking`: a single `request_redraw` is all it takes — each paint
/// re-requests the next while any animation remains, and the blocking
/// acquire paces the chain at the refresh rate. The wake deadline is
/// ignored; hook settling / `Done` ends the chain.
///
/// `Timer`: the legacy machinery — extend the shared waker through
/// `wake_deadline` (animation end plus grace) and open the due-gate so the
/// first frame paints immediately; mid-flight retargets keep the existing
/// gate (at most one period away) so wheel storms never paint at event
/// rate.
fn kick_animation(state: &mut AppState, waker: &AnimationWaker, wake_deadline: Instant) {
    match state.pacing_mode {
        PacingMode::VsyncBlocking => {
            state.window.request_redraw();
        }
        PacingMode::Timer => {
            waker.extend_until(wake_deadline);
            if state.animation_next_frame.is_none() {
                state.animation_next_frame = Some(Instant::now());
            }
            state.window.request_redraw();
        }
    }
}

/// Post-paint continuation for a `RedrawRequested` frame that advanced
/// animations. Keeps the chain alive while any animation remains, the way
/// the active pacing mode wants; sets plain `Wait` once everything
/// settled.
///
/// `painted` is false when the animation tick produced no visual change
/// and nothing was presented (see [`fast_paint_animation_frame`]); the
/// slow path always presents and passes true.
fn schedule_animation_followup(
    state: &mut AppState,
    event_loop: &dyn ActiveEventLoop,
    frame_start: Instant,
    painted: bool,
) {
    if !animations_active(state) {
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        return;
    }
    match state.pacing_mode {
        PacingMode::VsyncBlocking => {
            if painted {
                // The classic vsync loop: request the next frame now;
                // its blocking acquire paces the chain at the refresh
                // rate, so no timer is involved.
                state.window.request_redraw();
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            } else {
                // Nothing was presented, so no acquire will block and a
                // chained redraw would tick at CPU speed. Park until the
                // next display period; the stale-deadline branch in
                // `about_to_wait` re-issues the redraw when the timer
                // fires. Blind-ticking hooks therefore idle at display
                // cadence until their deadline settles them.
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                    skipped_frame_resume_at(
                        frame_start,
                        display_period_ns_from_mhz(state.display_refresh_mhz),
                    ),
                ));
            }
        }
        PacingMode::Timer => {
            // True-period due-gate. The redraw is deliberately NOT
            // requested here: a queued internal paint wakes the OS wait
            // immediately, so `about_to_wait` issues it once the gate
            // opens.
            let due = frame_start + state.frame_pacer.min_interval();
            state.animation_next_frame = Some(due);
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
        }
    }
}

/// Returns whether the frame painted (presented). A `false` return means
/// no hook produced a visual change and nothing reached the GPU, so no
/// acquire blocked: the caller must schedule the animation continuation
/// itself instead of chaining a redraw off a present that never happened.
fn fast_paint_animation_frame(
    state: &mut AppState,
    frame_start: Instant,
    decorations: bool,
    on_scroll_telemetry: Option<&ScrollTelemetryCallback>,
    on_frame_metrics: Option<&(dyn Fn(&FrameMetrics) + Send)>,
) -> bool {
    let mut metrics = FrameMetrics::default();

    // Animation positions are sampled at the predicted present time of
    // this frame, not at wake time, so scheduler jitter does not become
    // velocity noise. Everything else (metrics, pacer bookkeeping) keeps
    // using `frame_start`.
    let sample_ts = predicted_present_ts(
        state.last_paint_completed_at,
        display_period_ns_from_mhz(state.display_refresh_mhz),
        frame_start,
    );

    // A container smooth scroll repositions on every sample (including its
    // final pin-to-target frame), so its mere presence at entry means this
    // frame changes pixels. Grid hooks report per-tick whether anything
    // moved; when every hook is idle (e.g. the sampled position rounded to
    // the same device pixel) and no canvas wants a repaint, skip the batch
    // build and the GPU present entirely — the previous frame is still
    // correct. Lifecycle (deadline expiry, `Done`) advanced above.
    let container_scroll_active = state.smooth_scroll.is_some();
    tick_smooth_scroll(state, sample_ts, decorations, on_scroll_telemetry);
    let grid_visual_change = tick_grid_animations(state, sample_ts, true);
    if !container_scroll_active && !grid_visual_change && !state.gpu.any_canvas_needs_repaint() {
        return false;
    }

    state.gpu.glyph_atlas.advance_frame();

    let t4 = Instant::now();
    state.gpu.layered_batch.clear();
    state.batch_cache.begin_frame();
    {
        let mut rasterizer = Rasterizer {
            swash: &mut state.swash_cache,
            subpixel_swash: &mut state.subpixel_swash_cache,
            #[cfg(target_os = "windows")]
            dw: &state.dw_rasterizer,
        };
        batch::build_render_batch(
            &state.arena,
            state.root,
            &mut state.gpu.layered_batch,
            &mut state.gpu.glyph_atlas,
            &mut state.font_system,
            &mut rasterizer,
            &mut state.measure_cache,
            &mut state.shaped_cache,
            &mut state.gpu.svg_cache,
            &mut state.shape_cache,
            state.interaction.text_selection.as_ref(),
            Some(&state.canvas_registry),
            &state.scrollbar_visual,
            state.interaction.focused,
            &mut state.batch_cache,
            Some(&mut state.line_quad_cache),
        );
    }
    state.batch_cache.commit_frame();
    state.shaped_cache.finish_frame(state.gpu.glyph_atlas.generation);
    state.shape_cache.finish_frame();
    batch::clear_paint_flags_subtree(&mut state.arena, state.root);
    metrics.batch_build_us = t4.elapsed().as_micros() as u64;
    metrics.node_count = state.arena.len();

    {
        let mut total_quads: u32 = 0;
        let mut total_glyphs: u32 = 0;
        for layer in &state.gpu.layered_batch.layers {
            total_quads = total_quads.saturating_add(layer.quad_instances.len() as u32);
            total_glyphs = total_glyphs.saturating_add(layer.glyph_instances.len() as u32);
        }
        metrics.quad_count = total_quads;
        metrics.glyph_count = total_glyphs;
    }

    metrics.atlas_fill_ratio = if state.gpu.glyph_atlas.size > 0 {
        state.gpu.glyph_atlas.next_shelf_y as f32 / state.gpu.glyph_atlas.size as f32
    } else {
        0.0
    };
    metrics.gpu_upload_bytes =
        state.gpu.glyph_atlas.pending_uploads.iter().map(|g| (g.width * g.height) as u64).sum();

    let t5 = Instant::now();
    state.window.pre_present_notify();
    state.gpu.render();
    // Split the vblank wait out of the work numbers at the source so
    // every downstream consumer of gpu_render_us / total_us keeps
    // measuring CPU work (see FrameMetrics::present_wait_us).
    metrics.present_wait_us = state.gpu.last_acquire_wait().as_micros() as u64;
    metrics.gpu_render_us =
        (t5.elapsed().as_micros() as u64).saturating_sub(metrics.present_wait_us);

    if state.gpu.any_canvas_needs_repaint() {
        state.window.request_redraw();
    }

    finalize_frame_metrics(state, metrics, frame_start, on_frame_metrics);
    true
}

/// Pure function: active display refresh period in nanoseconds from a
/// refresh rate in millihertz. 0 mHz (rate unknown) maps to a 0ns period
/// so consumers can detect "unknown" and fall back to
/// [`FrameMetrics::pacer_min_interval_ns`]. Extracted so the mapping is
/// unit-testable without constructing an [`AppState`].
pub(crate) fn display_period_ns_from_mhz(mhz: u32) -> u64 {
    if mhz == 0 {
        0
    } else {
        1_000_000_000_000u64 / mhz as u64
    }
}

/// Pure function: wall-clock microseconds between the previous paint's
/// completion and `now`. 0 when there is no previous paint (first frame)
/// or when the clock would run backwards. Extracted so the interval
/// bookkeeping is unit-testable with synthetic instants without
/// constructing an [`AppState`].
pub(crate) fn present_interval_us(last_paint_completed_at: Option<Instant>, now: Instant) -> u64 {
    last_paint_completed_at
        .map(|prev| now.saturating_duration_since(prev).as_micros() as u64)
        .unwrap_or(0)
}

/// Pure function: classify a raw paint-to-paint gap as a measurable
/// cadence interval or an idle cadence break. Gaps of [`ACTIVITY_WINDOW`]
/// or longer mean the event loop stopped its speculative repaint rhythm
/// between the two paints (the app went idle by design; the 500ms
/// cursor-blink wake of an idle session is the canonical producer), so
/// they describe intentional idling rather than missed refreshes and are
/// mapped to 0, the same sentinel consumers already skip for the first
/// painted frame. Without this, an idle session's blink repaints would
/// saturate every interval quantile and dropped-frame counter with
/// ~500ms "intervals" that no refresh was ever missed for.
pub(crate) fn cadence_present_interval_us(raw_interval_us: u64) -> u64 {
    if raw_interval_us >= ACTIVITY_WINDOW.as_micros() as u64 {
        0
    } else {
        raw_interval_us
    }
}

/// Shared per-paint epilogue for both paint paths (the smooth-scroll
/// fast path and the full `RedrawRequested` pipeline). Records the paint
/// in the pacer, finalizes the frame's metrics (total time, rss, pacer
/// interval, presentation cadence), feeds both frame probes and emits
/// their once-per-second summaries, logs slow frames, fires the
/// `on_frame_metrics` callback, stores the metrics, and rolls the
/// window-title fps counter.
fn finalize_frame_metrics(
    state: &mut AppState,
    mut metrics: FrameMetrics,
    frame_start: Instant,
    on_frame_metrics: Option<&(dyn Fn(&FrameMetrics) + Send)>,
) {
    state.frame_pacer.record_paint(frame_start);
    // total_us is CPU work: the swapchain acquire wait that happened
    // inside gpu.render() (up to one display period under Fifo) is
    // subtracted so vsync pacing does not masquerade as a slow frame.
    metrics.total_us =
        (frame_start.elapsed().as_micros() as u64).saturating_sub(metrics.present_wait_us);
    metrics.rss_bytes = get_rss_bytes();
    metrics.pacer_min_interval_ns = state.frame_pacer.min_interval().as_nanos() as u64;

    // Presentation cadence: one completion timestamp per painted frame,
    // taken right after total_us is finalized. The interval back to the
    // previous completion is the honest paint-to-paint period the user
    // experiences, unlike total_us which only measures CPU work. Idle
    // cadence breaks are zeroed (see cadence_present_interval_us) so
    // blink-tick repaints do not masquerade as missed refreshes.
    let now = Instant::now();
    metrics.present_interval_us =
        cadence_present_interval_us(present_interval_us(state.last_paint_completed_at, now));
    state.last_paint_completed_at = Some(now);
    metrics.display_period_ns = display_period_ns_from_mhz(state.display_refresh_mhz);

    // Per-second frame-time probes. Always record into the rolling
    // windows so the in-app FPS overlay can read live quantiles without
    // rebuilding; `maybe_emit` only returns a summary when the runtime
    // enable flag is set (default on in debug, off in release). See
    // crate::frame_probe.
    state.frame_probe.record_frame(std::time::Duration::from_micros(metrics.total_us));
    if let Some(snap) = state.frame_probe.maybe_emit(now) {
        log::info!("[FRAME] {}", snap);
    }
    if metrics.present_interval_us > 0 {
        state
            .interval_probe
            .record_frame(std::time::Duration::from_micros(metrics.present_interval_us));
    }
    if let Some(snap) = state.interval_probe.maybe_emit(now) {
        // Interval counts get their own field label so log consumers can
        // tell cadence quantiles from the work-time `frames=` lines, and
        // a dropped count (intervals > 1.5x the display period, pacer
        // fallback) per the Phase 1 honest-metrics spec.
        let period_us = if metrics.display_period_ns > 0 {
            metrics.display_period_ns / 1_000
        } else {
            metrics.pacer_min_interval_ns / 1_000
        };
        let dropped = if period_us > 0 {
            state.interval_probe.count_above_us(period_us + period_us / 2)
        } else {
            0
        };
        log::info!(
            "[FRAME-INTERVAL] intervals={} min={:.2}ms p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms dropped={}",
            snap.count,
            snap.min_us as f64 / 1000.0,
            snap.p50_us as f64 / 1000.0,
            snap.p95_us as f64 / 1000.0,
            snap.p99_us as f64 / 1000.0,
            snap.max_us as f64 / 1000.0,
            dropped,
        );
    }

    // Log slow frames.
    if metrics.total_us > 8333 {
        log::warn!("[PERF] {}", metrics);
    } else {
        log::debug!("[PERF] {}", metrics);
    }

    if let Some(cb) = on_frame_metrics {
        cb(&metrics);
    }

    state.last_metrics = metrics;

    roll_over_window_title_fps(state);
}

/// Advance the once-per-second window-title fps counter. Runs on every
/// painted frame from both paint paths so the rollover cadence (and the
/// fps the title reports) is identical regardless of which path painted
/// the frame. The title text itself still updates at most once per
/// second.
fn roll_over_window_title_fps(state: &mut AppState) {
    state.frame_count += 1;
    let fps_elapsed = state.fps_timer.elapsed();
    if fps_elapsed.as_millis() >= 1000 {
        // Cheap pull-based refresh-rate re-probe (spec item 5): winit's
        // win32 backend surfaces no display-mode-change event, so an
        // in-place Hz change in Windows settings would otherwise leave
        // the pacer interval and display_period_ns stale forever. One
        // monitor query per second, only while frames are painting.
        refresh_pacer_from_window(state);
        state.current_fps = state.frame_count as f32 / fps_elapsed.as_secs_f32();
        let title = format!(
            "{} | {:.1}ms | {:.0} fps | rss {:.0}MB | nodes {}",
            state.app_title,
            state.last_metrics.total_us as f64 / 1000.0,
            state.current_fps,
            state.last_metrics.rss_bytes as f64 / (1024.0 * 1024.0),
            state.last_metrics.node_count,
        );
        state.window.set_title(&title);
        state.frame_count = 0;
        state.fps_timer = Instant::now();
    }
}

/// Pure function: whether `now` is within [`ACTIVITY_WINDOW`] of the
/// recorded `last_activity`. Extracted so it can be unit-tested with a
/// synthetic clock without constructing an entire [`AppState`].
pub(crate) fn is_within_activity_window(
    last_activity: Instant,
    now: Instant,
    window: Duration,
) -> bool {
    now.saturating_duration_since(last_activity) < window
}

fn cascade_root_for_restyle(
    restyle_root: Option<NodeId>,
    document_root: NodeId,
    scale_factor: f32,
) -> NodeId {
    if (scale_factor - 1.0).abs() >= 0.001 {
        document_root
    } else {
        restyle_root.unwrap_or(document_root)
    }
}

fn subtree_has_dirty_flags(arena: &NodeArena, node_id: NodeId, flags: DirtyFlags) -> bool {
    let Some(element) = arena.get(node_id) else {
        return false;
    };
    if element.dirty.intersects(flags) {
        return true;
    }
    let mut child = element.first_child;
    while !child.is_dangling() {
        if subtree_has_dirty_flags(arena, child, flags) {
            return true;
        }
        child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
    }
    false
}

fn mark_full_restyle_required(arena: &mut NodeArena, root: NodeId) {
    if let Some(element) = arena.get_mut(root) {
        element.dirty.insert(DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
    }
}

fn pseudo_restyle_root_for_change(
    arena: &NodeArena,
    old: NodeId,
    new: NodeId,
    root: NodeId,
) -> NodeId {
    let old_in = !old.is_dangling() && arena.get(old).is_some();
    let new_in = !new.is_dangling() && arena.get(new).is_some();
    if !old_in || !new_in {
        return root;
    }
    arena.lowest_common_ancestor(old, new, root)
}

/// Coalesces external events that arrive between two paints into a
/// single per-frame decision. The contract that callers depend on:
/// any number of [`ExternalEvent::RequestRebuild`] events that land in
/// one drain window collapse to exactly one tree rebuild on the next
/// paint. The renderer reads `needs_rebuild` once per frame, runs the
/// rebuild pipeline if set, then resets the flag, so additional events
/// that arrive after the drain but before the redraw still piggyback on
/// the same rebuild as long as they land in the same drain window.
///
/// Kept as a small explicit value type (rather than an inline boolean
/// in `proxy_wake_up`) so the coalescing guarantee has a unit test that
/// does not need a full winit [`AppState`]. See
/// [`tests::request_rebuild_events_coalesce_to_single_rebuild`].
#[derive(Default, Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct RebuildCoalescer {
    /// Set to true the first time a rebuild-implying event lands. Stays
    /// true regardless of how many further rebuild events arrive in the
    /// same drain.
    pub needs_rebuild: bool,
    /// True if any event was observed during the current drain. Used
    /// independently of `needs_rebuild` to mark UI activity for the
    /// speculative-frame window.
    pub saw_event: bool,
    /// Telemetry: how many rebuild-implying events arrived during the
    /// drain. Always >= 1 when `needs_rebuild` is set; saturates at
    /// `u32::MAX` to avoid wrapping under pathological storms.
    pub rebuild_request_count: u32,
}

impl RebuildCoalescer {
    /// Reset to "no events yet for this drain". Called once per
    /// `proxy_wake_up` entry so the same struct can be reused across
    /// frames without per-frame allocation.
    pub(crate) fn begin_drain(&mut self) {
        self.needs_rebuild = false;
        self.saw_event = false;
        self.rebuild_request_count = 0;
    }

    /// Record one event. `request_rebuild` is true when the event
    /// semantically implies a tree rebuild ([`ExternalEvent::RequestRebuild`],
    /// [`ExternalEvent::Custom`], hot-reloaded stylesheet). All other
    /// variants only mark activity.
    pub(crate) fn observe(&mut self, request_rebuild: bool) {
        self.saw_event = true;
        if request_rebuild {
            self.needs_rebuild = true;
            self.rebuild_request_count = self.rebuild_request_count.saturating_add(1);
        }
    }
}

/// Adapter that exposes a winit [`Window`] as a
/// [`crate::frame_pacer::MonitorRefreshSource`]. The adapter is the sole
/// point in the framework that talks to winit's monitor APIs; the pacer
/// logic and tests depend only on the trait.
struct WindowRefreshSource<'a>(&'a dyn Window);

impl<'a> crate::frame_pacer::MonitorRefreshSource for WindowRefreshSource<'a> {
    fn current_refresh_mhz(&self) -> Option<u32> {
        self.0
            .current_monitor()
            .or_else(|| self.0.primary_monitor())
            .and_then(|m| m.current_video_mode())
            .and_then(|v| v.refresh_rate_millihertz())
            .map(|nz| nz.get())
    }
}

/// Update the pacer's coalescing interval from the window's current
/// monitor, if it can be determined. Called on window creation, after
/// scale-factor changes inside surface metric reconciliation, and from
/// [`WindowEvent::Moved`]. Silently falls back to
/// [`crate::frame_pacer::FramePacer::DEFAULT_MIN_INTERVAL`] when the
/// platform cannot enumerate the monitor's refresh rate (headless / some
/// Wayland configs).
fn refresh_pacer_from_window(state: &mut AppState) {
    use crate::frame_pacer::MonitorRefreshSource as _;
    let mhz = WindowRefreshSource(&*state.window).current_refresh_mhz().unwrap_or(0);
    let before = state.frame_pacer.min_interval();
    state.frame_pacer.set_refresh_rate_mhz(mhz);
    state.display_refresh_mhz = mhz;
    state.last_refresh_probe = Instant::now();
    let after = state.frame_pacer.min_interval();
    if after != before {
        log::info!("pacer coalescing interval: {:.3}ms", after.as_secs_f64() * 1000.0);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceMetricsChange {
    None,
    Relayout,
    Rebuild,
}

fn classify_surface_metrics_change(
    current_size: (f32, f32),
    current_scale: f32,
    new_size: winit::dpi::PhysicalSize<u32>,
    new_scale: f32,
) -> SurfaceMetricsChange {
    let size_changed =
        (current_size.0 as u32, current_size.1 as u32) != (new_size.width, new_size.height);
    let has_size = new_size.width > 0 && new_size.height > 0;
    let scale_changed = (new_scale - current_scale).abs() > 0.01;

    if scale_changed {
        SurfaceMetricsChange::Rebuild
    } else if !has_size {
        SurfaceMetricsChange::None
    } else if size_changed {
        SurfaceMetricsChange::Relayout
    } else {
        SurfaceMetricsChange::None
    }
}

fn reconcile_surface_metrics(
    state: &mut AppState,
    new_size: winit::dpi::PhysicalSize<u32>,
    new_scale: f32,
    on_scale_factor: Option<&Arc<dyn Fn(f32) + Send + Sync>>,
) -> SurfaceMetricsChange {
    let current_size = state.gpu.window_size();
    let change =
        classify_surface_metrics_change(current_size, state.scale_factor, new_size, new_scale);

    if new_size.width > 0 && new_size.height > 0 {
        if (current_size.0 as u32) != new_size.width || (current_size.1 as u32) != new_size.height {
            state.gpu.resize(new_size);
        }
    }

    match change {
        SurfaceMetricsChange::Rebuild => {
            log::info!("Scale factor changed: {:.2}x -> {:.2}x", state.scale_factor, new_scale);
            state.scale_factor = new_scale;
            if let Some(cb) = on_scale_factor {
                cb(new_scale);
            }
            refresh_pacer_from_window(state);
            mark_full_restyle_required(&mut state.arena, state.root);
            state.needs_rebuild = true;
        }
        SurfaceMetricsChange::Relayout => {
            state.needs_relayout = true;
        }
        SurfaceMetricsChange::None => {}
    }

    change
}

fn reconcile_surface_metrics_from_window(
    state: &mut AppState,
    on_scale_factor: Option<&Arc<dyn Fn(f32) + Send + Sync>>,
) -> SurfaceMetricsChange {
    reconcile_surface_metrics(
        state,
        state.window.surface_size(),
        state.window.scale_factor() as f32,
        on_scale_factor,
    )
}

fn publish_window_maximized_change(
    last: &mut bool,
    current: bool,
    on_window_maximized: Option<&Arc<dyn Fn(bool) + Send + Sync>>,
) -> bool {
    if *last == current {
        return false;
    }

    *last = current;
    if let Some(cb) = on_window_maximized {
        cb(current);
    }
    true
}

fn sync_window_maximized_from_window(
    state: &mut AppState,
    on_window_maximized: Option<&Arc<dyn Fn(bool) + Send + Sync>>,
) -> bool {
    publish_window_maximized_change(
        &mut state.window_maximized,
        state.window.is_maximized(),
        on_window_maximized,
    )
}

impl AppState {
    /// Record that external activity (user input, PTY output, resize, etc.)
    /// occurred at `now`. Opens the window during which `about_to_wait`
    /// will schedule speculative frames.
    fn mark_activity(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// Whether the last external event occurred within
    /// [`ACTIVITY_WINDOW`] of `now`. When true, `about_to_wait`
    /// schedules a speculative repaint at the pacer deadline so painting
    /// runs at the pacer rhythm even when no dirty flag is set.
    fn is_recently_active(&self, now: Instant) -> bool {
        is_within_activity_window(self.last_activity, now, ACTIVITY_WINDOW)
    }

    /// Whether enough time has elapsed since the last monitor refresh
    /// probe to justify re-querying the compositor. Winit
    /// `WindowEvent::Moved` fires on every drag delta; without this gate
    /// we would hit `current_monitor()` once per pixel of motion.
    fn should_reprobe_refresh(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_refresh_probe) >= ACTIVITY_WINDOW
    }

    /// Mark the app as needing a restyle and narrow the next cascade
    /// to the lowest common ancestor of `old` and `new`.
    ///
    /// When called multiple times before the next paint (e.g. hover
    /// changes from A to B to C in one input drain), the scope widens
    /// to the LCA of every change so the eventual cascade is still
    /// correct. The narrowed root is reset to `None` (full tree) by the
    /// restyle pass via `take()`.
    fn mark_restyle_pseudo_change(&mut self, old: NodeId, new: NodeId) {
        self.needs_restyle = true;
        let candidate = pseudo_restyle_root_for_change(&self.arena, old, new, self.root);
        self.restyle_root = Some(match self.restyle_root {
            Some(prev) => self.arena.lowest_common_ancestor(prev, candidate, self.root),
            None => candidate,
        });
    }
}

impl App {
    pub fn new(config: AppConfig, tree_fn: impl Fn() -> ElementTree + 'static) -> Self {
        let (event_tx, event_rx) = flume::unbounded();
        let proxy_cell: Arc<OnceLock<EventLoopProxy>> = Arc::new(OnceLock::new());
        Self {
            config,
            tree_fn: Box::new(tree_fn),
            state: None,
            // Placeholder interval: the display's refresh rate is not
            // known until the window exists. `can_create_surfaces`
            // rebuilds the waker with the true period before any
            // animation can start (the tick thread spawns lazily, so
            // nothing is wasted).
            animation_waker: AnimationWaker::new(
                event_tx.clone(),
                Arc::clone(&proxy_cell),
                crate::frame_pacer::FramePacer::DEFAULT_MIN_INTERVAL,
            ),
            event_tx,
            event_rx,
            proxy_cell,
            canvas_registry: CanvasRegistry::new(),
            clipboard: Arc::new(ClipboardContext::new()),
            #[cfg(feature = "async")]
            runtime: crate::runtime::AsyncRuntime::new(),
            #[cfg(feature = "async")]
            subscription_manager: None,
        }
    }

    /// Returns an [`EventSink`] that can be moved into other threads to push
    /// events into the framework's event loop.
    ///
    /// May be called multiple times; each clone is independently usable.
    /// Must be called before [`run`](Self::run) to buffer events that arrive
    /// before the event loop starts.
    pub fn event_sink(&self) -> EventSink {
        EventSink::new(self.event_tx.clone(), Arc::clone(&self.proxy_cell))
    }

    /// Returns a shared reference to the clipboard context.
    ///
    /// The returned `Arc<ClipboardContext>` can be cloned and moved into
    /// other threads or handler closures.
    pub fn clipboard(&self) -> Arc<ClipboardContext> {
        Arc::clone(&self.clipboard)
    }

    /// Set the key combo that releases keyboard capture mode.
    ///
    /// Default: Ctrl+Shift+Escape. When a focused element has
    /// `captures_keyboard` enabled, pressing this chord will clear the
    /// capture flag instead of forwarding the key event.
    pub fn set_release_chord(&mut self, combo: unshit_core::shortcut::KeyCombo) {
        if let Some(ref mut state) = self.state {
            state.release_chord = combo;
        }
    }

    /// Configure the bell subsystem. Must be called before [`run`](Self::run)
    /// or early in the app lifecycle.
    pub fn set_bell_config(&mut self, config: BellConfig) {
        if let Some(ref mut state) = self.state {
            state.bell_state = BellState::new(config);
        }
    }

    /// Return a fresh snapshot of the input latency histograms.
    ///
    /// Returns `None` before the event loop starts (no [`AppState`] yet)
    /// or when the `input-latency-histogram` cargo feature is not
    /// compiled in. See [`crate::input_latency`] for the full
    /// instrument description.
    #[cfg(feature = "input-latency-histogram")]
    pub fn input_latency_snapshot(&self) -> Option<crate::input_latency::InputLatencySnapshot> {
        self.state.as_ref().map(|s| s.input_latency.snapshot())
    }

    /// Request window attention at the given urgency level.
    ///
    /// Maps directly to winit's `Window::request_attention`. On most
    /// platforms this flashes the taskbar entry or title bar.
    pub fn request_attention(&self, urgency: AttentionUrgency) {
        if let Some(ref state) = self.state {
            state.window.request_user_attention(Some(urgency.to_winit()));
        }
    }

    /// Fire a bell signal. Triggers both a brief visual overlay and a window
    /// attention request (informational), subject to the current
    /// [`BellConfig`]. Repeated calls within the rate-limit window are
    /// silently suppressed.
    pub fn bell(&mut self) {
        if let Some(ref mut state) = self.state {
            if state.bell_state.try_bell() {
                if state.bell_state.should_request_attention() {
                    state
                        .window
                        .request_user_attention(Some(AttentionUrgency::Informational.to_winit()));
                }
                // Visual bell overlay is rendered by the frame loop when
                // bell_state.visual_bell_active is true.
                state.window.request_redraw();
            }
        }
    }

    /// Send an OS-level notification with the given title and body.
    ///
    /// Requires the `notifications` Cargo feature. Without it, this is a
    /// no-op that compiles away.
    pub fn notify(&self, title: &str, body: &str) {
        crate::notification::send_os_notification(title, body);
    }

    /// Register a [`CustomPainter`] for a canvas element identified by `id`.
    ///
    /// The painter will be invoked each frame for any `<canvas>` element whose
    /// `id` attribute matches. Must be called before [`run`](Self::run).
    pub fn register_canvas(&mut self, id: impl Into<String>, painter: Arc<dyn CustomPainter>) {
        self.canvas_registry.register(id, painter);
    }

    /// Spawn a future on the background tokio runtime.
    ///
    /// The future runs on a tokio worker thread, not the UI thread.
    /// Use [`EventSink`] from inside the future to push results back.
    ///
    /// Requires the `async` feature.
    #[cfg(feature = "async")]
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(future)
    }

    /// Returns a reference to the background tokio runtime handle.
    ///
    /// Useful for code that needs to interact with the runtime directly
    /// (e.g. `block_on` from a non-async context).
    ///
    /// Requires the `async` feature.
    #[cfg(feature = "async")]
    pub fn runtime_handle(&self) -> &tokio::runtime::Handle {
        self.runtime.handle()
    }

    /// Set a function that returns the currently active subscriptions.
    ///
    /// Called after each tree rebuild; the framework starts new subscriptions
    /// and cancels removed ones based on identity. Each subscription produces
    /// a stream of [`ExternalEvent`]s polled on the background tokio runtime.
    ///
    /// Requires the `async` feature.
    #[cfg(feature = "async")]
    pub fn set_subscriptions(
        &mut self,
        f: impl Fn() -> Vec<crate::subscription::Subscription> + Send + 'static,
    ) {
        self.subscription_manager =
            Some(crate::subscription::SubscriptionManager::new(Box::new(f)));
    }

    pub fn run(self) {
        let event_loop = EventLoop::new().unwrap();
        // Wait instead of Poll: sleep until an OS event or proxy.wake_up().
        // This alone drops idle CPU from ~100% of one core to near zero.
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

        // Connect the proxy so EventSinks can wake the loop.
        let proxy = event_loop.create_proxy();
        // Ignore the Err (only fails if already set, which cannot happen).
        let _ = self.proxy_cell.set(proxy);

        let handler = AppHandler { event_rx: self.event_rx.clone(), app: self };
        event_loop.run_app(handler).unwrap();
    }
}

struct AppHandler {
    app: App,
    event_rx: flume::Receiver<ExternalEvent>,
}

impl ApplicationHandler for AppHandler {
    fn proxy_wake_up(&mut self, _event_loop: &dyn ActiveEventLoop) {
        let Some(state) = self.app.state.as_mut() else {
            return;
        };
        let mut coalescer = RebuildCoalescer::default();
        let mut saw_animation_frame = false;
        coalescer.begin_drain();
        for event in self.event_rx.try_iter() {
            match event {
                ExternalEvent::RequestRebuild => {
                    coalescer.observe(true);
                }
                ExternalEvent::RequestRedraw => {
                    coalescer.observe(false);
                }
                ExternalEvent::RequestAnimationFrame => {
                    saw_animation_frame = true;
                }
                ExternalEvent::ActivateWindow => {
                    state.window.set_visible(true);
                    state.window.set_minimized(false);
                    state.window.focus_window();
                    state
                        .window
                        .request_user_attention(Some(AttentionUrgency::Informational.to_winit()));
                    coalescer.observe(false);
                }
                ExternalEvent::MinimizeWindow => {
                    state.window.set_minimized(true);
                    coalescer.observe(false);
                }
                ExternalEvent::ToggleMaximizeWindow => {
                    let maximized = state.window.is_maximized();
                    let next_maximized = !maximized;
                    state.window.set_maximized(next_maximized);
                    publish_window_maximized_change(
                        &mut state.window_maximized,
                        next_maximized,
                        self.app.config.on_window_maximized.as_ref(),
                    );
                    coalescer.observe(true);
                }
                ExternalEvent::Custom(payload) => {
                    if let Some(ref handler) = self.app.config.on_external_event {
                        handler(payload);
                    }
                    // Custom events typically change app state, so rebuild.
                    coalescer.observe(true);
                }
                ExternalEvent::Bytes(data) => {
                    if let Some(ref handler) = self.app.config.on_bytes {
                        (handler)(data);
                    }
                    coalescer.observe(false);
                }
                #[cfg(feature = "hot-reload")]
                ExternalEvent::StylesheetReload(new_stylesheet) => {
                    state.stylesheet = *new_stylesheet;
                    state.needs_restyle = true;
                    state.shaped_cache.clear();
                    state.shape_cache.clear();
                    state.batch_cache.clear();
                    // Collect all node IDs first, then mark each dirty.
                    let node_ids: Vec<_> = state.arena.iter().map(|(id, _)| id).collect();
                    for id in node_ids {
                        if let Some(elem) = state.arena.get_mut(id) {
                            elem.dirty |= unshit_core::dirty::DirtyFlags::STYLE
                                | unshit_core::dirty::DirtyFlags::SUBTREE_STYLE;
                        }
                    }
                    coalescer.observe(true);
                }
            }
        }
        if coalescer.needs_rebuild {
            state.needs_rebuild = true;
        }
        if coalescer.saw_event {
            // An external source (PTY reader, subscription, bridge, hot
            // reload, etc.) produced work for the UI thread. Count this as
            // activity so `about_to_wait` schedules speculative frames
            // during the next [`ACTIVITY_WINDOW`].
            state.mark_activity(Instant::now());
        }
        if saw_animation_frame && animations_active(state) {
            // A Timer-fallback waker tick (the waker never runs in
            // VsyncBlocking mode). All painting goes through the single
            // paint site, `RedrawRequested`; the force flag lets the
            // tick bypass the due-gate and pacer gate once, since the
            // waker already ticks at the display period.
            state.force_animation_paint = true;
            state.window.request_redraw();
        } else if saw_animation_frame && !coalescer.saw_event {
            // Stale waker tick after every animation settled: nothing to
            // paint, and no flag to leave armed.
            state.force_animation_paint = false;
        } else {
            state.window.request_redraw();
        }
    }

    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.app.state.is_some() {
            return;
        }

        let window: Arc<dyn Window> = Arc::from(window::create_window(
            event_loop,
            &self.app.config.title,
            self.app.config.width,
            self.app.config.height,
            self.app.config.decorations,
        ));

        let scale_factor = window.scale_factor() as f32;
        log::info!("Display scale factor: {:.2}x", scale_factor);

        if let Some(ref cb) = self.app.config.on_scale_factor {
            cb(scale_factor);
        }
        let initial_window_maximized = window.is_maximized();
        if let Some(ref cb) = self.app.config.on_window_maximized {
            cb(initial_window_maximized);
        }

        // Read the active monitor's refresh rate so the frame pacer can
        // coalesce at the real panel rhythm rather than the historic 8ms
        // default. Falls back to 0 (and thus `DEFAULT_MIN_INTERVAL`) on
        // platforms or configurations that cannot report the rate.
        let startup_refresh_mhz = {
            use crate::frame_pacer::MonitorRefreshSource as _;
            WindowRefreshSource(&*window).current_refresh_mhz().unwrap_or(0)
        };
        log::info!("Display refresh rate: {} mHz", startup_refresh_mhz);

        let mut gpu = pollster::block_on(GpuContext::new(window.clone()));

        // One-shot pacing mode selection: sound because surface
        // reconfigures reuse the stored present mode, so it cannot
        // change at runtime. On the vsync-paced default the blocking
        // acquire paces every paint; the Timer machinery only runs on
        // surfaces without Fifo.
        let pacing_mode = pacing_mode_for_surface(gpu.is_vsync_paced());
        log::info!("frame pacing: {:?} (present mode {:?})", pacing_mode, gpu.present_mode());

        // Rebuild the animation waker with the display's true period now
        // that the rate is known (it was constructed before the window
        // existed). Cheap: the tick thread spawns lazily on first use,
        // and in VsyncBlocking mode it is never used at all.
        self.app.animation_waker = AnimationWaker::new(
            self.app.event_tx.clone(),
            Arc::clone(&self.app.proxy_cell),
            crate::frame_pacer::FramePacer::interval_from_mhz(startup_refresh_mhz),
        );

        // Apply configurable atlas size bound if set.
        if let Some(max_bytes) = self.app.config.max_atlas_bytes {
            let derived_size = (max_bytes as f64).sqrt() as u32;
            gpu.glyph_atlas.max_size = derived_size.max(512);
        }

        // If a css_path is set, read that file into config.css so it acts as
        // the initial stylesheet (both here and in the hot-reload watcher).
        #[cfg(feature = "hot-reload")]
        if let Some(ref css_path) = self.app.config.css_path {
            match std::fs::read_to_string(css_path) {
                Ok(contents) => self.app.config.css = contents,
                Err(e) => log::warn!("hot-reload: failed to read {:?}: {}", css_path, e),
            }
        }

        let stylesheet = CompiledStylesheet::parse(&self.app.config.css);
        // Dev aid: surface declarations the engine could not type and silently
        // dropped (unrecognized property, or a value its parser rejected). The
        // enforcing guardrail is the app's `stylesheet_coverage` test; this is a
        // low-noise debug summary so a new gap is visible during `cargo run`.
        if !stylesheet.dropped.is_empty() {
            let custom = stylesheet.dropped.iter().filter(|d| d.is_custom_property()).count();
            let mut props: Vec<&str> = stylesheet
                .dropped
                .iter()
                .filter(|d| !d.is_custom_property())
                .map(|d| d.property.as_str())
                .collect();
            props.sort_unstable();
            props.dedup();
            log::debug!(
                "stylesheet: {} dropped declaration(s) the engine does not understand \
                 ({custom} custom-property defs); unsupported properties: {props:?}",
                stylesheet.dropped.len(),
            );
        }
        let mut font_system = FontSystem::new();
        let font_report =
            crate::font::load_custom_fonts(&mut font_system, &self.app.config.fonts, &stylesheet);
        if font_report.config_faces
            + font_report.css_faces
            + font_report.config_errors
            + font_report.css_errors
            > 0
        {
            log::info!(
                "Custom fonts: {} config face(s), {} @font-face face(s), {} config error(s), {} css error(s)",
                font_report.config_faces,
                font_report.css_faces,
                font_report.config_errors,
                font_report.css_errors,
            );
        }
        crate::font::check_fallback_chain(&font_system, &self.app.config.fallback_chain);
        let swash_cache = SwashCache::new();
        let subpixel_swash_cache = SubpixelSwashCache::new();
        #[cfg(target_os = "windows")]
        let dw_rasterizer = {
            let font_name = self
                .app
                .config
                .fonts
                .first()
                .and_then(|f| match f {
                    crate::font::FontSource::System(name) => Some(name.as_str()),
                    _ => None,
                })
                .unwrap_or("Consolas");
            DwRasterizer::new_with_custom_font_paths(
                font_name,
                collect_directwrite_font_paths(&self.app.config.fonts, &stylesheet),
            )
        };

        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();

        let element_tree = (self.app.tree_fn)();
        let root =
            build_tree_from_def(&element_tree.root, &mut arena, &mut taffy, NodeId::DANGLING);

        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
        let mut pseudo_table = unshit_core::style::pseudo::PseudoSideTable::new();
        unshit_core::build::resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut pseudo_table,
        );
        scale_all_styles(&mut arena, root, scale_factor);

        let mut measure_cache = TextMeasureCache::new();
        let (w, h) = gpu.window_size();
        run_layout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            w,
            h,
            &mut measure_cache,
        );

        // Mark every node dirty for paint so the first frame renders all elements.
        unshit_core::build::mark_paint_dirty(&mut arena, root);

        window.request_redraw();

        let event_log = if std::env::var("UNSHIT_RECORD_EVENTS").as_deref() == Ok("1") {
            log::info!("Event recording enabled (UNSHIT_RECORD_EVENTS=1)");
            Some(Vec::new())
        } else {
            None
        };

        let mut shortcut_resolver = ShortcutResolver::new();
        shortcut_resolver.register_defaults();

        // Register user-supplied shortcuts from AppConfig::user_shortcuts.
        // Each entry is a (shortcut_string, command_name) pair. Parse
        // errors are logged at warn level and skipped so one bad entry
        // never aborts startup.
        for (shortcut_str, command) in &self.app.config.user_shortcuts {
            match unshit_core::shortcut::Shortcut::parse(shortcut_str) {
                Ok(shortcut) => {
                    shortcut_resolver.registry_mut().register(unshit_core::shortcut::KeyBinding {
                        shortcut,
                        command: command.clone(),
                        when: unshit_core::shortcut::WhenClause::Always,
                        priority: unshit_core::shortcut::BindingPriority::User,
                    });
                }
                Err(e) => {
                    log::warn!("Failed to parse user shortcut {:?}: {}", shortcut_str, e);
                }
            }
        }

        if let Some(ref path) = self.app.config.keybindings_path {
            match crate::shortcut::load_keybindings_from_file(path) {
                Ok(bindings) => {
                    for binding in bindings {
                        shortcut_resolver.registry_mut().register(binding);
                    }
                    log::info!("Loaded user keybindings from {}", path);
                }
                Err(e) => {
                    log::warn!("Failed to load keybindings from {}: {}", path, e);
                }
            }
        }

        // Move canvas_registry out of App into AppState
        let canvas_registry =
            std::mem::replace(&mut self.app.canvas_registry, CanvasRegistry::new());

        self.app.state = Some(AppState {
            window,
            gpu,
            arena,
            taffy,
            root,
            stylesheet,
            font_system,
            swash_cache,
            subpixel_swash_cache,
            #[cfg(target_os = "windows")]
            dw_rasterizer,
            interaction: InteractionState::default(),
            needs_rebuild: false,
            needs_restyle: false,
            needs_relayout: false,
            restyle_root: None,
            scale_factor,
            window_maximized: initial_window_maximized,
            zoom_factor: 1.0,
            ctrl_held: false,
            shift_held: false,
            modifiers_state: ModifiersState::default(),
            shortcut_resolver,
            measure_cache,
            shaped_cache: ShapedTextCache::new(),
            batch_cache: BatchCache::new(),
            shape_cache: ShapeCache::new(),
            line_quad_cache: unshit_renderer::line_quad_cache::LineQuadCache::new(),
            canvas_registry,
            last_metrics: FrameMetrics::default(),
            frame_count: 0,
            fps_timer: Instant::now(),
            current_fps: 0.0,
            app_title: self.app.config.title.clone(),
            event_log,
            event_log_start: Instant::now(),
            scrollbar_visual: ScrollbarVisualState::default(),
            smooth_scroll: None,
            grid_animations: HashMap::new(),
            animation_next_frame: None,
            force_animation_paint: false,
            active_transitions: ActiveTransitions::default(),
            animation_driver: AnimationDriver::new(),
            pseudo_table,
            release_chord: unshit_core::shortcut::KeyCombo::new(
                Key::Escape,
                unshit_core::event::Modifiers::CTRL | unshit_core::event::Modifiers::SHIFT,
            ),
            bell_state: BellState::new(BellConfig::default()),
            theme: self.app.config.theme.clone(),
            cell_metrics_fired: false,
            pacing_mode,
            frame_pacer: crate::frame_pacer::FramePacer::with_refresh_rate_mhz(startup_refresh_mhz),
            // Treat window creation as activity so the first few frames
            // after startup run at the speculative pacer rhythm. This
            // smooths over the initial PTY-spawn / cell-metrics dance on
            // the app side before any user input arrives.
            last_activity: Instant::now(),
            frame_probe: crate::frame_probe::FrameProbe::new(),
            interval_probe: crate::frame_probe::FrameProbe::new(),
            last_paint_completed_at: None,
            display_refresh_mhz: startup_refresh_mhz,
            #[cfg(feature = "input-latency-histogram")]
            input_latency: crate::input_latency::InputLatencyTracker::new()
                .expect("hdrhistogram construction with valid sigfigs cannot fail"),
            last_refresh_probe: Instant::now(),
            frame_arena: FrameArena::default(),
        });

        // Run the initial subscription reconcile so streams start immediately.
        #[cfg(feature = "async")]
        if let Some(ref mut mgr) = self.app.subscription_manager {
            let sink = EventSink::new(self.app.event_tx.clone(), Arc::clone(&self.app.proxy_cell));
            mgr.reconcile(self.app.runtime.handle(), &sink);
        }

        // Spawn the hot-reload file watcher if a css_path is configured.
        #[cfg(feature = "hot-reload")]
        if let Some(css_path) = self.app.config.css_path.clone() {
            use notify::{EventKind, RecursiveMode, Watcher};
            let sink = self.app.event_sink();
            let path = css_path.clone();
            std::thread::spawn(move || {
                let (tx, rx) = std::sync::mpsc::channel();
                let mut watcher = match notify::recommended_watcher(move |res| {
                    if let Ok(event) = res {
                        let _ = tx.send(event);
                    }
                }) {
                    Ok(w) => w,
                    Err(e) => {
                        log::warn!("hot-reload: failed to create watcher: {}", e);
                        return;
                    }
                };
                if let Err(e) = watcher.watch(&path, RecursiveMode::NonRecursive) {
                    log::warn!("hot-reload: failed to watch {:?}: {}", path, e);
                    return;
                }
                log::info!("hot-reload: watching {:?}", path);
                for event in rx {
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        match std::fs::read_to_string(&path) {
                            Ok(contents) => {
                                let stylesheet = CompiledStylesheet::parse(&contents);
                                let boxed = Box::new(stylesheet);
                                if sink.send(ExternalEvent::StylesheetReload(boxed)).is_err() {
                                    // Event loop shut down; exit the watcher thread.
                                    break;
                                }
                                log::info!("hot-reload: stylesheet reloaded from {:?}", path);
                            }
                            Err(e) => {
                                log::warn!("hot-reload: failed to read {:?}: {}", path, e);
                            }
                        }
                    }
                }
            });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.app.state.as_mut() else {
            return;
        };

        // Classify the event as external activity so `about_to_wait` can
        // schedule speculative repaints for the next [`ACTIVITY_WINDOW`].
        // RedrawRequested is NOT activity; paints are driven BY activity,
        // so including them here would create a self-sustaining loop that
        // never returns to `ControlFlow::Wait`.
        let is_activity = matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::PointerButton { .. }
                | WindowEvent::PointerMoved { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::SurfaceResized(_)
                | WindowEvent::Moved(_)
                | WindowEvent::Focused(_)
                | WindowEvent::ModifiersChanged(_)
                | WindowEvent::Ime(_)
        );
        // The subset of activity events that count as user input for the
        // input latency instrument. SurfaceResized and Focused are
        // external events but not "input"; excluding them matches the
        // six variants called out in the issue #85 plan.
        #[cfg(feature = "input-latency-histogram")]
        let is_input = matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::PointerButton { .. }
                | WindowEvent::PointerMoved { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::ModifiersChanged(_)
                | WindowEvent::Ime(_)
        );
        if is_activity {
            state.mark_activity(Instant::now());
            #[cfg(feature = "input-latency-histogram")]
            if is_input {
                state.input_latency.record_event(Instant::now());
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                let should_exit = self.app.config.on_close.as_ref().map(|cb| cb()).unwrap_or(true);
                if !should_exit {
                    // Application vetoed the close (e.g. to show a confirm
                    // prompt). The app is expected to drive its own exit
                    // via `process::exit` once the user decides. Schedule a
                    // rebuild + redraw so any UI state the callback set
                    // (like a confirm dialog) actually paints, otherwise
                    // the window appears frozen until the next input event.
                    state.needs_rebuild = true;
                    state.window.request_redraw();
                    return;
                }
                if let Some(log) = state.event_log.take() {
                    let json = format!("[{}]", log.join(",\n"));
                    std::fs::write("events.json", json).ok();
                    log::info!("Event recording saved to events.json");
                }
                event_loop.exit();
            }

            WindowEvent::SurfaceResized(new_size) => {
                reconcile_surface_metrics(
                    state,
                    new_size,
                    state.window.scale_factor() as f32,
                    self.app.config.on_scale_factor.as_ref(),
                );
                if sync_window_maximized_from_window(
                    state,
                    self.app.config.on_window_maximized.as_ref(),
                ) {
                    state.needs_rebuild = true;
                }
                state.window.request_redraw();
            }

            WindowEvent::ScaleFactorChanged { scale_factor, surface_size_writer } => {
                let new_size = surface_size_writer
                    .surface_size()
                    .unwrap_or_else(|_| state.window.surface_size());
                reconcile_surface_metrics(
                    state,
                    new_size,
                    scale_factor as f32,
                    self.app.config.on_scale_factor.as_ref(),
                );
                if sync_window_maximized_from_window(
                    state,
                    self.app.config.on_window_maximized.as_ref(),
                ) {
                    state.needs_rebuild = true;
                }
                state.window.request_redraw();
            }

            WindowEvent::Moved(_position) => {
                let metrics_changed = !matches!(
                    reconcile_surface_metrics_from_window(
                        state,
                        self.app.config.on_scale_factor.as_ref()
                    ),
                    SurfaceMetricsChange::None
                );
                let maximized_changed = sync_window_maximized_from_window(
                    state,
                    self.app.config.on_window_maximized.as_ref(),
                );
                if maximized_changed {
                    state.needs_rebuild = true;
                }
                if metrics_changed || maximized_changed {
                    state.window.request_redraw();
                }

                // A drag delta fires one event per mouse move; debounce so
                // we only re-read the compositor every ACTIVITY_WINDOW.
                // Monitors with different refresh rates (e.g. 144 + 60)
                // update within 250ms of the drag settling.
                let now = Instant::now();
                if state.should_reprobe_refresh(now) {
                    refresh_pacer_from_window(state);
                }
            }

            WindowEvent::PointerMoved { position, .. } => {
                let pos = (position.x as f32, position.y as f32);
                if let Some(ref mut log) = state.event_log {
                    let ms = state.event_log_start.elapsed().as_millis();
                    log.push(format!(
                        r#"{{"type":"CursorMoved","x":{},"y":{},"time_ms":{}}}"#,
                        pos.0, pos.1, ms
                    ));
                }
                state.interaction.last_cursor_pos = pos;

                // Handle active CSS resize drag
                if let Some(info) = state.interaction.resize_drag {
                    use unshit_core::style::types::Dimension;
                    let dx = pos.0 - info.origin.0;
                    let dy = pos.1 - info.origin.1;
                    if let Some(element) = state.arena.get_mut(info.node_id) {
                        let style = &element.computed_style;
                        // Clamp to min/max constraints
                        let min_w = match style.min_width {
                            Dimension::Px(v) => v,
                            _ => 0.0,
                        };
                        let max_w = match style.max_width {
                            Dimension::Px(v) => v,
                            _ => f32::MAX,
                        };
                        let min_h = match style.min_height {
                            Dimension::Px(v) => v,
                            _ => 0.0,
                        };
                        let max_h = match style.max_height {
                            Dimension::Px(v) => v,
                            _ => f32::MAX,
                        };
                        if info.allow_horizontal {
                            let new_w = (info.initial_width + dx).clamp(min_w.max(20.0), max_w);
                            element.resize_override_width = Some(new_w);
                        }
                        if info.allow_vertical {
                            let new_h = (info.initial_height + dy).clamp(min_h.max(20.0), max_h);
                            element.resize_override_height = Some(new_h);
                        }
                    }
                    // Set appropriate cursor during resize drag
                    let cursor = match (info.allow_horizontal, info.allow_vertical) {
                        (true, true) => CursorIcon::NwseResize,
                        (true, false) => CursorIcon::EwResize,
                        (false, true) => CursorIcon::NsResize,
                        _ => CursorIcon::Default,
                    };
                    state.window.set_cursor(cursor.into());
                    state.needs_rebuild = true;
                    state.needs_restyle = true;
                    state.window.request_redraw();
                }
                // Handle active scrollbar drag
                else if let Some(ref drag) = state.interaction.scrollbar_drag {
                    let drag = *drag;
                    let cursor_pos = match drag.axis {
                        ScrollbarAxis::Vertical => pos.1,
                        ScrollbarAxis::Horizontal => pos.0,
                    };
                    let new_scroll = scroll::scroll_from_drag(&drag, cursor_pos);
                    if scroll::set_axis_scroll_position(
                        &mut state.arena,
                        drag.node_id,
                        drag.axis,
                        new_scroll,
                    ) {
                        state.window.request_redraw();
                    }
                } else if state.interaction.dragging {
                    // Active element drag: dispatch DragUpdate (pointer captured)
                    if let Some(handler_node) = state.interaction.drag_target {
                        let origin = state.interaction.drag_origin.unwrap_or(pos);
                        let last = state.interaction.drag_last_pos;
                        let (local_x, local_y) =
                            local_pointer_coords(&state.arena, handler_node, pos.0, pos.1);
                        let event = DragEvent {
                            phase: DragPhase::Update,
                            x: pos.0,
                            y: pos.1,
                            local_x,
                            local_y,
                            delta_x: pos.0 - last.0,
                            delta_y: pos.1 - last.1,
                            total_delta_x: pos.0 - origin.0,
                            total_delta_y: pos.1 - origin.1,
                            button: state.interaction.drag_button,
                        };
                        if let Some(element) = state.arena.get(handler_node) {
                            if let Some(ref on_drag) = element.on_drag {
                                on_drag(&event);
                            }
                        }
                        state.interaction.drag_last_pos = pos;
                    }
                    state.needs_rebuild = true;
                    state.window.request_redraw();
                } else if let Some(origin) = state.interaction.drag_origin {
                    // Drag threshold check
                    let dx = pos.0 - origin.0;
                    let dy = pos.1 - origin.1;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= DRAG_THRESHOLD {
                        let target = state.interaction.mousedown_target.unwrap_or(NodeId::DANGLING);
                        if let Some(handler_node) = find_drag_handler(&state.arena, target) {
                            state.interaction.drag_target = Some(handler_node);
                            state.interaction.dragging = true;
                            state.interaction.drag_last_pos = origin;

                            // Dispatch DragStart
                            let (local_x, local_y) =
                                local_pointer_coords(&state.arena, handler_node, pos.0, pos.1);
                            let event = DragEvent {
                                phase: DragPhase::Start,
                                x: pos.0,
                                y: pos.1,
                                local_x,
                                local_y,
                                delta_x: dx,
                                delta_y: dy,
                                total_delta_x: dx,
                                total_delta_y: dy,
                                button: state.interaction.drag_button,
                            };
                            if let Some(element) = state.arena.get(handler_node) {
                                if let Some(ref on_drag) = element.on_drag {
                                    on_drag(&event);
                                }
                            }
                            state.interaction.drag_last_pos = pos;
                            state.needs_rebuild = true;
                            state.window.request_redraw();
                        } else {
                            state.interaction.drag_origin = None;
                        }
                    }
                    // If threshold not met yet, fall through to normal hover
                    if !state.interaction.dragging {
                        handle_normal_hover(state, pos, self.app.config.decorations);
                    }
                } else {
                    handle_normal_hover(state, pos, self.app.config.decorations);
                }
            }

            WindowEvent::PointerButton { state: button_state, button, .. } => {
                use unshit_core::element::ElementContent;
                use winit::event::ElementState;
                if let Some(ref mut log) = state.event_log {
                    let ms = state.event_log_start.elapsed().as_millis();
                    let kind =
                        if button_state == ElementState::Pressed { "MouseDown" } else { "MouseUp" };
                    log.push(format!(r#"{{"type":"{}","time_ms":{}}}"#, kind, ms));
                }
                let mouse_button = button.mouse_button();
                if mouse_button == Some(winit::event::MouseButton::Left) {
                    match button_state {
                        ElementState::Pressed => {
                            let sb_pos = state.interaction.last_cursor_pos;
                            handle_normal_hover(state, sb_pos, self.app.config.decorations);
                            let region_target = state.interaction.hovered;
                            if let Some(direction) = custom_window_resize_direction(
                                self.app.config.decorations,
                                state.window.surface_size(),
                                sb_pos,
                                state.window.scale_factor() as f32,
                            ) {
                                state.interaction.drag_origin = None;
                                state.interaction.dragging = false;
                                state.interaction.drag_target = None;
                                state.interaction.mousedown_target = None;
                                state.interaction.resize_drag = None;
                                state.interaction.scrollbar_drag = None;
                                if let Err(err) = state.window.drag_resize_window(direction) {
                                    log::warn!("native window resize drag failed: {err}");
                                }
                                state.window.request_redraw();
                                return;
                            }

                            if is_window_drag_region(&state.arena, region_target) {
                                state.interaction.drag_origin = None;
                                state.interaction.dragging = false;
                                state.interaction.drag_target = None;
                                state.interaction.mousedown_target = None;
                                state.interaction.resize_drag = None;
                                state.interaction.scrollbar_drag = None;
                                if let Err(err) = state.window.drag_window() {
                                    log::warn!("native window drag failed: {err}");
                                }
                                return;
                            }

                            // Check for scrollbar interaction first
                            if let Some(hit) = scroll::find_scrollbar_at(
                                &state.arena,
                                state.root,
                                sb_pos.0,
                                sb_pos.1,
                            ) {
                                state.smooth_scroll = None;
                                match hit.part {
                                    ScrollbarPart::Thumb => {
                                        let grab_offset = match hit.axis {
                                            ScrollbarAxis::Vertical => {
                                                sb_pos.1 - hit.geometry.thumb_y
                                            }
                                            ScrollbarAxis::Horizontal => {
                                                sb_pos.0 - hit.geometry.thumb_x
                                            }
                                        };
                                        state.interaction.scrollbar_drag =
                                            Some(scroll::ScrollbarDrag {
                                                node_id: hit.node_id,
                                                axis: hit.axis,
                                                grab_offset,
                                                geometry: hit.geometry,
                                            });
                                        state.scrollbar_visual.dragging_node = Some(hit.node_id);
                                        state.scrollbar_visual.dragging_axis = Some(hit.axis);
                                    }
                                    ScrollbarPart::TrackBefore | ScrollbarPart::TrackAfter => {
                                        let cursor_pos = match hit.axis {
                                            ScrollbarAxis::Vertical => sb_pos.1,
                                            ScrollbarAxis::Horizontal => sb_pos.0,
                                        };
                                        let new_scroll = scroll::scroll_from_track_click(
                                            &hit.geometry,
                                            cursor_pos,
                                        );
                                        scroll::set_axis_scroll_position(
                                            &mut state.arena,
                                            hit.node_id,
                                            hit.axis,
                                            new_scroll,
                                        );
                                    }
                                }
                                state.window.request_redraw();
                            } else if let Some(resize_info) = find_resize_grip_at(
                                &state.arena,
                                state.root,
                                state.interaction.last_cursor_pos.0,
                                state.interaction.last_cursor_pos.1,
                            ) {
                                // CSS resize: start resize drag
                                state.interaction.resize_drag = Some(resize_info);
                                state.window.request_redraw();
                            } else {
                                // Begin potential drag: record origin for threshold check
                                let sb_pos = state.interaction.last_cursor_pos;
                                state.interaction.drag_origin = Some(sb_pos);
                                state.interaction.drag_button = MouseButton::Left;
                                state.interaction.dragging = false;
                                state.interaction.drag_target = None;

                                // Detect double-click: two left presses within 500ms on the same node
                                let now = Instant::now();
                                let hovered = state.interaction.hovered;
                                let is_double_click =
                                    if let Some(prev_time) = state.interaction.last_click_time {
                                        now.duration_since(prev_time).as_millis() < 500
                                            && state.interaction.last_click_node == hovered
                                    } else {
                                        false
                                    };

                                state.interaction.last_click_time = Some(now);
                                state.interaction.last_click_node = hovered;

                                if !hovered.is_dangling() {
                                    let old_active =
                                        state.interaction.active.unwrap_or(NodeId::DANGLING);
                                    state.interaction.active = Some(hovered);
                                    state.interaction.mousedown_target = Some(hovered);
                                    state.mark_restyle_pseudo_change(old_active, hovered);
                                    state.window.request_redraw();
                                }

                                let new_focused = find_focusable_ancestor(
                                    &state.arena,
                                    state.interaction.hovered,
                                )
                                .unwrap_or(NodeId::DANGLING);
                                if new_focused != state.interaction.focused {
                                    let old_focused = state.interaction.focused;
                                    state.interaction.focused = new_focused;
                                    state.interaction.focus_via_keyboard = false;
                                    update_focus_context(state);
                                    state.mark_restyle_pseudo_change(old_focused, new_focused);
                                    state.window.request_redraw();
                                }

                                // Dispatch a MouseDown along the hovered chain so
                                // grid/canvas widgets (e.g. the terminal) can place
                                // a selection anchor with the press coordinates and
                                // modifiers a Click event cannot carry. Additive:
                                // no built-in element registers a MouseDown handler,
                                // so existing widgets are unaffected. `local_*` is
                                // relative to the hovered leaf, which is the grid
                                // element itself (it has no child cell nodes).
                                if !state.interaction.hovered.is_dangling() {
                                    let (lx, ly) = local_pointer_coords(
                                        &state.arena,
                                        state.interaction.hovered,
                                        sb_pos.0,
                                        sb_pos.1,
                                    );
                                    let md = Event::Mouse(MouseEvent {
                                        kind: MouseEventKind::Down,
                                        x: sb_pos.0,
                                        y: sb_pos.1,
                                        local_x: lx,
                                        local_y: ly,
                                        button: MouseButton::Left,
                                        modifiers: modifiers_from_winit(&state.modifiers_state),
                                    });
                                    if dispatch_mouse_button_event(
                                        &state.arena,
                                        state.interaction.hovered,
                                        EventType::MouseDown,
                                        &md,
                                    ) {
                                        state.needs_rebuild = true;
                                    }
                                }

                                // Click-to-position cursor in text input
                                if let Some(element) = state.arena.get(new_focused) {
                                    use unshit_core::element::InputType;
                                    if element.tag == Tag::Input {
                                        match element.input_state.input_type {
                                            InputType::Range => {
                                                // Jump thumb to click position.
                                                let pos = state.interaction.last_cursor_pos;
                                                let rect = element.layout_rect;
                                                let style = &element.computed_style;
                                                let track_x = rect.x + style.padding.left;
                                                let track_w = rect.width
                                                    - style.padding.left
                                                    - style.padding.right;
                                                if track_w > 0.0 {
                                                    let local_x =
                                                        (pos.0 - track_x).clamp(0.0, track_w);
                                                    let ratio = local_x / track_w;
                                                    let min = element.input_state.min;
                                                    let max = element.input_state.max;
                                                    let raw = min + ratio * (max - min);
                                                    let step = element.input_state.step;
                                                    let snapped = (raw / step).round() * step;
                                                    let clamped = snapped.clamp(min, max);
                                                    if let Some(elem) =
                                                        state.arena.get_mut(new_focused)
                                                    {
                                                        elem.input_state.numeric_value = clamped;
                                                        let display = if clamped.fract() == 0.0 {
                                                            format!("{}", clamped as i64)
                                                        } else {
                                                            format!("{}", clamped)
                                                        };
                                                        elem.input_state.value = display;
                                                        if let Some(f) = elem.on_change.clone() {
                                                            f(&elem.input_state.value.clone());
                                                        }
                                                    }
                                                }
                                            }
                                            InputType::Text
                                            | InputType::Password
                                            | InputType::Number
                                                if !element.input_state.value.is_empty() =>
                                            {
                                                let pos = state.interaction.last_cursor_pos;
                                                let rect = element.layout_rect;
                                                let style = &element.computed_style;
                                                let local_x = pos.0 - rect.x - style.padding.left;
                                                let local_y = pos.1 - rect.y - style.padding.top;
                                                let content_w = rect.width
                                                    - style.padding.left
                                                    - style.padding.right;

                                                // For password, hit-test on the masked text.
                                                let hit_text = if element.input_state.input_type
                                                    == InputType::Password
                                                {
                                                    "\u{2022}".repeat(
                                                        element.input_state.value.chars().count(),
                                                    )
                                                } else {
                                                    element.input_state.value.clone()
                                                };

                                                let byte_offset = layout::hit_test_text_position(
                                                    &hit_text,
                                                    style.font_size,
                                                    style.line_height,
                                                    style.letter_spacing,
                                                    Some(content_w),
                                                    local_x,
                                                    local_y,
                                                    &mut state.font_system,
                                                )
                                                .unwrap_or(element.input_state.value.len());

                                                if let Some(elem) = state.arena.get_mut(new_focused)
                                                {
                                                    elem.input_state.cursor_pos = byte_offset;
                                                }
                                            }
                                            // Checkbox, Radio, Hidden: no cursor positioning.
                                            _ => {}
                                        }
                                    }
                                }

                                // Text selection: start on mousedown over text.
                                // Respect user-select: none to prevent selection.
                                let user_select_none = state
                                    .arena
                                    .get(hovered)
                                    .map(|e| {
                                        e.computed_style.user_select
                                            == unshit_core::style::types::UserSelect::None
                                    })
                                    .unwrap_or(false);
                                let pos = state.interaction.last_cursor_pos;
                                if !user_select_none {
                                    if let Some((text_node, byte_offset)) = layout::text_hit_at(
                                        &state.arena,
                                        hovered,
                                        pos.0,
                                        pos.1,
                                        &mut state.font_system,
                                    ) {
                                        if is_double_click {
                                            // Double-click: select the word at the click position
                                            if let Some(elem) = state.arena.get(text_node) {
                                                if let ElementContent::Text(ref text) = elem.content
                                                {
                                                    let (start, end) =
                                                        word_boundary_at(text, byte_offset);
                                                    state.interaction.text_selection =
                                                        Some(TextSelection {
                                                            anchor_element: text_node,
                                                            anchor_offset: start,
                                                            focus_element: text_node,
                                                            focus_offset: end,
                                                        });
                                                    // Reset last_click_time so a third click is not another double-click
                                                    state.interaction.last_click_time = None;
                                                }
                                            }
                                            state.interaction.selecting = false;
                                        } else {
                                            state.interaction.text_selection =
                                                Some(TextSelection {
                                                    anchor_element: text_node,
                                                    anchor_offset: byte_offset,
                                                    focus_element: text_node,
                                                    focus_offset: byte_offset,
                                                });
                                            state.interaction.selecting = true;
                                        }
                                        state.window.request_redraw();
                                    } else {
                                        state.interaction.text_selection = None;
                                        state.interaction.selecting = false;
                                    }
                                } // user-select gate
                            }
                        }
                        ElementState::Released => {
                            if state.interaction.resize_drag.is_some() {
                                state.interaction.resize_drag = None;
                                state.window.request_redraw();
                            } else if state.interaction.scrollbar_drag.is_some() {
                                state.interaction.scrollbar_drag = None;
                                state.scrollbar_visual.clear_drag();
                                state.window.request_redraw();
                            } else if state.interaction.dragging {
                                // Dispatch DragEnd, suppress click
                                let pos = state.interaction.last_cursor_pos;
                                if let Some(handler_node) = state.interaction.drag_target {
                                    let origin = state.interaction.drag_origin.unwrap_or(pos);
                                    let last = state.interaction.drag_last_pos;
                                    let (local_x, local_y) = local_pointer_coords(
                                        &state.arena,
                                        handler_node,
                                        pos.0,
                                        pos.1,
                                    );
                                    let event = DragEvent {
                                        phase: DragPhase::End,
                                        x: pos.0,
                                        y: pos.1,
                                        local_x,
                                        local_y,
                                        delta_x: pos.0 - last.0,
                                        delta_y: pos.1 - last.1,
                                        total_delta_x: pos.0 - origin.0,
                                        total_delta_y: pos.1 - origin.1,
                                        button: state.interaction.drag_button,
                                    };
                                    if let Some(element) = state.arena.get(handler_node) {
                                        if let Some(ref on_drag) = element.on_drag {
                                            on_drag(&event);
                                        }
                                    }
                                }
                                state.interaction.drag_origin = None;
                                state.interaction.drag_target = None;
                                state.interaction.dragging = false;
                                state.interaction.mousedown_target = None;
                                state.needs_rebuild = true;
                                state.window.request_redraw();
                            } else if let Some(mousedown_target) =
                                state.interaction.mousedown_target.take()
                            {
                                // Normal click (no drag occurred)
                                state.interaction.drag_origin = None;
                                let pos = state.interaction.last_cursor_pos;
                                handle_normal_hover(state, pos, self.app.config.decorations);
                                // Handle checkbox/radio click before generic dispatch.
                                let input_handled = handle_input_click(state, mousedown_target);
                                let consumed_by_select = handle_select_click(state, pos.0, pos.1);
                                if input_handled
                                    || consumed_by_select
                                    || dispatch_click(
                                        &state.arena,
                                        mousedown_target,
                                        state.interaction.hovered,
                                    )
                                {
                                    state.needs_rebuild = true;
                                    state.window.request_redraw();
                                }
                            }

                            state.interaction.selecting = false;

                            if let Some(old_active) = state.interaction.active {
                                state.interaction.active = None;
                                state.mark_restyle_pseudo_change(old_active, NodeId::DANGLING);
                                state.window.request_redraw();
                            }
                        }
                    }
                } else if mouse_button == Some(winit::event::MouseButton::Right) {
                    if button_state == ElementState::Pressed {
                        let (cx, cy) = state.interaction.last_cursor_pos;
                        if dispatch_context_menu(&state.arena, state.interaction.hovered, cx, cy) {
                            state.needs_rebuild = true;
                            state.window.request_redraw();
                        }
                    }
                }
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                state.ctrl_held = modifiers.state().control_key();
                state.shift_held = modifiers.state().shift_key();
                state.modifiers_state = modifiers.state();
            }

            WindowEvent::Focused(focused) => {
                if !matches!(
                    reconcile_surface_metrics_from_window(
                        state,
                        self.app.config.on_scale_factor.as_ref(),
                    ),
                    SurfaceMetricsChange::None
                ) {
                    state.window.request_redraw();
                }

                // Losing focus mid-drag (e.g. Alt-Tab) means the mouse-up
                // will never be delivered to our window. Synthesize a
                // DragPhase::End so the app's on_drag handler can clean up
                // instead of leaving ghost overlays stuck on screen.
                if !focused && state.interaction.dragging {
                    let pos = state.interaction.last_cursor_pos;
                    if let Some(handler_node) = state.interaction.drag_target {
                        let origin = state.interaction.drag_origin.unwrap_or(pos);
                        let last = state.interaction.drag_last_pos;
                        let (local_x, local_y) =
                            local_pointer_coords(&state.arena, handler_node, pos.0, pos.1);
                        let event = DragEvent {
                            phase: DragPhase::End,
                            x: pos.0,
                            y: pos.1,
                            local_x,
                            local_y,
                            delta_x: pos.0 - last.0,
                            delta_y: pos.1 - last.1,
                            total_delta_x: pos.0 - origin.0,
                            total_delta_y: pos.1 - origin.1,
                            button: state.interaction.drag_button,
                        };
                        if let Some(element) = state.arena.get(handler_node) {
                            if let Some(ref on_drag) = element.on_drag {
                                on_drag(&event);
                            }
                        }
                    }
                    state.interaction.drag_origin = None;
                    state.interaction.drag_target = None;
                    state.interaction.dragging = false;
                    state.interaction.mousedown_target = None;
                    state.needs_rebuild = true;
                    state.window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == winit::event::ElementState::Pressed {
                    // FIRST: check if focused element captures keyboard input
                    let mut focused_captures = state
                        .arena
                        .get(state.interaction.focused)
                        .map(|e| e.captures_keyboard || e.computed_style.keyboard_capture)
                        .unwrap_or(false);

                    // Fallback: when focus is on a non-capturing, non-editable element
                    // (e.g., a sidebar entry or a button that was just clicked), route
                    // keyboard events to any element that declares captures_keyboard.
                    // This lets users type into a terminal pane immediately after
                    // clicking UI that switches to it, without a second click.
                    if !focused_captures {
                        let focused_editable = state
                            .arena
                            .get(state.interaction.focused)
                            .map(|e| matches!(e.tag, Tag::Input | Tag::Select))
                            .unwrap_or(false);
                        if !focused_editable {
                            let fallback_id = state
                                .arena
                                .iter()
                                .find(|(_, e)| e.captures_keyboard)
                                .map(|(id, _)| id);
                            if let Some(fallback_id) = fallback_id {
                                if fallback_id != state.interaction.focused {
                                    let old_focused = state.interaction.focused;
                                    state.interaction.focused = fallback_id;
                                    state.interaction.focus_via_keyboard = false;
                                    state.mark_restyle_pseudo_change(old_focused, fallback_id);
                                }
                                focused_captures = true;
                            }
                        }
                    }

                    if focused_captures {
                        if let Some(combo) =
                            key_combo_from_winit(&event.logical_key, &state.modifiers_state)
                        {
                            // Release chord: clear capture, do NOT forward the event
                            if combo == state.release_chord {
                                if let Some(e) = state.arena.get_mut(state.interaction.focused) {
                                    e.captures_keyboard = false;
                                }
                                state.window.request_redraw();
                            } else if consume_raw_key_hook(&self.app.config, &combo) {
                                state.needs_rebuild = true;
                                state.window.request_redraw();
                            } else {
                                // Command-level modifiers and function keys are app
                                // shortcuts even while a terminal captures keyboard input.
                                // Plain text keys still bypass this so normal typing and
                                // Shift+PageUp/Down reach the terminal.
                                let shortcut_handled =
                                    if should_check_shortcut_during_keyboard_capture(&combo) {
                                        let was_chord_pending =
                                            state.shortcut_resolver.is_chord_pending();
                                        let matched = state.shortcut_resolver.process_key(
                                            combo,
                                            &state.interaction,
                                            &state.arena,
                                        );
                                        if let Some(command) = matched {
                                            dispatch_command(
                                                state,
                                                &command,
                                                self.app.config.on_command.as_ref(),
                                            );
                                            true
                                        } else if state.shortcut_resolver.is_chord_pending()
                                            && !was_chord_pending
                                        {
                                            // Entered chord state (e.g. Ctrl+K as chord
                                            // leader). Consume the key; don't forward.
                                            state.window.request_redraw();
                                            true
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    };

                                if !shortcut_handled {
                                    // No shortcut match: forward to the capture handler.
                                    let focused_id = state.interaction.focused;
                                    let kbd_event = Event::Keyboard(KeyboardEvent {
                                        kind: unshit_core::event::KeyEventKind::Pressed,
                                        key: combo.key,
                                        modifiers: combo.modifiers,
                                        text: event.text.as_ref().map(|t| t.to_string()),
                                    });
                                    if let Some(element) = state.arena.get(focused_id) {
                                        for (et, handler) in &element.handlers {
                                            if *et == EventType::KeyboardCapture {
                                                handler(&kbd_event);
                                            }
                                        }
                                    }
                                    state.needs_rebuild = true;
                                    state.window.request_redraw();
                                }
                            }
                        }
                    } else {
                        // Normal keyboard handling: raw app hook, clipboard, select,
                        // text input, then shortcuts.
                        let combo =
                            key_combo_from_winit(&event.logical_key, &state.modifiers_state);
                        let consumed_by_raw = combo
                            .as_ref()
                            .map(|combo| consume_raw_key_hook(&self.app.config, combo))
                            .unwrap_or(false);
                        if consumed_by_raw {
                            state.needs_rebuild = true;
                            state.window.request_redraw();
                            return;
                        }

                        let focused_is_input = state
                            .arena
                            .get(state.interaction.focused)
                            .map(|e| e.tag == Tag::Input)
                            .unwrap_or(false);

                        let focused_is_select = state
                            .arena
                            .get(state.interaction.focused)
                            .map(|e| e.tag == Tag::Select)
                            .unwrap_or(false);

                        // Handle clipboard shortcuts (Ctrl+C/V/X) when a text input is focused
                        let handled_by_clipboard =
                            if focused_is_input && state.modifiers_state.control_key() {
                                handle_clipboard_shortcut(state, &event, &self.app.clipboard)
                            } else {
                                false
                            };

                        let handled_by_input = if handled_by_clipboard {
                            true
                        } else if focused_is_select {
                            handle_select_keyboard(state, &event)
                        } else if focused_is_input {
                            handle_text_input(state, &event)
                        } else {
                            false
                        };

                        if !handled_by_input {
                            if let Some(combo) = combo {
                                if combo.key == Key::Escape
                                    && state.shortcut_resolver.is_chord_pending()
                                {
                                    // Cancel chord on Escape
                                    state.shortcut_resolver.cancel_chord();
                                    state.window.request_redraw();
                                } else {
                                    let was_pending = state.shortcut_resolver.is_chord_pending();
                                    if let Some(command) = state.shortcut_resolver.process_key(
                                        combo,
                                        &state.interaction,
                                        &state.arena,
                                    ) {
                                        dispatch_command(
                                            state,
                                            &command,
                                            self.app.config.on_command.as_ref(),
                                        );
                                    }
                                    if was_pending != state.shortcut_resolver.is_chord_pending() {
                                        state.window.request_redraw();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(ref mut log) = state.event_log {
                    let ms = state.event_log_start.elapsed().as_millis();
                    let (dx, dy) = match delta {
                        winit::event::MouseScrollDelta::LineDelta(x, y) => (x, y),
                        winit::event::MouseScrollDelta::PixelDelta(pos) => {
                            (pos.x as f32, pos.y as f32)
                        }
                    };
                    log.push(format!(
                        r#"{{"type":"MouseWheel","dx":{},"dy":{},"time_ms":{}}}"#,
                        dx, dy, ms
                    ));
                }
                if state.ctrl_held {
                    // Zoom handling (Ctrl + scroll)
                    let scroll_y = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                        winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 50.0,
                    };
                    let old_zoom = state.zoom_factor;
                    state.zoom_factor =
                        (state.zoom_factor * (1.0 + scroll_y * 0.1)).clamp(0.25, 5.0);
                    if (state.zoom_factor - old_zoom).abs() > 0.001 {
                        log::info!("Zoom: {:.0}%", state.zoom_factor * 100.0);
                        state.shaped_cache.clear();
                        state.batch_cache.clear();
                        state.measure_cache.clear();
                        state.needs_rebuild = true;
                        state.window.request_redraw();
                    }
                } else {
                    let scroll_tuning = self
                        .app
                        .config
                        .scroll_tuning
                        .as_ref()
                        .map(|read| read())
                        .unwrap_or_default()
                        .sanitized();
                    let (delta_x, delta_y, smooth_scroll) = wheel_scroll_delta_pixels(
                        delta,
                        state.scale_factor,
                        state.zoom_factor,
                        scroll_tuning,
                        chars_per_notch(),
                        lines_per_notch(),
                    );
                    let duration_delta = unscaled_scroll_delta(
                        (delta_x, delta_y),
                        state.scale_factor,
                        state.zoom_factor,
                    );
                    let pos = state.interaction.last_cursor_pos;
                    handle_normal_hover(state, pos, self.app.config.decorations);

                    let scroll_target =
                        scroll::find_scroll_container(&state.arena, state.interaction.hovered);

                    if let Some(target_id) = scroll_target {
                        if smooth_scroll {
                            let current =
                                state.arena.get(target_id).map(|el| (el.scroll_x, el.scroll_y));
                            if let Some(current) = current {
                                let max_scroll = scroll::compute_max_scroll(
                                    &state.arena,
                                    &state.taffy,
                                    target_id,
                                );
                                let duration =
                                    browser_like_wheel_duration(duration_delta, scroll_tuning);
                                let scroll_started_at = Instant::now();
                                state.smooth_scroll = next_smooth_scroll(
                                    current,
                                    max_scroll,
                                    state.smooth_scroll,
                                    target_id,
                                    (delta_x, delta_y),
                                    scroll_started_at,
                                    duration,
                                    browser_like_initial_slope(duration_delta),
                                );
                                if state.smooth_scroll.is_some() {
                                    kick_animation(
                                        state,
                                        &self.app.animation_waker,
                                        scroll_started_at + duration + SMOOTH_SCROLL_WAKE_GRACE,
                                    );
                                    if let Some(scroll) = state.smooth_scroll {
                                        emit_scroll_telemetry(
                                            self.app.config.on_scroll_telemetry.as_deref(),
                                            scroll_telemetry(
                                                scroll,
                                                ScrollTelemetryPhase::Started,
                                                scroll.started_at,
                                            ),
                                        );
                                    }
                                }
                            }
                        } else {
                            state.smooth_scroll = None;
                            if scroll::scroll_by(
                                &mut state.arena,
                                &state.taffy,
                                target_id,
                                delta_x,
                                delta_y,
                            ) {
                                let pos = state.interaction.last_cursor_pos;
                                handle_normal_hover(state, pos, self.app.config.decorations);
                                let scroll =
                                    state.arena.get(target_id).map(|el| (el.scroll_x, el.scroll_y));
                                if let Some((scroll_x, scroll_y)) = scroll {
                                    emit_scroll_telemetry(
                                        self.app.config.on_scroll_telemetry.as_deref(),
                                        ScrollTelemetry {
                                            phase: ScrollTelemetryPhase::Instant,
                                            node_id: target_id,
                                            elapsed_ms: 0.0,
                                            duration_ms: 0.0,
                                            start_x: scroll_x,
                                            start_y: scroll_y,
                                            scroll_x,
                                            scroll_y,
                                            target_x: scroll_x,
                                            target_y: scroll_y,
                                            velocity_x: 0.0,
                                            velocity_y: 0.0,
                                            progress_y: 1.0,
                                        },
                                    );
                                }
                                state.window.request_redraw();
                            }
                        }
                    }

                    // Dispatch Scroll event to element handlers. Walk from the
                    // hovered element up to the root, firing the first handler
                    // found (bubble semantics).
                    //
                    // A handler that returns a boxed [`ScrollGridPatch`] opts
                    // into scoped invalidation: its fresh grid is written onto
                    // the handling node and only paint-dirtied, so the wheel
                    // event repaints without a full tree rebuild. Any other
                    // return value keeps the legacy full-rebuild behavior.
                    //
                    // The event carries the same animation parameters the
                    // container smooth-scroll path computed above (duration
                    // and initial-slope ramps for this delta), so app-managed
                    // scroll surfaces animate with bit-identical feel.
                    let (smooth_duration_ms, smooth_initial_slope) = if smooth_scroll {
                        (
                            browser_like_wheel_duration(duration_delta, scroll_tuning)
                                .as_secs_f32()
                                * 1000.0,
                            browser_like_initial_slope(duration_delta),
                        )
                    } else {
                        (0.0, 0.0)
                    };
                    let pos = state.interaction.last_cursor_pos;
                    let scroll_evt =
                        unshit_core::event::Event::Scroll(unshit_core::event::ScrollEvent {
                            delta_x,
                            delta_y,
                            x: pos.0,
                            y: pos.1,
                            animate: smooth_scroll,
                            smooth_duration_ms,
                            smooth_initial_slope,
                        });
                    let mut node = state.interaction.hovered;
                    while let Some(element) = state.arena.get(node) {
                        let handler = element
                            .handlers
                            .iter()
                            .find(|(et, _)| *et == unshit_core::event::EventType::Scroll)
                            .map(|(_, handler)| handler.clone());
                        if let Some(handler) = handler {
                            match handler(&scroll_evt)
                                .and_then(|value| value.downcast::<ScrollGridPatch>().ok())
                            {
                                Some(patch) => {
                                    if let Some(grid) = patch.grid {
                                        if apply_scroll_grid_patch(&mut state.arena, node, grid) {
                                            state.window.request_redraw();
                                        } else {
                                            // Node gone, non-grid content, or
                                            // dimensions changed underneath the
                                            // handler: fall back to a rebuild.
                                            state.needs_rebuild = true;
                                            state.window.request_redraw();
                                        }
                                    }
                                    // `grid: None`: consumed with no visual
                                    // change; schedule nothing (unless an
                                    // animation hook below wants frames).
                                    if let Some(hook) = patch.animation {
                                        // Register (or replace) the node's
                                        // grid animation and start (or
                                        // retarget) the frame chain; in
                                        // Timer mode the hook's deadline
                                        // (which already includes grace)
                                        // keeps the shared waker alive.
                                        let deadline = hook.deadline;
                                        state.grid_animations.insert(node, hook);
                                        kick_animation(state, &self.app.animation_waker, deadline);
                                    }
                                }
                                None => {
                                    state.needs_rebuild = true;
                                    state.window.request_redraw();
                                }
                            }
                            break;
                        }
                        let parent = element.parent;
                        if parent.is_dangling() {
                            break;
                        }
                        node = parent;
                    }
                }
            }

            WindowEvent::Ime(ime) => {
                use unshit_core::dirty::DirtyFlags;
                match ime {
                    winit::event::Ime::Enabled => {
                        // IME activated; no immediate action required.
                    }
                    winit::event::Ime::Preedit(text, cursor) => {
                        if let Some(elem) = state.arena.get_mut(state.interaction.focused) {
                            if elem.tag == Tag::Input {
                                elem.input_state.preedit =
                                    if text.is_empty() { None } else { Some(text.clone()) };
                                elem.input_state.preedit_cursor = cursor;
                                elem.dirty |= DirtyFlags::PAINT;
                                state.window.request_redraw();
                            }
                        }
                    }
                    winit::event::Ime::Commit(text) => {
                        if let Some(elem) = state.arena.get_mut(state.interaction.focused) {
                            if elem.tag == Tag::Input {
                                let pos = elem.input_state.cursor_pos;
                                elem.input_state.value.insert_str(pos, &text);
                                elem.input_state.cursor_pos += text.len();
                                elem.input_state.preedit = None;
                                elem.input_state.preedit_cursor = None;
                                elem.dirty |= DirtyFlags::LAYOUT;
                                state.needs_relayout = true;
                                state.shaped_cache.clear();
                                state.window.request_redraw();
                            }
                        }
                    }
                    winit::event::Ime::Disabled => {
                        if let Some(elem) = state.arena.get_mut(state.interaction.focused) {
                            if elem.tag == Tag::Input {
                                elem.input_state.preedit = None;
                                elem.input_state.preedit_cursor = None;
                            }
                        }
                    }
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => {
                let frame_start = Instant::now();
                // A minimized window cannot present: Vulkan acquires
                // return Outdated with no Fifo throttle, so painting
                // would loop full batch builds at CPU speed. Drop the
                // paint and let dirty flags accumulate untouched;
                // restoring the window generates a real WM_PAINT, which
                // resumes painting (and settles any expired animation
                // hooks). Polled here because winit's win32 backend
                // never delivers the zero-size resize events a
                // suppression flag could key off.
                if state.window.is_minimized().unwrap_or(false) {
                    return;
                }
                let force_animation_paint = std::mem::take(&mut state.force_animation_paint);
                let animation_active = animations_active(state);
                if animation_active && can_fast_paint_animations(state) {
                    // Timer fallback only: the due-gate paces the chain at
                    // the true period. The redraw is NOT re-requested on
                    // the wait branch (a queued internal paint would wake
                    // the OS wait immediately); `about_to_wait` re-issues
                    // it when the gate opens. In VsyncBlocking mode there
                    // is no gate to check — the blocking acquire paces.
                    if state.pacing_mode == PacingMode::Timer && !force_animation_paint {
                        let due = state.animation_next_frame.unwrap_or(frame_start);
                        if frame_start < due {
                            event_loop
                                .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
                            return;
                        }
                    }
                    let painted = fast_paint_animation_frame(
                        state,
                        frame_start,
                        self.app.config.decorations,
                        self.app.config.on_scroll_telemetry.as_deref(),
                        self.app.config.on_frame_metrics.as_deref(),
                    );
                    schedule_animation_followup(state, event_loop, frame_start, painted);
                    return;
                }

                // Flip the input latency tracker BEFORE the pacer early
                // return so events that arrive during a pacer sleep are
                // counted as mid draw drops rather than leaking into the
                // next frame's latency sample.
                #[cfg(feature = "input-latency-histogram")]
                state.input_latency.mark_frame_start();

                // Timer fallback only: coalesce RequestRebuild and
                // input-driven redraws into at most one paint per
                // frame_pacer.min_interval (the true display period).
                // Waker-driven animation ticks bypass the gate — they
                // already arrive at the period. In VsyncBlocking mode the
                // gate is skipped entirely: the blocking acquire is the
                // coalescer, and a redraw storm paints back-to-back
                // frames that each block until their vblank.
                let bypass_pacer_gate = match state.pacing_mode {
                    PacingMode::VsyncBlocking => true,
                    PacingMode::Timer => force_animation_paint,
                };
                if !bypass_pacer_gate {
                    match state.frame_pacer.on_redraw_requested(frame_start) {
                        crate::frame_pacer::PaceDecision::PaintNow => {}
                        crate::frame_pacer::PaceDecision::WaitUntil(deadline) => {
                            // Sleep until the coalescing deadline; the
                            // speculative/hygiene branches in
                            // `about_to_wait` re-issue the redraw once it
                            // passes (requesting it here would wake the
                            // OS wait immediately and spin).
                            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                                deadline,
                            ));
                            return;
                        }
                    }
                }

                let mut metrics = FrameMetrics::default();

                // Animation positions sample at the predicted present
                // time (see predicted_present_ts); everything else in
                // this frame keeps using frame_start.
                let sample_ts = predicted_present_ts(
                    state.last_paint_completed_at,
                    display_period_ns_from_mhz(state.display_refresh_mhz),
                    frame_start,
                );
                tick_smooth_scroll(
                    state,
                    sample_ts,
                    self.app.config.decorations,
                    self.app.config.on_scroll_telemetry.as_deref(),
                );
                // Grid animations advance on the slow path too, BEFORE the
                // rebuild below, so a rebuilt snapshot reflects the animated
                // position and reconcile classifies the grid swap as
                // paint-only. On rebuild frames the per-node patch apply is
                // skipped (`tree_fn` recomposes every pane anyway); on
                // restyle/relayout-only frames the patches are applied so
                // the animated content still reaches this frame's batch.
                tick_grid_animations(state, sample_ts, !state.needs_rebuild);

                // Advance LRU frame counter at the start of each rendered frame.
                state.gpu.glyph_atlas.advance_frame();

                // Periodically evict glyphs unused for more than 300 frames.
                {
                    let atlas_frame = state.gpu.glyph_atlas.lru.frame_counter;
                    if atlas_frame > 0 && atlas_frame % 300 == 0 {
                        let prev_generation = state.gpu.glyph_atlas.generation;
                        let device = state.gpu.device.clone();
                        let queue = state.gpu.queue.clone();
                        state.gpu.glyph_atlas.evict_unused(300, &device, &queue);
                        if state.gpu.glyph_atlas.generation != prev_generation {
                            state.shaped_cache.clear();
                            state.gpu.refresh_glyph_atlas_bind_groups();
                        }
                    }
                }

                if state.needs_rebuild {
                    let t0 = Instant::now();
                    // Prefer the bump path when configured. The bump tree
                    // is built into the per-frame arena, reconciled, and
                    // the arena is reset at the end of the frame to keep
                    // chunk capacity for the next frame (zed pattern).
                    let pending_mounts =
                        if let Some(ref tree_fn_bump) = self.app.config.tree_fn_bump {
                            // Borrow the arena immutably for the lifetime
                            // of the returned bump tree. The tree is
                            // consumed fully inside reconcile_bump; the
                            // borrow ends before the frame-end arena
                            // reset.
                            let bump_tree = (tree_fn_bump)(&state.frame_arena);
                            unshit_core::reconcile::reconcile_bump(
                                &mut state.arena,
                                &mut state.taffy,
                                state.root,
                                &bump_tree.root,
                            )
                        } else {
                            let new_tree = (self.app.tree_fn)();
                            unshit_core::reconcile::reconcile(
                                &mut state.arena,
                                &mut state.taffy,
                                state.root,
                                &new_tree.root,
                            )
                        };
                    // Fire mount callbacks now that the arena borrow is released.
                    for (node_id, cb) in pending_mounts {
                        cb(node_id);
                    }
                    metrics.tree_build_us = t0.elapsed().as_micros() as u64;

                    // Invalidate stale interaction state
                    if state.arena.get(state.interaction.hovered).is_none() {
                        state.interaction.hovered = NodeId::DANGLING;
                    }
                    if let Some(active) = state.interaction.active {
                        if state.arena.get(active).is_none() {
                            state.interaction.active = None;
                        }
                    }
                    if state.arena.get(state.interaction.focused).is_none() {
                        state.interaction.focused = NodeId::DANGLING;
                    }

                    // Honor an autofocus request recorded while building a
                    // freshly mounted node (e.g. a dialog input). Building
                    // runs only for new nodes, so this fires once when the
                    // element first appears and never on the reconcile-only
                    // rebuilds that typing into the input triggers, so it
                    // cannot yank focus back after the user tabs away.
                    if let Some(focus_id) = state.arena.pending_autofocus.take() {
                        if state.arena.get(focus_id).is_some()
                            && state.interaction.focused != focus_id
                        {
                            let old_focused = state.interaction.focused;
                            state.interaction.focused = focus_id;
                            state.interaction.focus_via_keyboard = false;
                            update_focus_context(state);
                            state.mark_restyle_pseudo_change(old_focused, focus_id);
                        }
                    }

                    let style_work = subtree_has_dirty_flags(
                        &state.arena,
                        state.root,
                        DirtyFlags::STYLE | DirtyFlags::SUBTREE_STYLE,
                    );
                    if style_work {
                        let t1 = Instant::now();
                        resolve_all_styles_with_transitions(
                            &mut state.arena,
                            &state.stylesheet,
                            state.root,
                            state.interaction.hovered,
                            state.interaction.active,
                            state.interaction.focused,
                            state.interaction.focus_via_keyboard,
                            Some(frame_start),
                            Some(&mut state.active_transitions),
                        );
                        unshit_core::build::resolve_pseudo_elements(
                            &mut state.arena,
                            &mut state.taffy,
                            &state.stylesheet,
                            state.root,
                            state.interaction.hovered,
                            state.interaction.active,
                            state.interaction.focused,
                            &mut state.pseudo_table,
                        );
                        metrics.style_resolve_us = t1.elapsed().as_micros() as u64;

                        let t2 = Instant::now();
                        scale_all_styles(&mut state.arena, state.root, state.scale_factor);
                        metrics.scale_us = t2.elapsed().as_micros() as u64;

                        mark_layout_dirty(&mut state.arena, state.root);
                    }

                    let layout_work = subtree_has_dirty_flags(
                        &state.arena,
                        state.root,
                        DirtyFlags::LAYOUT | DirtyFlags::SUBTREE_LAYOUT | DirtyFlags::CHILDREN,
                    );
                    if layout_work {
                        let t3 = Instant::now();
                        let (w, h) = state.gpu.window_size();
                        run_layout_pipeline(
                            &mut state.arena,
                            &mut state.taffy,
                            state.root,
                            &mut state.font_system,
                            w,
                            h,
                            &mut state.measure_cache,
                        );
                        metrics.layout_us = t3.elapsed().as_micros() as u64;
                    }

                    metrics.node_count = state.arena.len();
                    state.needs_rebuild = false;
                    state.needs_restyle = false;
                    state.needs_relayout = false;
                    // Full rebuild walked from root, so any narrowed
                    // `restyle_root` from a prior hover/focus/active
                    // change is now irrelevant. Drop it so the next
                    // restyle sees a clean slate.
                    state.restyle_root = None;

                    // Reconcile subscriptions after each rebuild.
                    #[cfg(feature = "async")]
                    if let Some(ref mut mgr) = self.app.subscription_manager {
                        let sink = EventSink::new(
                            self.app.event_tx.clone(),
                            Arc::clone(&self.app.proxy_cell),
                        );
                        mgr.reconcile(self.app.runtime.handle(), &sink);
                    }
                } else if state.needs_restyle {
                    // Pseudo-class state changes (hover / focus / active)
                    // narrow `restyle_root` to the LCA of the leaving and
                    // entering nodes. At non-1.0 scale factors, however,
                    // inherited values outside that subtree have already
                    // been scaled in-place, so a narrow cascade can inherit
                    // scaled font metrics and then scale them again. Use a
                    // full cascade when scaling is active.
                    let cascade_root = cascade_root_for_restyle(
                        state.restyle_root.take(),
                        state.root,
                        state.scale_factor,
                    );
                    let t1 = Instant::now();
                    resolve_all_styles_with_transitions(
                        &mut state.arena,
                        &state.stylesheet,
                        cascade_root,
                        state.interaction.hovered,
                        state.interaction.active,
                        state.interaction.focused,
                        state.interaction.focus_via_keyboard,
                        Some(frame_start),
                        Some(&mut state.active_transitions),
                    );
                    unshit_core::build::resolve_pseudo_elements(
                        &mut state.arena,
                        &mut state.taffy,
                        &state.stylesheet,
                        cascade_root,
                        state.interaction.hovered,
                        state.interaction.active,
                        state.interaction.focused,
                        &mut state.pseudo_table,
                    );
                    metrics.style_resolve_us = t1.elapsed().as_micros() as u64;

                    // Scale only the subtree we just re-resolved.
                    // `scale_all_styles` mutates `computed_style` in place,
                    // so calling it from the document root would compound
                    // the scale onto already-scaled nodes outside the LCA
                    // every restyle. Visible as runaway font / layout
                    // sizes after a few hover changes on HiDPI displays.
                    let t2 = Instant::now();
                    scale_all_styles(&mut state.arena, cascade_root, state.scale_factor);
                    metrics.scale_us = t2.elapsed().as_micros() as u64;

                    mark_layout_dirty(&mut state.arena, state.root);

                    let t3 = Instant::now();
                    let (w, h) = state.gpu.window_size();
                    run_layout_pipeline(
                        &mut state.arena,
                        &mut state.taffy,
                        state.root,
                        &mut state.font_system,
                        w,
                        h,
                        &mut state.measure_cache,
                    );
                    metrics.layout_us = t3.elapsed().as_micros() as u64;

                    metrics.node_count = state.arena.len();
                    state.needs_restyle = false;
                    apply_cursor_icon(&*state.window, &state.arena, state.interaction.hovered);
                } else if state.needs_relayout {
                    let t3 = Instant::now();
                    let (w, h) = state.gpu.window_size();
                    let resize_callbacks_fired = relayout_pipeline(
                        &mut state.arena,
                        &mut state.taffy,
                        state.root,
                        &mut state.font_system,
                        w,
                        h,
                        &mut state.measure_cache,
                    );
                    metrics.layout_us = t3.elapsed().as_micros() as u64;

                    metrics.node_count = state.arena.len();
                    state.needs_relayout = false;
                    if resize_callbacks_fired {
                        state.needs_rebuild = true;
                        state.window.request_redraw();
                    }
                } else {
                    metrics.node_count = state.arena.len();
                }

                // Sync keyframe animations into the driver side table after
                // every restyle pass so newly matched `animation:` rules
                // start ticking on the next frame.
                sync_all_animations(
                    &state.arena,
                    &mut state.animation_driver,
                    state.root,
                    frame_start,
                );

                // Tick active transitions: interpolate values and apply to styles.
                if state.active_transitions.has_active() {
                    tick_all_transitions(
                        &mut state.arena,
                        &mut state.active_transitions,
                        frame_start,
                    );
                }

                // Tick keyframe animations. The driver owns its own side
                // table so this call runs in O(running_animations) regardless
                // of the arena size.
                if state.animation_driver.has_active() {
                    tick_all_animations(
                        &mut state.arena,
                        &mut state.animation_driver,
                        &state.stylesheet,
                        frame_start,
                    );
                }

                // Tick cursor blink for the focused element.
                {
                    let focused_id = state.interaction.focused;
                    if !focused_id.is_dangling() {
                        if let Some(el) = state.arena.get_mut(focused_id) {
                            // Sync cursor shape/rate from computed style
                            el.cursor_state.shape = el.computed_style.caret_shape;
                            el.cursor_state.blink_rate_ms = el.computed_style.caret_blink_rate;
                            el.cursor_state.tick(frame_start);
                        }
                    }
                }

                let t4 = Instant::now();
                state.gpu.layered_batch.clear();
                state.batch_cache.begin_frame();
                let mut rasterizer = Rasterizer {
                    swash: &mut state.swash_cache,
                    subpixel_swash: &mut state.subpixel_swash_cache,
                    #[cfg(target_os = "windows")]
                    dw: &state.dw_rasterizer,
                };
                batch::build_render_batch(
                    &state.arena,
                    state.root,
                    &mut state.gpu.layered_batch,
                    &mut state.gpu.glyph_atlas,
                    &mut state.font_system,
                    &mut rasterizer,
                    &mut state.measure_cache,
                    &mut state.shaped_cache,
                    &mut state.gpu.svg_cache,
                    &mut state.shape_cache,
                    state.interaction.text_selection.as_ref(),
                    Some(&state.canvas_registry),
                    &state.scrollbar_visual,
                    state.interaction.focused,
                    &mut state.batch_cache,
                    Some(&mut state.line_quad_cache),
                );
                state.batch_cache.commit_frame();
                // Atlas generation is passed so `ShapedTextCache` can detect
                // coarse atlas residency changes between frames on top of the
                // per-glyph check in `emit_shaped_text_run`.
                state.shaped_cache.finish_frame(state.gpu.glyph_atlas.generation);
                state.shape_cache.finish_frame();
                batch::clear_paint_flags_subtree(&mut state.arena, state.root);
                metrics.batch_build_us = t4.elapsed().as_micros() as u64;

                // One-shot: fire on_cell_metrics once valid metrics are available.
                // publish_cell_metrics is called inside build_render_batch, so
                // the globals are populated by the time we reach this point.
                if !state.cell_metrics_fired {
                    let cw = unshit_core::cell_grid::CellGrid::global_cell_w();
                    let ch = unshit_core::cell_grid::CellGrid::global_cell_h();
                    if cw > 0.0 && ch > 0.0 {
                        state.cell_metrics_fired = true;
                        if let Some(ref cb) = self.app.config.on_cell_metrics {
                            cb(cw, ch);
                        }
                    }
                }

                // Collect quad/glyph counts from all layers.
                {
                    let mut total_quads: u32 = 0;
                    let mut total_glyphs: u32 = 0;
                    for layer in &state.gpu.layered_batch.layers {
                        total_quads = total_quads.saturating_add(layer.quad_instances.len() as u32);
                        total_glyphs =
                            total_glyphs.saturating_add(layer.glyph_instances.len() as u32);
                    }
                    metrics.quad_count = total_quads;
                    metrics.glyph_count = total_glyphs;
                }

                // Compute atlas fill ratio from next occupied row vs total atlas height.
                metrics.atlas_fill_ratio = if state.gpu.glyph_atlas.size > 0 {
                    state.gpu.glyph_atlas.next_shelf_y as f32 / state.gpu.glyph_atlas.size as f32
                } else {
                    0.0
                };

                // Estimate GPU upload bytes from pending glyph uploads.
                metrics.gpu_upload_bytes = state
                    .gpu
                    .glyph_atlas
                    .pending_uploads
                    .iter()
                    .map(|g| (g.width * g.height) as u64)
                    .sum();

                // Select widget overlays (label text and open dropdown panels)
                {
                    let (vw, vh) = state.gpu.window_size();
                    let mut rasterizer2 = Rasterizer {
                        swash: &mut state.swash_cache,
                        subpixel_swash: &mut state.subpixel_swash_cache,
                        #[cfg(target_os = "windows")]
                        dw: &state.dw_rasterizer,
                    };
                    batch::emit_select_overlays(
                        &state.arena,
                        state.root,
                        &mut state.gpu.layered_batch,
                        &mut state.gpu.glyph_atlas,
                        &mut state.font_system,
                        &mut rasterizer2,
                        &mut state.shaped_cache,
                        vw,
                        vh,
                    );
                }

                // Chord indicator overlay: show a teal pill at bottom-center
                if state.shortcut_resolver.is_chord_pending() {
                    let (vw, vh) = state.gpu.window_size();
                    let pill_w = 160.0;
                    let pill_h = 32.0;
                    let pill_x = (vw - pill_w) / 2.0;
                    let pill_y = vh - pill_h - 16.0;

                    state.gpu.layered_batch.layer_mut(Layer::Overlay).quad_instances.push(
                        QuadInstance {
                            pos: [pill_x, pill_y],
                            size: [pill_w, pill_h],
                            color: [0.06, 0.72, 0.50, 0.85],
                            border_color: [0.26, 0.90, 0.70, 0.40],
                            border_width: [1.0, 1.0, 1.0, 1.0],
                            border_radius: [16.0, 16.0, 16.0, 16.0],
                            clip_rect: [0.0, 0.0, vw, vh],
                            shadow_color: [0.0, 0.0, 0.0, 0.0],
                            shadow_offset: [0.0, 0.0],
                            shadow_params: [0.0, 0.0],
                            shadow_spread: [0.0, 0.0],
                            gradient_stop_colors: [[0.0; 4];
                                unshit_renderer::pipeline::quad::MAX_GRADIENT_STOPS],
                            gradient_stop_positions: [0.0;
                                unshit_renderer::pipeline::quad::MAX_GRADIENT_STOPS],
                            gradient_params: [0.0, 0.0, 0.0, 0.0],
                            gradient_extra: [0.0, 0.0, 0.0, 0.0],
                            mask_stops_01: [0.0, 0.0, 0.0, 0.0],
                            mask_stops_23: [0.0, 0.0, 0.0, 0.0],
                            mask_params: [0.0, 0.0, 0.0, 0.0],
                            xform: [0.0; 4],
                            xform_translate: [0.0; 2],
                        },
                    );
                }

                let t5 = Instant::now();
                state.window.pre_present_notify();
                state.gpu.render();
                // Split the vblank wait out of the work numbers at the
                // source so every downstream consumer of gpu_render_us /
                // total_us keeps measuring CPU work (see
                // FrameMetrics::present_wait_us).
                metrics.present_wait_us = state.gpu.last_acquire_wait().as_micros() as u64;
                metrics.gpu_render_us =
                    (t5.elapsed().as_micros() as u64).saturating_sub(metrics.present_wait_us);

                if state.gpu.any_canvas_needs_repaint() {
                    state.window.request_redraw();
                }

                // Unified wake-time calculation covering all animation sources:
                // cursor blink, CSS keyframe animations, and CSS transitions.
                // When any source is active we set WaitUntil to the minimum
                // wake time across all sources, so the event loop sleeps
                // between frames instead of busy-polling.
                if animations_active(state) {
                    schedule_animation_followup(state, event_loop, frame_start, true);
                } else {
                    let mut next_wake: Option<Instant> = None;

                    // Cursor blink.
                    let focused_id = state.interaction.focused;
                    if !focused_id.is_dangling() {
                        if let Some(el) = state.arena.get(focused_id) {
                            if let Some(next_toggle) = el.cursor_state.next_toggle_time() {
                                next_wake = Some(next_toggle);
                            }
                        }
                    }

                    // CSS keyframe animations.
                    if let Some(driver_wake) = state.animation_driver.next_wake(frame_start) {
                        next_wake = Some(match next_wake {
                            Some(current) if current <= driver_wake => current,
                            _ => driver_wake,
                        });
                    }

                    // CSS transitions: schedule WaitUntil instead of
                    // request_redraw so the thread sleeps between frames.
                    if let Some(transition_wake) = ActiveTransitions::next_wake(
                        &state.arena,
                        &state.active_transitions,
                        frame_start,
                    ) {
                        next_wake = Some(match next_wake {
                            Some(current) if current <= transition_wake => current,
                            _ => transition_wake,
                        });
                    }

                    if let Some(wake) = next_wake {
                        event_loop
                            .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(wake));
                        if wake <= Instant::now() {
                            state.window.request_redraw();
                        }
                    }
                }

                // Shared paint epilogue: pacer bookkeeping, metric
                // finalization, frame probes, slow-frame logging, the
                // on_frame_metrics callback, and the window-title fps
                // rollover. See [`finalize_frame_metrics`].
                finalize_frame_metrics(
                    state,
                    metrics,
                    frame_start,
                    self.app.config.on_frame_metrics.as_deref(),
                );

                // Close the input latency frame window only on the path
                // that actually rendered. Pacer skipped frames bailed
                // out well above; they never reach this line.
                #[cfg(feature = "input-latency-histogram")]
                {
                    state.input_latency.record_frame_presented(Instant::now());
                    if let Some(ref cb) = self.app.config.on_input_latency {
                        cb(&state.input_latency.snapshot());
                    }
                }

                // Reset the per-frame bump arena now that the tree has
                // been fully consumed by reconcile and the render batch
                // has been submitted. O(1) pointer reset; preserves
                // chunk capacity for the next frame. Safe to call even
                // when no bump tree was built this frame.
                state.frame_arena.reset();
            }

            _ => {}
        }
    }

    /// Fires right before the event loop blocks for the next event. Two
    /// jobs, both ending in either a queued redraw or a true sleep:
    ///
    /// Speculative repainting: while the window has been "recently
    /// active" (any external event within the last [`ACTIVITY_WINDOW`])
    /// and no animation is chaining its own frames, keep painting so a
    /// PTY chunk or keystroke landing mid-frame reaches the screen on
    /// the very next refresh — the Ghostty `DRAW_INTERVAL` pattern. In
    /// [`PacingMode::VsyncBlocking`] (the default) the redraw is queued
    /// unconditionally and the blocking swapchain acquire paces the
    /// resulting paint loop at the refresh rate; in [`PacingMode::Timer`]
    /// the pacer's speculative deadline schedules the wake instead.
    ///
    /// Wake hygiene: convert an expired `WaitUntil` into the redraw it
    /// was armed for (this is how parked frames — a skipped animation
    /// tick, a cursor-blink deadline — resume) and reset stray `Poll`s
    /// so idle CPU returns to ~zero. Runs regardless of the activity
    /// window because skipped animation frames park mid-interaction,
    /// well inside it.
    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Some(state) = self.app.state.as_mut() else {
            return;
        };

        let now = Instant::now();
        if state.pacing_mode == PacingMode::Timer {
            if animations_active(state) && !state.window.is_minimized().unwrap_or(false) {
                // Pace the animation chain at the due-gate. Requesting
                // the redraw while the gate is still closed would wake
                // the OS wait immediately (a queued internal paint
                // counts as input), so the redraw is only issued once
                // the gate has opened.
                let due = state.animation_next_frame.unwrap_or(now);
                if now < due {
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
                } else {
                    state.window.request_redraw();
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
                }
                return;
            }

            if state.is_recently_active(now) {
                // Pick the earlier of (speculative pacer deadline, any
                // wake the paint handler already set). Taking the min
                // means a cursor-blink or transition wake-up that happens
                // to fall before the speculative deadline still fires on
                // time; the speculative deadline is what drives the paint
                // rate during typing / PTY bursts.
                let spec_deadline = state.frame_pacer.speculative_deadline(now);
                let deadline = match event_loop.control_flow() {
                    winit::event_loop::ControlFlow::WaitUntil(prev) if prev < spec_deadline => prev,
                    _ => spec_deadline,
                };
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
                if deadline <= now {
                    state.window.request_redraw();
                }
                return;
            }
        } else if !animations_active(state)
            && state.is_recently_active(now)
            && !state.window.is_minimized().unwrap_or(false)
        {
            // VsyncBlocking speculative repaint (the Ghostty
            // DRAW_INTERVAL pattern, re-anchored from a wall-clock timer
            // to the display itself): while the window is recently
            // active, keep one redraw queued so the loop paints every
            // vblank — each paint's blocking acquire IS the pacing — and
            // a PTY chunk or keystroke landing mid-frame reaches the
            // screen on the very next refresh. Paints on clean state are
            // cheap (the renderer short-circuits through its caches).
            // Excluded while animations run: their painted frames chain
            // their own redraws, and their *skipped* frames deliberately
            // park on a WaitUntil that an unconditional redraw here would
            // defeat, spinning the tick loop at CPU speed. Excluded while
            // minimized: acquires fail instead of blocking, so this loop
            // would also spin.
            state.window.request_redraw();
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            return;
        }

        // Wake hygiene (both modes). A
        // stale `WaitUntil` (already in the past) would spin-wake winit
        // on every iteration; convert it into the redraw it was armed
        // for and a true sleep. Future-valued deadlines (cursor blink,
        // CSS transitions, parked animation resumes) are left alone.
        match event_loop.control_flow() {
            winit::event_loop::ControlFlow::WaitUntil(deadline) if deadline <= now => {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
                state.window.request_redraw();
            }
            winit::event_loop::ControlFlow::Poll => {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            }
            _ => {}
        }
    }
}

/// Update highlighted_index for any open select dropdown based on cursor position.
fn update_select_hover(state: &mut AppState, px: f32, py: f32) {
    let (_, vh) = state.gpu.window_size();
    let open_selects: Vec<NodeId> = state
        .arena
        .iter()
        .filter_map(|(id, el)| {
            if el.tag == Tag::Select {
                el.select_state.as_ref().and_then(|ss| if ss.open { Some(id) } else { None })
            } else {
                None
            }
        })
        .collect();
    for select_id in open_selects {
        let dropdown = select_dropdown_rect(&state.arena, select_id, vh);
        if let Some((dx, dy, dw, dh)) = dropdown {
            let (item_h, opt_len) = {
                let el = state.arena.get(select_id).unwrap();
                let ss = el.select_state.as_ref().unwrap();
                let style = &el.computed_style;
                (select_item_h(style.font_size.max(10.0), style.line_height), ss.options.len())
            };
            let new_hi = if px >= dx && px <= dx + dw && py >= dy && py <= dy + dh {
                let idx = ((py - dy) / item_h).floor() as usize;
                if idx < opt_len {
                    Some(idx as u32)
                } else {
                    None
                }
            } else {
                None
            };
            let el = state.arena.get_mut(select_id).unwrap();
            if let Some(ref mut ss) = el.select_state {
                if ss.highlighted_index != new_hi {
                    ss.highlighted_index = new_hi;
                    state.window.request_redraw();
                }
            }
        }
    }
}

/// Normal hover/selection handling extracted from PointerMoved, so it can be
/// called from both the "no drag" path and the "threshold not met yet" path.
fn handle_normal_hover(state: &mut AppState, pos: (f32, f32), decorations: bool) {
    if let Some(direction) = custom_window_resize_direction(
        decorations,
        state.window.surface_size(),
        pos,
        state.window.scale_factor() as f32,
    ) {
        state.window.set_cursor(resize_direction_cursor_icon(direction).into());
        if !state.interaction.hovered.is_dangling() {
            let old_hover = state.interaction.hovered;
            state.interaction.hovered = NodeId::DANGLING;
            state.mark_restyle_pseudo_change(old_hover, NodeId::DANGLING);
            state.window.request_redraw();
        }
        return;
    }

    // Check scrollbar hover
    let sb_hit = scroll::find_scrollbar_at(&state.arena, state.root, pos.0, pos.1);
    let old_visual = state.scrollbar_visual;
    state.scrollbar_visual.set_hover(sb_hit.as_ref());
    if state.scrollbar_visual != old_visual {
        state.window.request_redraw();
    }

    let new_hover = hit_test(&state.arena, state.root, pos.0, pos.1).unwrap_or(NodeId::DANGLING);

    if new_hover != state.interaction.hovered {
        let old_hover = state.interaction.hovered;
        state.interaction.hovered = new_hover;
        apply_cursor_icon(&*state.window, &state.arena, new_hover);
        state.mark_restyle_pseudo_change(old_hover, new_hover);
        let (local_x, local_y) = local_pointer_coords(&state.arena, new_hover, pos.0, pos.1);
        let event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Move,
            x: pos.0,
            y: pos.1,
            local_x,
            local_y,
            button: MouseButton::None,
            modifiers: Modifiers::empty(),
        });
        if dispatch_mouse_move_event(&state.arena, new_hover, &event) {
            state.needs_rebuild = true;
        }
        state.window.request_redraw();
    }

    // Update select dropdown item highlighting on hover
    update_select_hover(state, pos.0, pos.1);

    // Extend text selection while dragging
    if state.interaction.selecting {
        if let Some((text_node, byte_offset)) = layout::nearest_text_hit_at(
            &state.arena,
            state.root,
            pos.0,
            pos.1,
            &mut state.font_system,
        ) {
            if let Some(ref mut sel) = state.interaction.text_selection {
                if sel.focus_element != text_node || sel.focus_offset != byte_offset {
                    sel.focus_element = text_node;
                    sel.focus_offset = byte_offset;
                    state.window.request_redraw();
                }
            }
        }
    }
}

fn dispatch_mouse_move_event(arena: &NodeArena, start: NodeId, event: &Event) -> bool {
    let mut node = start;
    while !node.is_dangling() {
        let Some(element) = arena.get(node) else {
            break;
        };
        for (event_type, handler) in &element.handlers {
            if *event_type == EventType::MouseMove {
                handler(event);
                return true;
            }
        }
        node = element.parent;
    }
    false
}

/// Cursor position relative to a node's content box (padding box origin),
/// in the same coordinate space the renderer lays cells out in. Falls back
/// to the raw window coordinates when the node is missing so callers always
/// get a usable value. Mirrors the built-in text-input hit-test convention
/// (`pos - rect - padding`) so grid/canvas handlers can divide by the
/// published cell metrics to recover a cell index.
fn local_pointer_coords(arena: &NodeArena, node: NodeId, x: f32, y: f32) -> (f32, f32) {
    if let Some(el) = arena.get(node) {
        let r = el.layout_rect;
        let p = &el.computed_style.padding;
        (x - r.x - p.left, y - r.y - p.top)
    } else {
        (x, y)
    }
}

/// Walk up from `start` to find the nearest element with a handler for
/// `event_type` (used for `MouseDown`/`MouseUp`), invoke it, and report
/// whether one fired. Mirrors [`dispatch_mouse_move_event`] but matches an
/// arbitrary button event type so press/release reach grid handlers that
/// need the press coordinates and modifiers a `Click` cannot carry.
fn dispatch_mouse_button_event(
    arena: &NodeArena,
    start: NodeId,
    event_type: EventType,
    event: &Event,
) -> bool {
    let mut node = start;
    while !node.is_dangling() {
        let Some(element) = arena.get(node) else {
            break;
        };
        for (et, handler) in &element.handlers {
            if *et == event_type {
                handler(event);
                return true;
            }
        }
        node = element.parent;
    }
    false
}

/// Convert the renderer's tracked winit modifier state into the framework's
/// `Modifiers` bitflags so pointer events can carry the live modifier set.
fn modifiers_from_winit(state: &winit::keyboard::ModifiersState) -> Modifiers {
    let mut m = Modifiers::empty();
    if state.shift_key() {
        m |= Modifiers::SHIFT;
    }
    if state.control_key() {
        m |= Modifiers::CTRL;
    }
    if state.alt_key() {
        m |= Modifiers::ALT;
    }
    if state.meta_key() {
        m |= Modifiers::META;
    }
    m
}

fn update_focus_context(state: &mut AppState) {
    state.shortcut_resolver.set_context("inputFocused", false);
    state.shortcut_resolver.set_context("buttonFocused", false);
    state.shortcut_resolver.set_context("selectFocused", false);

    let focused = state.interaction.focused;
    if let Some(element) = state.arena.get_mut(focused) {
        // Reset cursor blink to visible on focus gain
        element.cursor_state.reset_blink(Instant::now());

        match element.tag {
            Tag::Input => {
                state.shortcut_resolver.set_context("inputFocused", true);
            }
            Tag::Button => {
                state.shortcut_resolver.set_context("buttonFocused", true);
            }
            Tag::Select => {
                state.shortcut_resolver.set_context("selectFocused", true);
            }
            _ => {}
        }
    }
}

/// Handle a click on an Input element. Returns `true` if the click was
/// consumed (checkbox/radio toggled). Does nothing for other element types.
fn handle_input_click(state: &mut AppState, target: NodeId) -> bool {
    use unshit_core::element::{InputType, Tag};
    let Some(element) = state.arena.get(target) else { return false };
    if element.tag != Tag::Input {
        return false;
    }
    match element.input_state.input_type {
        InputType::Checkbox => {
            let (new_checked, on_change) = {
                let elem = state.arena.get_mut(target).unwrap();
                elem.input_state.checked = !elem.input_state.checked;
                (elem.input_state.checked, elem.on_change.clone())
            };
            if let Some(f) = on_change {
                f(if new_checked { "true" } else { "false" });
            }
            true
        }
        InputType::Radio => {
            let already_checked =
                state.arena.get(target).map(|e| e.input_state.checked).unwrap_or(false);
            if !already_checked {
                let radio_name = state.arena.get(target).and_then(|e| e.name.clone());
                check_radio(state, target, radio_name.as_deref());
            }
            true
        }
        _ => false,
    }
}

fn handle_text_input(state: &mut AppState, event: &winit::event::KeyEvent) -> bool {
    use unshit_core::element::InputType;
    use winit::keyboard::Key as WinitKey;
    use winit::keyboard::NamedKey;

    let focused = state.interaction.focused;

    // Determine input type of the focused element.
    let input_type =
        state.arena.get(focused).map(|e| e.input_state.input_type).unwrap_or(InputType::Text);

    // Handle Space key for checkbox/radio toggling.
    // In winit, Space is WinitKey::Character(" ") not a NamedKey.
    let is_space = matches!(&event.logical_key, WinitKey::Character(c) if c.as_str() == " ")
        || matches!(&event.text, Some(t) if t.as_str() == " ");
    if is_space {
        match input_type {
            InputType::Checkbox => {
                let (new_checked, on_change) = {
                    let element = state.arena.get_mut(focused).unwrap();
                    element.input_state.checked = !element.input_state.checked;
                    (element.input_state.checked, element.on_change.clone())
                };
                if let Some(f) = on_change {
                    f(if new_checked { "true" } else { "false" });
                }
                state.needs_rebuild = true;
                state.window.request_redraw();
                return true;
            }
            InputType::Radio => {
                // Only check; radio buttons cannot be unchecked by Space.
                let already_checked =
                    state.arena.get(focused).map(|e| e.input_state.checked).unwrap_or(false);
                if !already_checked {
                    let radio_name = state.arena.get(focused).and_then(|e| e.name.clone());
                    check_radio(state, focused, radio_name.as_deref());
                }
                return true;
            }
            // Other types: fall through.
            _ => {}
        }
    }

    // Range, checkbox, radio, and hidden do not accept text or most key events.
    match input_type {
        InputType::Range | InputType::Checkbox | InputType::Radio | InputType::Hidden => {
            // Range arrow keys handled through apply_key below.
            if input_type == InputType::Range {
                if let WinitKey::Named(named) = &event.logical_key {
                    let key = match named {
                        NamedKey::ArrowLeft => Some(unshit_core::event::Key::ArrowLeft),
                        NamedKey::ArrowRight => Some(unshit_core::event::Key::ArrowRight),
                        NamedKey::ArrowUp => Some(unshit_core::event::Key::ArrowUp),
                        NamedKey::ArrowDown => Some(unshit_core::event::Key::ArrowDown),
                        _ => None,
                    };
                    if let Some(k) = key {
                        let (changed, new_value, on_change) = {
                            let element = state.arena.get_mut(focused).unwrap();
                            let old_nv = element.input_state.numeric_value;
                            unshit_core::input::apply_key(&mut element.input_state, &k);
                            let diff = element.input_state.numeric_value != old_nv;
                            let nv = element.input_state.value.clone();
                            let cb = element.on_change.clone();
                            (diff, nv, cb)
                        };
                        if changed {
                            if let Some(f) = on_change {
                                f(&new_value);
                            }
                        }
                        state.window.request_redraw();
                        return true;
                    }
                }
            }
            return false;
        }
        _ => {}
    }

    // Try character input first (from event.text)
    if let Some(ref text) = event.text {
        let text = text.as_str();
        if !text.is_empty()
            && !text.chars().all(char::is_control)
            && !state.modifiers_state.control_key()
            && !state.modifiers_state.meta_key()
        {
            let (old_value, new_value, accepted, on_change) = {
                let element = state.arena.get_mut(focused).unwrap();
                let old = element.input_state.value.clone();
                let accepted =
                    unshit_core::input::insert_text_filtered(&mut element.input_state, text);
                element.cursor_state.reset_blink(Instant::now());
                let new = element.input_state.value.clone();
                let cb = element.on_change.clone();
                (old, new, accepted, cb)
            };

            if accepted && new_value != old_value {
                if let Some(f) = on_change {
                    f(&new_value);
                }
            }
            if accepted {
                state.needs_relayout = true;
                state.shaped_cache.clear();
                state.batch_cache.clear();
                state.window.request_redraw();
            }
            return accepted;
        }
    }

    // Handle named keys
    if let WinitKey::Named(named) = &event.logical_key {
        let key = match named {
            NamedKey::Backspace => Some(unshit_core::event::Key::Backspace),
            NamedKey::Delete => Some(unshit_core::event::Key::Delete),
            NamedKey::Insert => Some(unshit_core::event::Key::Insert),
            NamedKey::ArrowLeft => Some(unshit_core::event::Key::ArrowLeft),
            NamedKey::ArrowRight => Some(unshit_core::event::Key::ArrowRight),
            NamedKey::ArrowUp => Some(unshit_core::event::Key::ArrowUp),
            NamedKey::ArrowDown => Some(unshit_core::event::Key::ArrowDown),
            NamedKey::Home => Some(unshit_core::event::Key::Home),
            NamedKey::End => Some(unshit_core::event::Key::End),
            NamedKey::Enter => {
                // Clamp number inputs on Enter.
                if input_type == InputType::Number {
                    let element_mut = state.arena.get_mut(focused).unwrap();
                    unshit_core::input::clamp_number_input(&mut element_mut.input_state);
                }
                let element = state.arena.get(focused).unwrap();
                let on_submit = element.on_submit.clone();
                let value = element.input_state.value.clone();
                if let Some(f) = on_submit {
                    f(&value);
                }
                state.needs_relayout = true;
                state.window.request_redraw();
                return true;
            }
            // Let Tab and Escape fall through to shortcuts
            NamedKey::Tab | NamedKey::Escape => return false,
            _ => None,
        };

        if let Some(k) = key {
            let (changed, new_value, on_change) = {
                let element = state.arena.get_mut(focused).unwrap();
                let old = element.input_state.value.clone();
                unshit_core::input::apply_key(&mut element.input_state, &k);
                element.cursor_state.reset_blink(Instant::now());
                let diff = element.input_state.value != old;
                let nv = element.input_state.value.clone();
                let cb = element.on_change.clone();
                (diff, nv, cb)
            };

            if changed {
                if let Some(f) = on_change {
                    f(&new_value);
                }
                state.shaped_cache.clear();
                state.batch_cache.clear();
            }
            state.needs_relayout = true;
            state.window.request_redraw();
            return true;
        }
    }

    false
}

/// Uncheck all radio buttons in the same named group, then check the target.
fn check_radio(state: &mut AppState, target: NodeId, name: Option<&str>) {
    use unshit_core::element::InputType;
    // Collect all radio inputs with the same name.
    let siblings: Vec<NodeId> = state
        .arena
        .iter()
        .filter(|(id, elem)| {
            *id != target
                && elem.tag == unshit_core::element::Tag::Input
                && elem.input_state.input_type == InputType::Radio
                && name.is_some()
                && elem.name.as_deref() == name
        })
        .map(|(id, _)| id)
        .collect();

    for sid in siblings {
        if let Some(elem) = state.arena.get_mut(sid) {
            elem.input_state.checked = false;
        }
    }
    let on_change = if let Some(elem) = state.arena.get_mut(target) {
        elem.input_state.checked = true;
        elem.on_change.clone()
    } else {
        None
    };
    if let Some(f) = on_change {
        f("true");
    }
    state.needs_rebuild = true;
    state.window.request_redraw();
}

/// Handle Ctrl+C, Ctrl+V, Ctrl+X clipboard shortcuts when a text input is focused.
/// Returns `true` if the event was consumed.
fn handle_clipboard_shortcut(
    state: &mut AppState,
    event: &winit::event::KeyEvent,
    clipboard: &Arc<ClipboardContext>,
) -> bool {
    use winit::keyboard::Key as WinitKey;

    let focused = state.interaction.focused;

    let char_key = match &event.logical_key {
        WinitKey::Character(c) => {
            let s = c.as_str().to_ascii_lowercase();
            if s.len() == 1 {
                s.chars().next()
            } else {
                None
            }
        }
        _ => None,
    };

    match char_key {
        Some('c') => {
            // Copy: get selected text from focused input and write to clipboard
            if let Some(ref sel) = state.interaction.text_selection {
                if let Some((start, end)) = sel.single_element_range() {
                    if let Some(element) = state.arena.get(focused) {
                        let text = &element.input_state.value;
                        if start < text.len() && end <= text.len() && start < end {
                            let selected = &text[start..end];
                            if let Err(e) = clipboard.write_text(selected) {
                                log::warn!("Clipboard copy failed: {}", e);
                            }
                        }
                    }
                }
            }
            true
        }
        Some('x') => {
            // Cut: copy selected text to clipboard, then delete the selection
            if let Some(ref sel) = state.interaction.text_selection.clone() {
                if let Some((start, end)) = sel.single_element_range() {
                    let (cut_text, new_value, on_change) = {
                        let element = state.arena.get_mut(focused).unwrap();
                        let text = &element.input_state.value;
                        if start < text.len() && end <= text.len() && start < end {
                            let selected = text[start..end].to_string();
                            element.input_state.value =
                                format!("{}{}", &text[..start], &text[end..]);
                            element.input_state.cursor_pos = start;
                            let nv = element.input_state.value.clone();
                            let cb = element.on_change.clone();
                            (Some(selected), nv, cb)
                        } else {
                            (None, String::new(), None)
                        }
                    };

                    if let Some(selected) = cut_text {
                        if let Err(e) = clipboard.write_text(&selected) {
                            log::warn!("Clipboard cut failed: {}", e);
                        }
                        state.interaction.text_selection = None;
                        if let Some(f) = on_change {
                            f(&new_value);
                        }
                        state.shaped_cache.clear();
                        state.batch_cache.clear();
                        state.needs_relayout = true;
                        state.window.request_redraw();
                    }
                }
            }
            true
        }
        Some('v') => {
            // Paste: read text from clipboard and insert at cursor
            match clipboard.read_text() {
                Ok(text) if !text.is_empty() => {
                    // If there is a selection, delete it first
                    if let Some(ref sel) = state.interaction.text_selection.clone() {
                        if let Some((start, end)) = sel.single_element_range() {
                            if let Some(element) = state.arena.get_mut(focused) {
                                let val = &element.input_state.value;
                                if start < val.len() && end <= val.len() && start < end {
                                    element.input_state.value =
                                        format!("{}{}", &val[..start], &val[end..]);
                                    element.input_state.cursor_pos = start;
                                }
                            }
                            state.interaction.text_selection = None;
                        }
                    }

                    let (new_value, on_change) = {
                        let element = state.arena.get_mut(focused).unwrap();
                        unshit_core::input::insert_text(&mut element.input_state, &text);
                        element.cursor_state.reset_blink(Instant::now());
                        let nv = element.input_state.value.clone();
                        let cb = element.on_change.clone();
                        (nv, cb)
                    };

                    if let Some(f) = on_change {
                        f(&new_value);
                    }
                    state.shaped_cache.clear();
                    state.batch_cache.clear();
                    state.needs_relayout = true;
                    state.window.request_redraw();
                }
                Ok(_) => {} // Empty clipboard, nothing to paste
                Err(e) => {
                    log::warn!("Clipboard paste failed: {}", e);
                }
            }
            true
        }
        _ => false,
    }
}

fn dispatch_command(
    state: &mut AppState,
    command: &str,
    on_command: Option<&Arc<dyn Fn(&str) -> bool + Send + Sync>>,
) {
    match command {
        "focus.next" => {
            let new_focused = next_focusable(&state.arena, state.root, state.interaction.focused);
            if let Some(id) = new_focused {
                if id != state.interaction.focused {
                    let old_focused = state.interaction.focused;
                    state.interaction.focused = id;
                    state.interaction.focus_via_keyboard = true;
                    update_focus_context(state);
                    state.mark_restyle_pseudo_change(old_focused, id);
                    state.window.request_redraw();
                }
            }
        }
        "focus.prev" => {
            let new_focused = prev_focusable(&state.arena, state.root, state.interaction.focused);
            if let Some(id) = new_focused {
                if id != state.interaction.focused {
                    let old_focused = state.interaction.focused;
                    state.interaction.focused = id;
                    state.interaction.focus_via_keyboard = true;
                    update_focus_context(state);
                    state.mark_restyle_pseudo_change(old_focused, id);
                    state.window.request_redraw();
                }
            }
        }
        _ => {
            // Fall through to the user-supplied command handler. If the
            // handler reports that it consumed the command we schedule a
            // rebuild + redraw so any state mutation is reflected on screen.
            if let Some(handler) = on_command {
                if handler(command) {
                    state.needs_rebuild = true;
                    state.window.request_redraw();
                    return;
                }
            }
            log::debug!("Unhandled shortcut command: {}", command);
        }
    }
}

fn apply_cursor_icon(window: &dyn Window, arena: &NodeArena, hovered: NodeId) {
    use unshit_core::style::types::CursorStyle;
    let cursor = if !hovered.is_dangling() {
        arena
            .get(hovered)
            .map(|elem| match elem.computed_style.cursor {
                CursorStyle::Default => CursorIcon::Default,
                CursorStyle::None => CursorIcon::Default,
                CursorStyle::Pointer => CursorIcon::Pointer,
                CursorStyle::Text => CursorIcon::Text,
                CursorStyle::Grab => CursorIcon::Grab,
                CursorStyle::Grabbing => CursorIcon::Grabbing,
                CursorStyle::NotAllowed => CursorIcon::NotAllowed,
                CursorStyle::Crosshair => CursorIcon::Crosshair,
                CursorStyle::Move => CursorIcon::Move,
                CursorStyle::Wait => CursorIcon::Wait,
                CursorStyle::Help => CursorIcon::Help,
                CursorStyle::Progress => CursorIcon::Progress,
                CursorStyle::ColResize => CursorIcon::ColResize,
                CursorStyle::RowResize => CursorIcon::RowResize,
                CursorStyle::NResize => CursorIcon::NResize,
                CursorStyle::SResize => CursorIcon::SResize,
                CursorStyle::EResize => CursorIcon::EResize,
                CursorStyle::WResize => CursorIcon::WResize,
                CursorStyle::NeResize => CursorIcon::NeResize,
                CursorStyle::NwResize => CursorIcon::NwResize,
                CursorStyle::SeResize => CursorIcon::SeResize,
                CursorStyle::SwResize => CursorIcon::SwResize,
                CursorStyle::NsResize => CursorIcon::NsResize,
                CursorStyle::EwResize => CursorIcon::EwResize,
                CursorStyle::NeswResize => CursorIcon::NeswResize,
                CursorStyle::NwseResize => CursorIcon::NwseResize,
                CursorStyle::ZoomIn => CursorIcon::ZoomIn,
                CursorStyle::ZoomOut => CursorIcon::ZoomOut,
            })
            .unwrap_or(CursorIcon::Default)
    } else {
        CursorIcon::Default
    };
    window.set_cursor(cursor.into());
}

#[cfg(target_os = "windows")]
fn collect_directwrite_font_paths(
    config_fonts: &[crate::font::FontSource],
    stylesheet: &CompiledStylesheet,
) -> Vec<PathBuf> {
    use unshit_core::style::parse::FontFaceSrc;

    let mut paths = Vec::new();
    for source in config_fonts {
        if let crate::font::FontSource::Path(path) = source {
            paths.push(path.clone());
        }
    }
    for rule in &stylesheet.font_faces {
        if let FontFaceSrc::Url(url) = &rule.src {
            if !url.starts_with("data:") {
                paths.push(PathBuf::from(url));
            }
        }
    }
    paths
}

/// Lightweight re-layout: recompute taffy layout and positions without rebuilding tree or styles.
fn relayout_pipeline(
    arena: &mut NodeArena,
    taffy: &mut taffy::TaffyTree<TextMeasureCtx>,
    root: NodeId,
    font_system: &mut FontSystem,
    width: f32,
    height: f32,
    cache: &mut TextMeasureCache,
) -> bool {
    if let Some(tn) = arena.get(root).and_then(|e| e.taffy_node) {
        layout::compute_layout(taffy, tn, width, height, font_system, cache);
        layout::read_layout_results(arena, taffy, root, 0.0, 0.0);
        mark_paint_dirty(arena, root);
    }
    dispatch_resize_callbacks(arena, root)
}

/// Get current process RSS (resident set size) in bytes.
#[cfg(target_os = "windows")]
fn get_rss_bytes() -> usize {
    use std::mem;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessMemoryCounters {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }

    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, pmc: *mut ProcessMemoryCounters, cb: u32)
            -> i32;
    }

    // SAFETY: ProcessMemoryCounters is a repr(C) struct of plain integers, so zeroed
    // memory is a valid representation. GetCurrentProcess() returns a pseudo-handle that
    // does not need to be closed, and K32GetProcessMemoryInfo reads from a kernel-managed
    // buffer into our stack-allocated struct with the correct size passed via `cb`.
    unsafe {
        let mut pmc: ProcessMemoryCounters = mem::zeroed();
        pmc.cb = mem::size_of::<ProcessMemoryCounters>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
            pmc.WorkingSetSize
        } else {
            0
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn get_rss_bytes() -> usize {
    0
}

// ---------------------------------------------------------------------------
// Select widget interaction helpers
// ---------------------------------------------------------------------------

/// Compute the item height for a select dropdown, matching the renderer.
fn select_item_h(font_size: f32, line_height: f32) -> f32 {
    (font_size * line_height * 1.2).max(24.0)
}

/// Returns the dropdown panel rect (x, y, w, h) for an open select.
/// Mirrors the logic in `emit_select_node`.
fn select_dropdown_rect(
    arena: &NodeArena,
    node_id: NodeId,
    vh: f32,
) -> Option<(f32, f32, f32, f32)> {
    let element = arena.get(node_id)?;
    let ss = element.select_state.as_ref()?;
    if !ss.open || ss.options.is_empty() {
        return None;
    }
    let rect = element.layout_rect;
    let style = &element.computed_style;
    let item_h = select_item_h(style.font_size.max(10.0), style.line_height);
    let dropdown_h = item_h * ss.options.len() as f32;
    let dropdown_y = rect.y + rect.height;
    let actual_y =
        if dropdown_y + dropdown_h > vh { (rect.y - dropdown_h).max(0.0) } else { dropdown_y };
    Some((rect.x, actual_y, rect.width, dropdown_h))
}

/// Walk the arena and close all open select elements except `except_id`.
/// Returns true if any were closed.
fn close_all_selects_except(arena: &mut NodeArena, _root: NodeId, except_id: NodeId) -> bool {
    let to_close: Vec<NodeId> = arena
        .iter()
        .filter_map(|(id, el)| {
            if id != except_id && el.tag == Tag::Select {
                el.select_state.as_ref().and_then(|ss| if ss.open { Some(id) } else { None })
            } else {
                None
            }
        })
        .collect();
    let changed = !to_close.is_empty();
    for id in to_close {
        if let Some(el) = arena.get_mut(id) {
            if let Some(ref mut ss) = el.select_state {
                ss.open = false;
            }
        }
    }
    changed
}

/// Find the first open select element in the arena (if any) and check if
/// `(px, py)` hits any of its dropdown item rows. Returns `(select_id, item_index)`.
fn hit_test_select_dropdown(
    arena: &NodeArena,
    _root: NodeId,
    px: f32,
    py: f32,
    vh: f32,
) -> Option<(NodeId, usize)> {
    // Collect open selects first to avoid borrow issues
    let open_selects: Vec<NodeId> = arena
        .iter()
        .filter_map(|(id, el)| {
            if el.tag == Tag::Select {
                el.select_state.as_ref().and_then(|ss| {
                    if ss.open && !ss.options.is_empty() {
                        Some(id)
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
        .collect();

    for id in open_selects {
        if let Some((dx, dy, dw, dh)) = select_dropdown_rect(arena, id, vh) {
            if px >= dx && px <= dx + dw && py >= dy && py <= dy + dh {
                let (item_h, opt_len) = {
                    let el = arena.get(id)?;
                    let ss = el.select_state.as_ref()?;
                    let style = &el.computed_style;
                    (select_item_h(style.font_size.max(10.0), style.line_height), ss.options.len())
                };
                let idx = ((py - dy) / item_h).floor() as usize;
                if idx < opt_len {
                    return Some((id, idx));
                }
            }
        }
    }
    None
}

/// Handle a click for select widget logic. Called before normal click dispatch.
/// Returns true if the event was consumed by select logic.
fn handle_select_click(state: &mut AppState, px: f32, py: f32) -> bool {
    let (_, vh) = state.gpu.window_size();

    // Check if the click is on an open dropdown item
    if let Some((select_id, item_idx)) =
        hit_test_select_dropdown(&state.arena, state.root, px, py, vh)
    {
        // Select this option
        let (value, on_change) = {
            if let Some(el) = state.arena.get_mut(select_id) {
                if let Some(ref mut ss) = el.select_state {
                    ss.selected_index = item_idx as u32;
                    ss.open = false;
                    ss.highlighted_index = None;
                    let val = ss.options[item_idx].value.clone();
                    let cb = el.on_change.clone();
                    (val, cb)
                } else {
                    return false;
                }
            } else {
                return false;
            }
        };
        if let Some(f) = on_change {
            f(&value);
        }
        state.needs_rebuild = true;
        state.window.request_redraw();
        return true;
    }

    // Check if click is on a closed/open select element itself
    let hovered = state.interaction.hovered;
    if hovered.is_dangling() {
        // Click outside: close any open selects
        let changed = close_all_selects_except(&mut state.arena, state.root, NodeId::DANGLING);
        if changed {
            state.window.request_redraw();
        }
        return false;
    }

    // Walk up from hovered to find a select ancestor
    let mut cur = hovered;
    while !cur.is_dangling() {
        let tag = state.arena.get(cur).map(|e| e.tag);
        if tag == Some(Tag::Select) {
            break;
        }
        cur = state.arena.get(cur).map(|e| e.parent).unwrap_or(NodeId::DANGLING);
    }

    if cur.is_dangling() {
        // Not on a select; close any open selects (click outside)
        let changed = close_all_selects_except(&mut state.arena, state.root, NodeId::DANGLING);
        if changed {
            state.window.request_redraw();
        }
        return false;
    }

    let select_id = cur;

    // Toggle the select's open state
    if let Some(el) = state.arena.get_mut(select_id) {
        if let Some(ref mut ss) = el.select_state {
            ss.open = !ss.open;
            if ss.open {
                ss.highlighted_index = Some(ss.selected_index);
            } else {
                ss.highlighted_index = None;
            }
        }
    }

    // Close all other open selects
    close_all_selects_except(&mut state.arena, state.root, select_id);

    state.window.request_redraw();
    true
}

/// Handle keyboard events for a focused select element.
/// Returns true if the event was consumed.
fn handle_select_keyboard(state: &mut AppState, event: &winit::event::KeyEvent) -> bool {
    use winit::keyboard::Key as WinitKey;
    use winit::keyboard::NamedKey;

    let focused = state.interaction.focused;
    if focused.is_dangling() {
        return false;
    }

    let is_select = state.arena.get(focused).map(|e| e.tag == Tag::Select).unwrap_or(false);
    if !is_select {
        return false;
    }

    // Space is WinitKey::Character(" "), not NamedKey; handle it here.
    let is_space = matches!(&event.logical_key, WinitKey::Character(s) if s.as_str() == " ");

    if is_space {
        // Toggle open / confirm highlighted
        let (confirm_value, on_change) = {
            let el = state.arena.get_mut(focused).unwrap();
            if let Some(ref mut ss) = el.select_state {
                if ss.open {
                    let idx = ss.highlighted_index.unwrap_or(ss.selected_index) as usize;
                    let idx = idx.min(ss.options.len().saturating_sub(1));
                    ss.selected_index = idx as u32;
                    ss.open = false;
                    ss.highlighted_index = None;
                    let val = ss.options.get(idx).map(|o| o.value.clone()).unwrap_or_default();
                    (Some(val), el.on_change.clone())
                } else {
                    ss.open = true;
                    ss.highlighted_index = Some(ss.selected_index);
                    (None, None)
                }
            } else {
                (None, None)
            }
        };
        if let (Some(val), Some(f)) = (confirm_value, on_change) {
            f(&val);
            state.needs_rebuild = true;
        }
        state.window.request_redraw();
        return true;
    }

    let WinitKey::Named(named) = &event.logical_key else {
        return false;
    };

    match named {
        NamedKey::Enter => {
            // Confirm highlighted
            let (confirm_value, on_change) = {
                let el = state.arena.get_mut(focused).unwrap();
                if let Some(ref mut ss) = el.select_state {
                    if ss.open {
                        let idx = ss.highlighted_index.unwrap_or(ss.selected_index) as usize;
                        let idx = idx.min(ss.options.len().saturating_sub(1));
                        ss.selected_index = idx as u32;
                        ss.open = false;
                        ss.highlighted_index = None;
                        let val = ss.options.get(idx).map(|o| o.value.clone()).unwrap_or_default();
                        (Some(val), el.on_change.clone())
                    } else {
                        ss.open = true;
                        ss.highlighted_index = Some(ss.selected_index);
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            };
            if let (Some(val), Some(f)) = (confirm_value, on_change) {
                f(&val);
                state.needs_rebuild = true;
            }
            state.window.request_redraw();
            true
        }

        NamedKey::Escape => {
            let el = state.arena.get_mut(focused).unwrap();
            if let Some(ref mut ss) = el.select_state {
                if ss.open {
                    ss.open = false;
                    ss.highlighted_index = None;
                    state.window.request_redraw();
                    return true;
                }
            }
            false
        }

        NamedKey::ArrowDown => {
            {
                let el = state.arena.get_mut(focused).unwrap();
                if let Some(ref mut ss) = el.select_state {
                    let len = ss.options.len() as u32;
                    if len == 0 {
                        return true;
                    }
                    if !ss.open {
                        ss.open = true;
                        ss.highlighted_index = Some(ss.selected_index);
                    } else {
                        let cur = ss.highlighted_index.unwrap_or(ss.selected_index);
                        ss.highlighted_index = Some((cur + 1).min(len - 1));
                    }
                }
            }
            state.window.request_redraw();
            true
        }

        NamedKey::ArrowUp => {
            {
                let el = state.arena.get_mut(focused).unwrap();
                if let Some(ref mut ss) = el.select_state {
                    let len = ss.options.len() as u32;
                    if len == 0 {
                        return true;
                    }
                    if !ss.open {
                        ss.open = true;
                        ss.highlighted_index = Some(ss.selected_index);
                    } else {
                        let cur = ss.highlighted_index.unwrap_or(ss.selected_index);
                        ss.highlighted_index = Some(cur.saturating_sub(1));
                    }
                }
            }
            state.window.request_redraw();
            true
        }

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scroll_motion::browser_scroll_ease;

    #[test]
    fn default_app_config_uses_dark_theme() {
        let config = AppConfig::default();
        assert_eq!(config.theme.name, "dark");
        assert!(config.theme.colors.background.r < 50);
        assert!(config.theme.colors.text.r > 200);
    }

    #[test]
    fn app_config_css_path_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.css_path.is_none());
    }

    #[test]
    fn app_config_decorations_defaults_to_native_chrome() {
        let config = AppConfig::default();
        assert!(config.decorations);
    }

    #[test]
    fn window_resize_direction_maps_edges_and_corners() {
        let size = PhysicalSize::new(800, 600);

        assert_eq!(window_resize_direction(size, (4.0, 300.0), 1.0), Some(ResizeDirection::West));
        assert_eq!(window_resize_direction(size, (796.0, 300.0), 1.0), Some(ResizeDirection::East));
        assert_eq!(
            window_resize_direction(size, (4.0, 4.0), 1.0),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(window_resize_direction(size, (400.0, 300.0), 1.0), None);
    }

    #[test]
    fn window_resize_direction_scales_grip_for_hidpi() {
        let size = PhysicalSize::new(800, 600);

        assert_eq!(window_resize_direction(size, (20.0, 300.0), 1.0), None);
        assert_eq!(window_resize_direction(size, (20.0, 300.0), 2.0), Some(ResizeDirection::West));
    }

    #[test]
    fn custom_window_resize_direction_is_disabled_with_native_decorations() {
        let size = PhysicalSize::new(800, 600);

        assert_eq!(custom_window_resize_direction(true, size, (4.0, 4.0), 1.0), None);
        assert_eq!(
            custom_window_resize_direction(false, size, (4.0, 4.0), 1.0),
            Some(ResizeDirection::NorthWest)
        );
    }

    #[test]
    fn resize_direction_cursor_icon_uses_expected_edge_and_corner_cursors() {
        assert_eq!(resize_direction_cursor_icon(ResizeDirection::West), CursorIcon::EwResize);
        assert_eq!(resize_direction_cursor_icon(ResizeDirection::South), CursorIcon::NsResize);
        assert_eq!(
            resize_direction_cursor_icon(ResizeDirection::NorthWest),
            CursorIcon::NwseResize
        );
        assert_eq!(
            resize_direction_cursor_icon(ResizeDirection::NorthEast),
            CursorIcon::NeswResize
        );
    }

    #[test]
    fn compiled_stylesheet_parse_roundtrip() {
        let stylesheet = CompiledStylesheet::parse(".hot { color: green; }");
        assert!(!stylesheet.rules.is_empty(), "should parse at least one rule");
    }

    #[test]
    fn frame_metrics_default_has_zero_values() {
        let m = FrameMetrics::default();
        assert_eq!(m.tree_build_us, 0);
        assert_eq!(m.style_resolve_us, 0);
        assert_eq!(m.scale_us, 0);
        assert_eq!(m.layout_us, 0);
        assert_eq!(m.batch_build_us, 0);
        assert_eq!(m.gpu_render_us, 0);
        assert_eq!(m.total_us, 0);
        assert_eq!(m.node_count, 0);
        assert_eq!(m.rss_bytes, 0);
        assert_eq!(m.nodes_visited, 0);
        assert_eq!(m.nodes_skipped, 0);
        assert_eq!(m.quad_count, 0);
        assert_eq!(m.glyph_count, 0);
        assert_eq!(m.atlas_fill_ratio, 0.0);
        assert_eq!(m.gpu_upload_bytes, 0);
        assert_eq!(m.damage_area_px, 0);
    }

    #[test]
    fn app_config_on_frame_metrics_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.on_frame_metrics.is_none());
    }

    #[test]
    fn app_config_scroll_tuning_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.scroll_tuning.is_none());
    }

    #[test]
    fn app_config_scroll_telemetry_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.on_scroll_telemetry.is_none());
    }

    #[test]
    fn scroll_tuning_sanitizes_unusable_values() {
        let tuning =
            ScrollTuning { line_scroll_px: f32::NAN, smooth_scroll_duration_ms: 0 }.sanitized();

        assert_eq!(tuning.line_scroll_px, DEFAULT_WHEEL_LINE_SCROLL_PX);
        assert_eq!(tuning.smooth_scroll_duration_ms, 16);
    }

    #[test]
    fn app_config_on_cell_metrics_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.on_cell_metrics.is_none());
    }

    #[test]
    fn app_config_on_window_maximized_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.on_window_maximized.is_none());
    }

    #[test]
    fn publish_window_maximized_change_only_notifies_on_change() {
        use std::sync::Mutex;

        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_clone = calls.clone();
        let callback: Arc<dyn Fn(bool) + Send + Sync> =
            Arc::new(move |maximized| calls_clone.lock().unwrap().push(maximized));
        let mut last = false;

        assert!(!publish_window_maximized_change(&mut last, false, Some(&callback)));
        assert!(publish_window_maximized_change(&mut last, true, Some(&callback)));
        assert!(!publish_window_maximized_change(&mut last, true, Some(&callback)));
        assert!(publish_window_maximized_change(&mut last, false, Some(&callback)));

        assert_eq!(*calls.lock().unwrap(), vec![true, false]);
    }

    #[test]
    fn app_config_on_cell_metrics_accepts_callback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let called = std::sync::Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        let config = AppConfig {
            on_cell_metrics: Some(std::sync::Arc::new(move |w: f32, h: f32| {
                assert!(w > 0.0);
                assert!(h > 0.0);
                called_clone.store(true, Ordering::SeqCst);
            })),
            ..AppConfig::default()
        };
        // Fire the callback manually to verify the signature is correct.
        if let Some(ref cb) = config.on_cell_metrics {
            cb(8.0, 16.0);
        }
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn lightweight_relayout_dispatches_resize_callbacks() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let fired = Arc::new(AtomicU32::new(0));
        let fired_clone = fired.clone();
        let stylesheet = CompiledStylesheet::parse(
            ".root { width: 100%; height: 100%; } .pane { width: 100%; height: 100%; }",
        );
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def = ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("pane").on_resize(move |_w, _h| {
                fired_clone.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        let mut font_system = FontSystem::new();
        let mut measure_cache = TextMeasureCache::new();

        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
        run_layout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            800.0,
            600.0,
            &mut measure_cache,
        );
        assert_eq!(fired.load(Ordering::SeqCst), 1);

        let callbacks_fired = relayout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            400.0,
            600.0,
            &mut measure_cache,
        );

        assert!(
            callbacks_fired,
            "lightweight relayout must report fired resize callbacks so the app can rebuild snapshots"
        );
        assert_eq!(
            fired.load(Ordering::SeqCst),
            2,
            "window-size relayout must fire on_resize when element dimensions change"
        );
    }

    #[test]
    fn lightweight_relayout_marks_paint_dirty_after_window_resize() {
        let stylesheet = CompiledStylesheet::parse(
            ".root { width: 100%; height: 100%; } .pane { width: 100%; height: 100%; }",
        );
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def = ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("pane"));
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        let pane = arena.children(root)[0];
        let mut font_system = FontSystem::new();
        let mut measure_cache = TextMeasureCache::new();

        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
        run_layout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            800.0,
            600.0,
            &mut measure_cache,
        );
        let ids: Vec<NodeId> = arena.iter().map(|(id, _)| id).collect();
        for id in ids {
            arena.get_mut(id).unwrap().dirty = DirtyFlags::empty();
        }

        relayout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            400.0,
            600.0,
            &mut measure_cache,
        );

        assert!(
            arena.get(root).unwrap().dirty.contains(DirtyFlags::PAINT),
            "root must repaint after a window-size relayout"
        );
        assert!(
            arena.get(pane).unwrap().dirty.contains(DirtyFlags::PAINT),
            "resized child subtree must repaint after a window-size relayout"
        );
    }

    #[test]
    fn lightweight_relayout_resizes_full_height_flex_shell() {
        let stylesheet = CompiledStylesheet::parse(
            "
            .app { width: 100%; height: 100%; display: flex; flex-direction: column; }
            .titlebar { height: 34px; flex-shrink: 0; }
            .content { flex: 1; min-height: 0; }
            .statusbar { height: 24px; flex-shrink: 0; }
            ",
        );
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def = ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(ElementDef::new(Tag::Div).with_class("titlebar"))
            .with_child(ElementDef::new(Tag::Div).with_class("content"))
            .with_child(ElementDef::new(Tag::Div).with_class("statusbar"));
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        let mut font_system = FontSystem::new();
        let mut measure_cache = TextMeasureCache::new();

        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
        run_layout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            1280.0,
            800.0,
            &mut measure_cache,
        );

        relayout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            1280.0,
            1368.0,
            &mut measure_cache,
        );

        let children = arena.children(root);
        let content = arena.get(children[1]).unwrap().layout_rect;
        let statusbar = arena.get(children[2]).unwrap().layout_rect;

        assert!(
            (content.height - 1310.0).abs() < 1.0,
            "content must grow to fill snapped window height, got {}",
            content.height
        );
        assert!(
            (statusbar.y - 1344.0).abs() < 1.0,
            "statusbar must stay at bottom after snapped resize, got y={}",
            statusbar.y
        );
    }

    #[test]
    fn surface_metric_change_detects_snap_resize_without_scale_change() {
        let change = classify_surface_metrics_change(
            (1280.0, 800.0),
            1.0,
            winit::dpi::PhysicalSize::new(1280, 1368),
            1.0,
        );

        assert_eq!(change, SurfaceMetricsChange::Relayout);
    }

    #[test]
    fn surface_metric_change_ignores_zero_size() {
        let change = classify_surface_metrics_change(
            (1280.0, 800.0),
            1.0,
            winit::dpi::PhysicalSize::new(0, 1368),
            1.0,
        );

        assert_eq!(change, SurfaceMetricsChange::None);
    }

    #[test]
    fn surface_metric_change_promotes_scale_change_to_rebuild() {
        let change = classify_surface_metrics_change(
            (1280.0, 800.0),
            1.0,
            winit::dpi::PhysicalSize::new(640, 400),
            1.25,
        );

        assert_eq!(change, SurfaceMetricsChange::Rebuild);
    }

    #[test]
    fn frame_metrics_can_be_constructed_with_non_zero_values() {
        let m = FrameMetrics {
            tree_build_us: 100,
            style_resolve_us: 200,
            scale_us: 10,
            layout_us: 300,
            batch_build_us: 50,
            gpu_render_us: 400,
            total_us: 1060,
            node_count: 42,
            rss_bytes: 1024 * 1024,
            nodes_visited: 40,
            nodes_skipped: 2,
            quad_count: 128,
            glyph_count: 512,
            atlas_fill_ratio: 0.75,
            gpu_upload_bytes: 8192,
            damage_area_px: 1920 * 1080,
            pacer_min_interval_ns: 6_944_444,
            present_interval_us: 8_333,
            display_period_ns: 8_333_333,
            present_wait_us: 7_900,
        };
        assert_eq!(m.quad_count, 128);
        assert_eq!(m.glyph_count, 512);
        assert!((m.atlas_fill_ratio - 0.75).abs() < f32::EPSILON);
        assert_eq!(m.gpu_upload_bytes, 8192);
        assert_eq!(m.damage_area_px, 1920 * 1080);
        assert_eq!(m.pacer_min_interval_ns, 6_944_444);
        assert_eq!(m.present_interval_us, 8_333);
        assert_eq!(m.display_period_ns, 8_333_333);
        assert_eq!(m.present_wait_us, 7_900);
    }

    #[test]
    fn frame_metrics_default_has_zero_presentation_fields() {
        let m = FrameMetrics::default();
        assert_eq!(m.present_interval_us, 0);
        assert_eq!(m.display_period_ns, 0);
        assert_eq!(m.present_wait_us, 0);
    }

    #[test]
    fn pacing_mode_follows_surface_vsync_pacing() {
        assert_eq!(pacing_mode_for_surface(true), PacingMode::VsyncBlocking);
        assert_eq!(pacing_mode_for_surface(false), PacingMode::Timer);
    }

    #[test]
    fn predicted_present_ts_extrapolates_one_period_from_last_paint() {
        let t0 = Instant::now();
        let frame_start = t0 + Duration::from_millis(2);
        assert_eq!(
            predicted_present_ts(Some(t0), 8_333_333, frame_start),
            t0 + Duration::from_nanos(8_333_333),
        );
    }

    #[test]
    fn predicted_present_ts_falls_back_to_frame_start_when_unknown() {
        let t0 = Instant::now();
        let frame_start = t0 + Duration::from_millis(2);
        // No previous paint.
        assert_eq!(predicted_present_ts(None, 8_333_333, frame_start), frame_start);
        // Unknown display period.
        assert_eq!(predicted_present_ts(Some(t0), 0, frame_start), frame_start);
    }

    #[test]
    fn predicted_present_ts_never_moves_behind_frame_start() {
        // After a missed vblank, last + period lands before this frame
        // even started; the clamp keeps sampled timestamps monotone.
        let t0 = Instant::now();
        let frame_start = t0 + Duration::from_millis(20);
        assert_eq!(predicted_present_ts(Some(t0), 8_333_333, frame_start), frame_start);
    }

    #[test]
    fn predicted_present_ts_treats_idle_cadence_breaks_as_unknown() {
        // A gap of ACTIVITY_WINDOW or more means the paint rhythm was
        // intentionally idle (blink ticks); extrapolating from the stale
        // completion would be meaningless.
        let t0 = Instant::now();
        let frame_start = t0 + ACTIVITY_WINDOW;
        assert_eq!(predicted_present_ts(Some(t0), 8_333_333, frame_start), frame_start);
    }

    #[test]
    fn skipped_frame_resume_is_strictly_future_across_consecutive_skips() {
        // Regression for the skipped-frame busy-spin hazard: each skip
        // anchors its resume to the skipping frame's own start, never a
        // stale completion timestamp, so two consecutive skips schedule
        // two strictly increasing wakes instead of collapsing into the
        // past and spinning the loop through zero-timeout waits.
        let t0 = Instant::now();
        let first = skipped_frame_resume_at(t0, 8_333_333);
        assert!(first > t0);
        let second = skipped_frame_resume_at(first, 8_333_333);
        assert!(second > first);
        assert_eq!(second, first + Duration::from_nanos(8_333_333));
    }

    #[test]
    fn skipped_frame_resume_falls_back_to_120hz_period_when_rate_unknown() {
        let t0 = Instant::now();
        assert_eq!(skipped_frame_resume_at(t0, 0), t0 + Duration::from_nanos(8_333_333));
    }

    #[test]
    fn display_period_ns_from_mhz_maps_rates_and_unknown() {
        // 120Hz panel: 1e12 / 120_000 mHz = 8_333_333ns.
        assert_eq!(display_period_ns_from_mhz(120_000), 8_333_333);
        // Unknown rate maps to 0 so consumers can fall back to the
        // pacer's coalescing interval.
        assert_eq!(display_period_ns_from_mhz(0), 0);
        // 500Hz panel: 2ms period.
        assert_eq!(display_period_ns_from_mhz(500_000), 2_000_000);
    }

    #[test]
    fn present_interval_us_is_zero_for_first_frame() {
        assert_eq!(present_interval_us(None, Instant::now()), 0);
    }

    #[test]
    fn present_interval_us_measures_gap_to_previous_paint() {
        let t0 = Instant::now();
        assert_eq!(present_interval_us(Some(t0), t0 + Duration::from_micros(8_333)), 8_333);
        // A non-monotonic pair saturates to zero instead of panicking.
        assert_eq!(present_interval_us(Some(t0 + Duration::from_millis(5)), t0), 0);
    }

    #[test]
    fn cadence_present_interval_us_passes_active_gaps_through() {
        assert_eq!(cadence_present_interval_us(8_333), 8_333);
        // A badly missed refresh (3x the 120Hz period) is still cadence.
        assert_eq!(cadence_present_interval_us(25_000), 25_000);
        let just_below = ACTIVITY_WINDOW.as_micros() as u64 - 1;
        assert_eq!(cadence_present_interval_us(just_below), just_below);
    }

    #[test]
    fn cadence_present_interval_us_zeroes_idle_breaks() {
        // At or past the activity window the loop had gone idle by
        // design, so the gap is a cadence break, not a missed refresh.
        assert_eq!(cadence_present_interval_us(ACTIVITY_WINDOW.as_micros() as u64), 0);
        // The 500ms cursor-blink wake of an idle session.
        assert_eq!(cadence_present_interval_us(500_000), 0);
    }

    #[test]
    fn activity_window_is_250ms() {
        // Document the chosen coalescing window. If this value changes,
        // the speculative-frame behavior and idle CPU profile both move,
        // so callers should understand the intent.
        assert_eq!(ACTIVITY_WINDOW, Duration::from_millis(250));
    }

    #[test]
    fn keyboard_capture_shortcut_gate_allows_function_keys() {
        use unshit_core::shortcut::KeyCombo;

        assert!(should_check_shortcut_during_keyboard_capture(&KeyCombo::plain(Key::F(2))));
        assert!(should_check_shortcut_during_keyboard_capture(&KeyCombo::new(
            Key::Char('t'),
            Modifiers::CTRL
        )));
        assert!(!should_check_shortcut_during_keyboard_capture(&KeyCombo::plain(Key::Char('a'))));
    }

    #[test]
    fn raw_key_hook_can_consume_plain_navigation_keys() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use unshit_core::shortcut::KeyCombo;

        let called = Arc::new(AtomicBool::new(false));
        let called_for_hook = called.clone();
        let config = AppConfig {
            on_raw_key: Some(Arc::new(move |combo| {
                called_for_hook.store(true, Ordering::SeqCst);
                combo.key == Key::ArrowDown && combo.modifiers.is_empty()
            })),
            ..AppConfig::default()
        };

        assert!(consume_raw_key_hook(&config, &KeyCombo::plain(Key::ArrowDown)));
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn mouse_move_event_walks_to_ancestor_handler() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let called = Arc::new(AtomicBool::new(false));
        let called_for_handler = called.clone();
        let mut arena = NodeArena::new();
        let mut parent = Element::new(Tag::Div);
        parent.handlers.push((
            EventType::MouseMove,
            Arc::new(move |_| {
                called_for_handler.store(true, Ordering::SeqCst);
                None
            }),
        ));
        let parent_id = arena.alloc(parent);
        let child_id = arena.alloc(Element::new(Tag::Span));
        arena.append_child(parent_id, child_id);

        let event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Move,
            x: 1.0,
            y: 1.0,
            local_x: 1.0,
            local_y: 1.0,
            button: MouseButton::None,
            modifiers: Modifiers::empty(),
        });

        assert!(dispatch_mouse_move_event(&arena, child_id, &event));
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn is_recently_active_is_true_within_250ms() {
        let last = Instant::now();
        // Zero elapsed: just bumped activity, obviously within the window.
        assert!(is_within_activity_window(last, last, ACTIVITY_WINDOW));

        // 100ms later: still well inside the 250ms window.
        let now = last + Duration::from_millis(100);
        assert!(is_within_activity_window(last, now, ACTIVITY_WINDOW));

        // 249ms later: right up against the boundary but still inside.
        let now = last + Duration::from_millis(249);
        assert!(is_within_activity_window(last, now, ACTIVITY_WINDOW));
    }

    #[test]
    fn is_recently_active_is_false_after_250ms() {
        let last = Instant::now();
        // Exactly 250ms: boundary is exclusive, so this counts as idle
        // and the event loop falls back to `ControlFlow::Wait`.
        let now = last + Duration::from_millis(250);
        assert!(!is_within_activity_window(last, now, ACTIVITY_WINDOW));

        // 1s later: clearly idle.
        let now = last + Duration::from_secs(1);
        assert!(!is_within_activity_window(last, now, ACTIVITY_WINDOW));
    }

    #[test]
    fn is_recently_active_handles_clock_skew() {
        // If `now` somehow precedes `last_activity` (should not happen on
        // winit but paranoia is cheap), `saturating_duration_since` returns
        // zero and the helper reports "recently active". This matches the
        // intuitive reading: no time has passed, so we just saw activity.
        let last = Instant::now() + Duration::from_millis(100);
        let now = last - Duration::from_millis(50);
        assert!(is_within_activity_window(last, now, ACTIVITY_WINDOW));
    }

    #[test]
    fn line_wheel_delta_is_normalized_to_smooth_pixel_scroll() {
        let (dx, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::LineDelta(1.0, -2.0),
            1.0,
            1.0,
            ScrollTuning::default(),
            1.0,
            1.0,
        );

        assert_eq!(dx, DEFAULT_WHEEL_LINE_SCROLL_PX);
        assert_eq!(dy, -2.0 * DEFAULT_WHEEL_LINE_SCROLL_PX);
        assert!(smooth);
    }

    #[test]
    fn line_wheel_delta_uses_scroll_tuning() {
        let (dx, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::LineDelta(1.0, -1.0),
            1.0,
            1.0,
            ScrollTuning { line_scroll_px: 72.0, smooth_scroll_duration_ms: 80 },
            1.0,
            1.0,
        );

        assert_eq!(dx, 72.0);
        assert_eq!(dy, -72.0);
        assert!(smooth);
    }

    #[test]
    fn windows_wheel_notch_delta_is_normalized_to_one_browser_step() {
        let (_, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::LineDelta(0.0, -3.0),
            1.5,
            1.0,
            ScrollTuning::default(),
            3.0,
            3.0,
        );

        assert_eq!(dy, -DEFAULT_WHEEL_LINE_SCROLL_PX * 1.5);
        assert!(smooth);
    }

    #[test]
    fn sub_notch_line_delta_scales_proportionally_instead_of_amplifying() {
        let (_, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::LineDelta(0.0, -0.6),
            1.0,
            1.0,
            ScrollTuning::default(),
            3.0,
            3.0,
        );

        assert!((dy - -0.2 * DEFAULT_WHEEL_LINE_SCROLL_PX).abs() < 0.001);
        assert!(smooth);
    }

    #[test]
    fn pixel_wheel_delta_stays_direct_for_precision_devices() {
        let (dx, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
                3.0, -14.0,
            )),
            1.0,
            1.0,
            ScrollTuning::default(),
            3.0,
            3.0,
        );

        assert_eq!(dx, 3.0);
        assert_eq!(dy, -14.0);
        assert!(!smooth);
    }

    #[test]
    fn normalize_wheel_line_delta_divides_unconditionally_by_lines_per_notch() {
        assert_eq!(normalize_wheel_line_delta(3.0, 3.0), 1.0);
        assert_eq!(normalize_wheel_line_delta(-3.0, 3.0), -1.0);
        assert!((normalize_wheel_line_delta(1.0, 3.0) - 1.0 / 3.0).abs() < 1e-6);
        assert!((normalize_wheel_line_delta(0.6, 3.0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn fragmented_sub_notch_events_sum_to_one_notch() {
        let notches: f32 = (0..5).map(|_| normalize_wheel_line_delta(0.6, 3.0)).sum();
        assert!((notches - 1.0).abs() < 1e-6);
    }

    #[test]
    fn one_line_per_notch_platforms_pass_line_deltas_through() {
        assert_eq!(normalize_wheel_line_delta(1.0, 1.0), 1.0);
        assert_eq!(normalize_wheel_line_delta(-0.25, 1.0), -0.25);
    }

    #[test]
    fn invalid_lines_per_notch_falls_back_to_default_notch_size() {
        assert_eq!(normalize_wheel_line_delta(3.0, 0.0), 1.0);
        assert_eq!(normalize_wheel_line_delta(3.0, -2.0), 1.0);
        assert_eq!(normalize_wheel_line_delta(3.0, f32::NAN), 1.0);
    }

    #[test]
    fn lines_per_notch_is_positive_and_finite() {
        let lines = lines_per_notch();
        assert!(lines.is_finite() && lines > 0.0);
    }

    #[test]
    fn chars_per_notch_is_positive_and_finite() {
        let chars = chars_per_notch();
        assert!(chars.is_finite() && chars > 0.0);
    }

    #[test]
    fn horizontal_wheel_delta_is_normalized_by_chars_per_notch() {
        // One horizontal detent arrives as 3 chars; lines-per-notch is
        // deliberately different so any axis mix-up would be visible.
        let (dx, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::LineDelta(3.0, 0.0),
            1.0,
            1.0,
            ScrollTuning::default(),
            3.0,
            10.0,
        );

        assert_eq!(dx, DEFAULT_WHEEL_LINE_SCROLL_PX);
        assert_eq!(dy, 0.0);
        assert!(smooth);
    }

    #[test]
    fn asymmetric_per_axis_divisors_normalize_each_axis_independently() {
        // chars-per-notch 3 and lines-per-notch 10: one detent on each
        // axis must normalize to exactly one browser step per axis.
        let (dx, dy, smooth) = wheel_scroll_delta_pixels(
            winit::event::MouseScrollDelta::LineDelta(3.0, -10.0),
            1.0,
            1.0,
            ScrollTuning::default(),
            3.0,
            10.0,
        );

        assert_eq!(dx, DEFAULT_WHEEL_LINE_SCROLL_PX);
        assert_eq!(dy, -DEFAULT_WHEEL_LINE_SCROLL_PX);
        assert!(smooth);
    }

    #[test]
    fn browser_like_wheel_duration_matches_edge_wheel_metrics() {
        let tuning = ScrollTuning::default();
        assert_eq!(browser_like_wheel_duration((0.0, -100.0), tuning), Duration::from_millis(180));
        assert_eq!(browser_like_wheel_duration((0.0, -200.0), tuning), Duration::from_millis(144));
        assert_eq!(browser_like_wheel_duration((0.0, -400.0), tuning), Duration::from_millis(94));
    }

    #[test]
    fn browser_like_notch_scroll_keeps_120hz_frame_steps_small() {
        let now = Instant::now();
        let slope = browser_like_initial_slope((0.0, -100.0));
        let scroll = SmoothScroll {
            node_id: NodeId { index: 1, generation: 0 },
            start_x: 0.0,
            start_y: 0.0,
            target_x: 0.0,
            target_y: DEFAULT_WHEEL_LINE_SCROLL_PX * 1.5,
            started_at: now,
            duration: Duration::from_millis(DEFAULT_SMOOTH_SCROLL_DURATION_MS),
            initial_slope: slope,
        };

        let mut previous_y = 0.0;
        let mut max_step = 0.0_f32;
        let frame_interval = Duration::from_nanos(8_333_333);
        for frame in 1..=22 {
            let ((_, y), _) = scroll.position_at(now + frame_interval * frame);
            max_step = max_step.max((y - previous_y).abs());
            previous_y = y;
        }

        assert!(
            max_step <= 12.0,
            "120Hz wheel frames should move less than 12px at 150px total distance, got {max_step:.2}px"
        );
    }

    #[test]
    fn browser_scroll_ease_tracks_measured_edge_notch_curve() {
        let slope = browser_like_initial_slope((0.0, -100.0));
        let (quarter, _) = browser_scroll_ease(0.25, slope);
        let (half, _) = browser_scroll_ease(0.5, slope);
        let (three_quarter, _) = browser_scroll_ease(0.75, slope);

        assert!((quarter - 0.17).abs() < 0.03);
        assert!((half - 0.54).abs() < 0.03);
        assert!((three_quarter - 0.88).abs() < 0.03);
    }

    #[test]
    fn browser_scroll_ease_gets_more_front_loaded_for_large_wheel_deltas() {
        let small = browser_like_initial_slope((0.0, -100.0));
        let large = browser_like_initial_slope((0.0, -400.0));
        let (small_half, _) = browser_scroll_ease(0.5, small);
        let (large_half, _) = browser_scroll_ease(0.5, large);

        assert!(large > small);
        assert!(large_half > small_half);
    }

    #[test]
    fn smooth_scroll_compounds_wheel_ticks_from_pending_target() {
        let node_id = NodeId { index: 42, generation: 0 };
        let now = Instant::now();
        let first = next_smooth_scroll(
            (0.0, 0.0),
            (0.0, 500.0),
            None,
            node_id,
            (0.0, -80.0),
            now,
            Duration::from_millis(80),
            0.25,
        )
        .expect("first wheel tick should start scroll");

        let second = next_smooth_scroll(
            (0.0, 12.0),
            (0.0, 500.0),
            Some(first),
            node_id,
            (0.0, -80.0),
            now + Duration::from_millis(20),
            Duration::from_millis(80),
            0.25,
        )
        .expect("second wheel tick should extend scroll target");

        assert_eq!(first.target_y, 80.0);
        assert_eq!(second.start_y, 12.0);
        assert_eq!(second.target_y, 160.0);
        assert!(second.initial_slope > 0.25, "retargeted wheel scroll should preserve velocity");
    }

    #[test]
    fn smooth_scroll_eases_to_exact_target() {
        let node_id = NodeId { index: 7, generation: 0 };
        let now = Instant::now();
        let scroll = SmoothScroll {
            node_id,
            start_x: 0.0,
            start_y: 0.0,
            target_x: 0.0,
            target_y: 100.0,
            started_at: now,
            duration: Duration::from_millis(100),
            initial_slope: 0.0,
        };

        let ((_, mid_y), done) = scroll.position_at(now + Duration::from_millis(50));
        assert!((mid_y - 50.0).abs() < 0.1);
        assert!(!done);

        let ((_, end_y), done) = scroll.position_at(now + Duration::from_millis(100));
        assert_eq!(end_y, 100.0);
        assert!(done);
    }

    #[test]
    fn cascade_root_for_restyle_keeps_narrow_scope_at_normal_scale() {
        let document_root = NodeId { index: 1, generation: 0 };
        let restyle_root = NodeId { index: 8, generation: 0 };

        assert_eq!(cascade_root_for_restyle(Some(restyle_root), document_root, 1.0), restyle_root);
    }

    #[test]
    fn cascade_root_for_restyle_uses_document_root_when_scaled() {
        let document_root = NodeId { index: 1, generation: 0 };
        let restyle_root = NodeId { index: 8, generation: 0 };

        assert_eq!(
            cascade_root_for_restyle(Some(restyle_root), document_root, 1.25),
            document_root
        );
    }

    #[test]
    fn cascade_root_for_restyle_defaults_to_document_root() {
        let document_root = NodeId { index: 1, generation: 0 };

        assert_eq!(cascade_root_for_restyle(None, document_root, 1.0), document_root);
    }

    #[test]
    fn subtree_has_dirty_flags_detects_descendant_work() {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def =
            ElementDef::new(Tag::Div).with_child(ElementDef::new(Tag::Div).with_class("child"));
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        let ids: Vec<NodeId> = arena.iter().map(|(id, _)| id).collect();
        for id in ids {
            arena.get_mut(id).unwrap().dirty = DirtyFlags::empty();
        }
        let child = arena.children(root)[0];
        arena.get_mut(child).unwrap().dirty = DirtyFlags::LAYOUT;

        assert!(subtree_has_dirty_flags(&arena, root, DirtyFlags::LAYOUT));
        assert!(!subtree_has_dirty_flags(&arena, root, DirtyFlags::STYLE));
    }

    #[test]
    fn mark_full_restyle_required_sets_rebuild_work_flags() {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def = ElementDef::new(Tag::Div);
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        arena.get_mut(root).unwrap().dirty = DirtyFlags::empty();

        mark_full_restyle_required(&mut arena, root);

        let dirty = arena.get(root).unwrap().dirty;
        assert!(dirty.contains(DirtyFlags::STYLE));
        assert!(dirty.contains(DirtyFlags::LAYOUT));
        assert!(dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn pseudo_restyle_entering_from_no_hover_uses_root() {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def = ElementDef::new(Tag::Div).with_child(
            ElementDef::new(Tag::Button).with_child(ElementDef::new(Tag::Span).with_text("go")),
        );
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        let button = arena.children(root)[0];
        let span = arena.children(button)[0];

        assert_eq!(
            pseudo_restyle_root_for_change(&arena, NodeId::DANGLING, span, root),
            root,
            "entering a child must restyle ancestors that may match :hover"
        );
    }

    #[test]
    fn pseudo_restyle_between_valid_nodes_uses_lca() {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let root_def = ElementDef::new(Tag::Div).with_child(
            ElementDef::new(Tag::Button)
                .with_child(ElementDef::new(Tag::Span).with_class("a"))
                .with_child(ElementDef::new(Tag::Span).with_class("b")),
        );
        let root = build_tree_from_def(&root_def, &mut arena, &mut taffy, NodeId::DANGLING);
        let button = arena.children(root)[0];
        let children = arena.children(button);

        assert_eq!(pseudo_restyle_root_for_change(&arena, children[0], children[1], root), button);
    }

    // === RebuildCoalescer (#135 Phase 1, item 1) ===

    #[test]
    fn rebuild_coalescer_default_is_idle() {
        let c = RebuildCoalescer::default();
        assert!(!c.needs_rebuild);
        assert!(!c.saw_event);
        assert_eq!(c.rebuild_request_count, 0);
    }

    #[test]
    fn rebuild_coalescer_observe_redraw_only_marks_activity() {
        let mut c = RebuildCoalescer::default();
        c.observe(false);
        assert!(c.saw_event, "redraw-only events still count as activity");
        assert!(!c.needs_rebuild, "redraw-only events do not request a rebuild");
        assert_eq!(c.rebuild_request_count, 0);
    }

    #[test]
    fn rebuild_coalescer_observe_rebuild_sets_flag() {
        let mut c = RebuildCoalescer::default();
        c.observe(true);
        assert!(c.saw_event);
        assert!(c.needs_rebuild);
        assert_eq!(c.rebuild_request_count, 1);
    }

    #[test]
    fn one_hundred_rebuild_events_coalesce_to_single_rebuild_flag() {
        // The Phase 1 cornerstone guarantee: any number of rebuild
        // requests that arrive in one drain window collapse to exactly
        // one tree rebuild on the next paint. The flag is idempotent;
        // the counter records how many requests landed for telemetry,
        // but `needs_rebuild` is unchanged after the first observation.
        let mut c = RebuildCoalescer::default();
        c.begin_drain();
        for _ in 0..100 {
            c.observe(true);
        }
        assert!(c.needs_rebuild, "100 rebuild events must set the flag exactly once");
        assert!(c.saw_event);
        assert_eq!(
            c.rebuild_request_count, 100,
            "telemetry counts every rebuild request even though they coalesce"
        );
    }

    #[test]
    fn begin_drain_resets_state_between_frames() {
        let mut c = RebuildCoalescer::default();
        for _ in 0..50 {
            c.observe(true);
        }
        assert!(c.needs_rebuild);
        assert_eq!(c.rebuild_request_count, 50);

        c.begin_drain();
        assert!(!c.needs_rebuild, "begin_drain clears the rebuild flag for the next frame");
        assert!(!c.saw_event);
        assert_eq!(c.rebuild_request_count, 0);

        // The next drain stands on its own: a single redraw event does not
        // resurrect a stale rebuild flag from the previous drain.
        c.observe(false);
        assert!(c.saw_event);
        assert!(!c.needs_rebuild);
        assert_eq!(c.rebuild_request_count, 0);
    }

    #[test]
    fn rebuild_coalescer_mixed_events_collapse_to_single_rebuild() {
        // A mix of redraw-only and rebuild-implying events lands in the
        // same drain. We expect the flag set exactly once and the counter
        // to track only the rebuild-implying events.
        let mut c = RebuildCoalescer::default();
        c.begin_drain();
        c.observe(false); // RequestRedraw
        c.observe(true); // RequestRebuild
        c.observe(false); // Bytes
        c.observe(true); // Custom (mutating)
        c.observe(false); // RequestRedraw
        c.observe(true); // RequestRebuild

        assert!(c.needs_rebuild);
        assert_eq!(c.rebuild_request_count, 3, "only rebuild-implying events count");
    }

    #[test]
    fn rebuild_coalescer_counter_saturates_under_extreme_storm() {
        // Pathological input must not wrap the telemetry counter. Pick
        // a sentinel value just below u32::MAX and confirm `saturating_add`
        // pins it at the ceiling rather than overflowing.
        let mut c = RebuildCoalescer {
            needs_rebuild: true,
            saw_event: true,
            rebuild_request_count: u32::MAX - 1,
        };
        c.observe(true);
        c.observe(true);
        c.observe(true);
        assert_eq!(c.rebuild_request_count, u32::MAX);
    }

    /// Build a root div with one grid child, clear the initial dirty flags,
    /// and return (arena, root, grid child).
    fn arena_with_grid_child(rows: usize, cols: usize) -> (NodeArena, NodeId, NodeId) {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let def = ElementDef::new(Tag::Div).with_child(
            ElementDef::new(Tag::Div).with_grid(unshit_core::cell_grid::CellGrid::new(rows, cols)),
        );
        let root = build_tree_from_def(&def, &mut arena, &mut taffy, NodeId::DANGLING);
        let child = arena.children(root)[0];
        for id in [root, child] {
            arena.get_mut(id).unwrap().dirty = DirtyFlags::empty();
        }
        (arena, root, child)
    }

    #[test]
    fn scroll_grid_patch_swaps_content_and_paint_dirties_only() {
        use unshit_core::cell_grid::{Cell, CellGrid};
        let (mut arena, root, child) = arena_with_grid_child(3, 4);
        let mut fresh = CellGrid::new(3, 4);
        fresh.set_cell(0, 0, Cell::with_char('x'));

        assert!(apply_scroll_grid_patch(&mut arena, child, fresh.clone()));

        match &arena.get(child).unwrap().content {
            ElementContent::Grid(grid) => assert_eq!(grid, &fresh),
            _ => panic!("expected grid content on the patched node"),
        }
        let child_dirty = arena.get(child).unwrap().dirty;
        assert!(child_dirty.contains(DirtyFlags::PAINT));
        assert!(!child_dirty.contains(DirtyFlags::STYLE));
        assert!(!child_dirty.contains(DirtyFlags::LAYOUT));
        assert!(
            arena.get(root).unwrap().dirty.contains(DirtyFlags::SUBTREE_PAINT),
            "ancestors must be SUBTREE_PAINT so the batch walk reaches the node"
        );
    }

    #[test]
    fn scroll_grid_patch_rejects_dimension_changes() {
        use unshit_core::cell_grid::CellGrid;
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);

        assert!(!apply_scroll_grid_patch(&mut arena, child, CellGrid::new(5, 4)));

        match &arena.get(child).unwrap().content {
            ElementContent::Grid(grid) => assert_eq!((grid.rows(), grid.cols()), (3, 4)),
            _ => panic!("expected grid content to be left untouched"),
        }
        assert!(!arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn scroll_grid_patch_rejects_non_grid_and_missing_nodes() {
        use unshit_core::cell_grid::CellGrid;
        let (mut arena, root, _child) = arena_with_grid_child(3, 4);

        assert!(!apply_scroll_grid_patch(&mut arena, root, CellGrid::new(3, 4)));
        assert!(!arena.get(root).unwrap().dirty.contains(DirtyFlags::PAINT));

        assert!(!apply_scroll_grid_patch(&mut arena, NodeId::DANGLING, CellGrid::new(3, 4)));
    }

    // === Grid animation hooks (scroll smoothness Phase 3, Stage F) ===

    #[test]
    fn default_scroll_grid_patch_is_consumed_no_change() {
        let patch = ScrollGridPatch::default();
        assert!(patch.grid.is_none());
        assert!(patch.animation.is_none());
    }

    /// A hook whose sampler returns a fixed sequence of `GridTick`s and
    /// counts invocations, plus the timestamps it was called with.
    fn counting_hook(
        deadline: Instant,
        results: Vec<fn(usize) -> GridTick>,
    ) -> (GridAnimationHook, Arc<std::sync::Mutex<Vec<Instant>>>) {
        let calls: Arc<std::sync::Mutex<Vec<Instant>>> = Arc::default();
        let calls_in_hook = Arc::clone(&calls);
        let hook = GridAnimationHook {
            deadline,
            tick: Arc::new(move |now| {
                let mut guard = calls_in_hook.lock().unwrap();
                let index = guard.len();
                guard.push(now);
                results[index.min(results.len() - 1)](index)
            }),
        };
        (hook, calls)
    }

    fn fresh_grid_tick(_: usize) -> GridTick {
        use unshit_core::cell_grid::{Cell, CellGrid};
        let mut grid = CellGrid::new(3, 4);
        grid.set_cell(0, 0, Cell::with_char('x'));
        GridTick::Continue(grid)
    }

    fn done_grid_tick(_: usize) -> GridTick {
        GridTick::Done(unshit_core::cell_grid::CellGrid::new(3, 4))
    }

    fn idle_tick(_: usize) -> GridTick {
        GridTick::Idle
    }

    fn wrong_dims_tick(_: usize) -> GridTick {
        GridTick::Continue(unshit_core::cell_grid::CellGrid::new(5, 9))
    }

    #[test]
    fn grid_animation_continue_applies_paint_only_and_keeps_hook() {
        let (mut arena, root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) = counting_hook(now + Duration::from_secs(1), vec![fresh_grid_tick]);
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        let sample_time = now + Duration::from_millis(8);
        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, sample_time, true);

        assert!(visual_change);
        assert!(!rebuild_required);
        assert!(animations.contains_key(&child), "Continue keeps the hook registered");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[sample_time],
            "the sampler receives the injected frame timestamp exactly once"
        );
        // The patch landed paint-only: content swapped, PAINT on the node,
        // SUBTREE_PAINT on ancestors, no style/layout work.
        match &arena.get(child).unwrap().content {
            ElementContent::Grid(grid) => assert_eq!(grid.get_cell(0, 0).map(|c| c.ch), Some('x')),
            _ => panic!("expected grid content"),
        }
        let dirty = arena.get(child).unwrap().dirty;
        assert!(dirty.contains(DirtyFlags::PAINT));
        assert!(!dirty.contains(DirtyFlags::STYLE));
        assert!(!dirty.contains(DirtyFlags::LAYOUT));
        assert!(arena.get(root).unwrap().dirty.contains(DirtyFlags::SUBTREE_PAINT));
    }

    #[test]
    fn grid_animation_done_applies_and_unregisters() {
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) = counting_hook(now + Duration::from_secs(1), vec![done_grid_tick]);
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, now, true);

        assert!(visual_change);
        assert!(!rebuild_required);
        assert!(animations.is_empty(), "Done unregisters the hook");
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn grid_animation_idle_changes_nothing_and_keeps_hook() {
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, _calls) = counting_hook(now + Duration::from_secs(1), vec![idle_tick]);
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, now, true);

        assert!(!visual_change);
        assert!(!rebuild_required);
        assert!(animations.contains_key(&child));
        assert!(!arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn grid_animation_dims_mismatch_requests_rebuild_but_keeps_hook() {
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, _calls) = counting_hook(now + Duration::from_secs(1), vec![wrong_dims_tick]);
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, now, true);

        assert!(visual_change);
        assert!(rebuild_required, "a dims mismatch must fall back to a full rebuild");
        assert!(
            animations.contains_key(&child),
            "the hook survives the rebuild so the animation resumes afterwards"
        );
        // The mismatched grid was rejected: the node keeps its old dims.
        match &arena.get(child).unwrap().content {
            ElementContent::Grid(grid) => assert_eq!((grid.rows(), grid.cols()), (3, 4)),
            _ => panic!("expected grid content"),
        }
    }

    #[test]
    fn grid_animation_expired_deadline_settles_with_one_final_tick() {
        // A frame gap past the deadline must not strand the app-side
        // animation mid-flight: the hook gets exactly one final sample
        // (which lets the app settle on its target) and its frame is
        // painted before the hook is unregistered.
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) = counting_hook(now, vec![fresh_grid_tick]);
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        let sample_time = now + Duration::from_millis(1);
        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, sample_time, true);

        assert!(visual_change, "the settling frame is a visual change");
        assert!(!rebuild_required);
        assert!(animations.is_empty(), "expired hooks are unregistered");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[sample_time],
            "expired hooks are sampled exactly once so the animation settles"
        );
        match &arena.get(child).unwrap().content {
            ElementContent::Grid(grid) => assert_eq!(grid.get_cell(0, 0).map(|c| c.ch), Some('x')),
            _ => panic!("expected grid content"),
        }
        assert!(arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn grid_animation_dangling_node_keeps_ticking_blind_until_done() {
        // A vanished node (reconcile re-keyed or unmounted it mid-flight)
        // must not strand the app-side animation: the hook keeps sampling
        // without painting, requests a rebuild so a re-mounted surface
        // shows the advanced position, and unregisters on `Done`.
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) =
            counting_hook(now + Duration::from_secs(1), vec![fresh_grid_tick, done_grid_tick]);
        let mut animations = HashMap::new();
        animations.insert(NodeId::DANGLING, hook);

        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, now, true);

        assert!(!visual_change, "nothing on screen changed");
        assert!(rebuild_required, "the advanced sample needs a rebuild to become visible");
        assert!(animations.contains_key(&NodeId::DANGLING), "the hook survives to settle");
        assert!(!arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT));

        let later = now + Duration::from_millis(8);
        let (visual_change, _) =
            tick_grid_animations_core(&mut arena, &mut animations, later, true);

        assert!(!visual_change);
        assert!(animations.is_empty(), "Done unregisters the blind hook");
        assert_eq!(calls.lock().unwrap().as_slice(), &[now, later]);
    }

    #[test]
    fn grid_animation_expired_deadline_with_dangling_node_still_settles() {
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) = counting_hook(now, vec![fresh_grid_tick]);
        let mut animations = HashMap::new();
        animations.insert(NodeId::DANGLING, hook);

        let sample_time = now + Duration::from_millis(1);
        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, sample_time, true);

        assert!(!visual_change, "no node to paint");
        assert!(!rebuild_required, "the hook is gone; the next rebuild composes fresh anyway");
        assert!(animations.is_empty());
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[sample_time],
            "the settling sample still runs so the app-side animation lands"
        );
        assert!(!arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn grid_animation_sample_only_advances_lifecycle_without_patching() {
        // Rebuild-bound frames tick hooks with `apply_patches = false`:
        // the sampler still advances (and `Done` still unregisters) but
        // the arena is left for the rebuild to recompose.
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) = counting_hook(now + Duration::from_secs(1), vec![done_grid_tick]);
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        let (visual_change, rebuild_required) =
            tick_grid_animations_core(&mut arena, &mut animations, now, false);

        assert!(visual_change);
        assert!(!rebuild_required);
        assert!(animations.is_empty(), "Done unregisters even without applying");
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(
            !arena.get(child).unwrap().dirty.contains(DirtyFlags::PAINT),
            "sample-only ticks must not touch the arena"
        );
    }

    #[test]
    fn grid_animation_runs_to_done_over_multiple_ticks() {
        let (mut arena, _root, child) = arena_with_grid_child(3, 4);
        let now = Instant::now();
        let (hook, calls) = counting_hook(
            now + Duration::from_secs(1),
            vec![fresh_grid_tick, fresh_grid_tick, done_grid_tick],
        );
        let mut animations = HashMap::new();
        animations.insert(child, hook);

        for frame in 0..3 {
            let tick_time = now + Duration::from_millis(8 * (frame + 1));
            tick_grid_animations_core(&mut arena, &mut animations, tick_time, true);
        }

        assert!(animations.is_empty(), "the third tick's Done must unregister");
        assert_eq!(calls.lock().unwrap().len(), 3);
    }
}
