use crate::clipboard::ClipboardContext;
use crate::event_sink::{EventSink, ExternalEvent};
use crate::notification::{AttentionUrgency, BellConfig, BellState};
use crate::shortcut::{key_combo_from_winit, ShortcutResolver};
use crate::window;
use cosmic_text::{FontSystem, SwashCache};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use unshit_core::build::{
    build_tree_from_def, mark_layout_dirty, resolve_all_styles,
    resolve_all_styles_with_transitions, run_layout_pipeline, scale_all_styles,
    sync_all_animations, tick_all_animations, tick_all_transitions,
};
use unshit_core::element::*;
use unshit_core::event::*;
use unshit_core::id::NodeId;
use unshit_core::layout::{self, TextMeasureCache, TextMeasureCtx};
use unshit_core::scroll::{self, ScrollbarAxis, ScrollbarPart, ScrollbarVisualState};
use unshit_core::style::animation::AnimationDriver;
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::style::theme::Theme;
use unshit_core::style::transition::ActiveTransitions;
use unshit_core::style::types::Layer;
use unshit_core::tree::NodeArena;
use unshit_renderer::batch::Rasterizer;
use unshit_renderer::batch::{self, BatchCache, ShapeCache, ShapedTextCache};
use unshit_renderer::canvas::{CanvasRegistry, CustomPainter};
#[cfg(target_os = "windows")]
use unshit_renderer::dw_rasterizer::DwRasterizer;
use unshit_renderer::gpu::GpuContext;
use unshit_renderer::pipeline::quad::QuadInstance;
use winit::application::ApplicationHandler;
use winit::cursor::CursorIcon;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

pub struct AppConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
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
    /// Callback invoked when the OS reports the DPI scale factor.
    /// Fires once at startup and again whenever the window moves between
    /// monitors with different scale factors.
    pub on_scale_factor: Option<Arc<dyn Fn(f32) + Send + Sync>>,
    /// Callback invoked just before the application window closes.
    pub on_close: Option<Arc<dyn Fn() + Send + Sync>>,
    /// One-shot callback invoked once the renderer publishes valid cell
    /// metrics (cell width and height in pixels). Fires after the first
    /// render pass that produces non-zero values, giving the application a
    /// reliable point to compute initial PTY dimensions.
    pub on_cell_metrics: Option<Arc<dyn Fn(f32, f32) + Send + Sync>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "unshit".to_string(),
            width: 800,
            height: 600,
            css: String::new(),
            keybindings_path: None,
            on_external_event: None,
            on_bytes: None,
            user_shortcuts: Vec::new(),
            on_command: None,
            fonts: Vec::new(),
            fallback_chain: crate::font::FallbackChain::default_chain(),
            theme: Theme::dark(),
            max_atlas_bytes: None,
            css_path: None,
            on_frame_metrics: None,
            on_scale_factor: None,
            on_close: None,
            on_cell_metrics: None,
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
    #[cfg(target_os = "windows")]
    dw_rasterizer: DwRasterizer,
    interaction: InteractionState,
    needs_rebuild: bool,
    needs_restyle: bool,
    needs_relayout: bool,
    scale_factor: f32,
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
    /// Coalesces redraw requests into at most one paint per
    /// `FramePacer::min_interval`. Prevents per-PTY-chunk rebuild storms
    /// from dominating the event loop. See [`crate::frame_pacer`].
    frame_pacer: crate::frame_pacer::FramePacer,
    /// Rolling window of per-frame durations. Debug-only; emits p50/p95/
    /// p99 quantiles once per second via `log::info!`. See
    /// [`crate::frame_probe`].
    #[cfg(debug_assertions)]
    frame_probe: crate::frame_probe::FrameProbe,
}

/// Snapshot the dirty-state signals that drive the post-paint paint-loop
/// scheduling in `RedrawRequested`. See issue #52 step 2.
///
/// The per-node paint-dirty walk is only a safety net:
/// `clear_paint_flags_subtree` runs during the paint so in the common case
/// no node carries `PAINT`/`SUBTREE_PAINT` after we return. The walk is
/// O(arena) so we short-circuit it whenever a cheaper flag already reports
/// dirty, keeping the frame-time overhead near zero on the hot path.
fn collect_dirty_signals(
    state: &AppState,
    event_rx: &flume::Receiver<ExternalEvent>,
) -> crate::frame_pacer::DirtySignals {
    let needs_rebuild = state.needs_rebuild;
    let needs_restyle = state.needs_restyle;
    let needs_relayout = state.needs_relayout;
    let has_pending_events = !event_rx.is_empty();

    // Short-circuit: if any cheap flag is set, the scheduler already knows
    // to schedule another paint, so the arena walk would be wasted work.
    let any_node_paint_dirty =
        if needs_rebuild || needs_restyle || needs_relayout || has_pending_events {
            false
        } else {
            state.arena.iter().any(|(_, element)| {
                element.dirty.intersects(
                    unshit_core::dirty::DirtyFlags::PAINT
                        | unshit_core::dirty::DirtyFlags::SUBTREE_PAINT,
                )
            })
        };

    crate::frame_pacer::DirtySignals {
        needs_rebuild,
        needs_restyle,
        needs_relayout,
        has_pending_events,
        any_node_paint_dirty,
    }
}

impl App {
    pub fn new(config: AppConfig, tree_fn: impl Fn() -> ElementTree + 'static) -> Self {
        let (event_tx, event_rx) = flume::unbounded();
        Self {
            config,
            tree_fn: Box::new(tree_fn),
            state: None,
            event_tx,
            event_rx,
            proxy_cell: Arc::new(OnceLock::new()),
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
        for event in self.event_rx.try_iter() {
            match event {
                ExternalEvent::RequestRebuild => {
                    state.needs_rebuild = true;
                }
                ExternalEvent::RequestRedraw => {
                    // Redraw happens below via request_redraw.
                }
                ExternalEvent::Custom(payload) => {
                    if let Some(ref handler) = self.app.config.on_external_event {
                        handler(payload);
                    }
                    // Custom events typically change app state, so rebuild.
                    state.needs_rebuild = true;
                }
                ExternalEvent::Bytes(data) => {
                    if let Some(ref handler) = self.app.config.on_bytes {
                        (handler)(data);
                    }
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
                    state.needs_rebuild = true;
                }
            }
        }
        state.window.request_redraw();
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
        ));

        let scale_factor = window.scale_factor() as f32;
        log::info!("Display scale factor: {:.2}x", scale_factor);

        if let Some(ref cb) = self.app.config.on_scale_factor {
            cb(scale_factor);
        }

        let mut gpu = pollster::block_on(GpuContext::new(window.clone()));

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
            DwRasterizer::new(font_name)
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
            #[cfg(target_os = "windows")]
            dw_rasterizer,
            interaction: InteractionState::default(),
            needs_rebuild: false,
            needs_restyle: false,
            needs_relayout: false,
            scale_factor,
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
            frame_pacer: crate::frame_pacer::FramePacer::new(),
            #[cfg(debug_assertions)]
            frame_probe: crate::frame_probe::FrameProbe::new(),
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

        match event {
            WindowEvent::CloseRequested => {
                if let Some(ref cb) = self.app.config.on_close {
                    cb();
                }
                if let Some(log) = state.event_log.take() {
                    let json = format!("[{}]", log.join(",\n"));
                    std::fs::write("events.json", json).ok();
                    log::info!("Event recording saved to events.json");
                }
                event_loop.exit();
            }

            WindowEvent::SurfaceResized(new_size) => {
                state.gpu.resize(new_size);
                let new_scale = state.window.scale_factor() as f32;
                if (new_scale - state.scale_factor).abs() > 0.01 {
                    // Scale factor changed (e.g. moved to different monitor)
                    log::info!(
                        "Scale factor changed: {:.2}x -> {:.2}x",
                        state.scale_factor,
                        new_scale
                    );
                    state.scale_factor = new_scale;
                    if let Some(ref cb) = self.app.config.on_scale_factor {
                        cb(new_scale);
                    }
                    state.needs_rebuild = true;
                } else {
                    // Just a resize, only re-layout needed
                    state.needs_relayout = true;
                }
                state.window.request_redraw();
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
                    if let Some(element) = state.arena.get_mut(drag.node_id) {
                        match drag.axis {
                            ScrollbarAxis::Vertical => element.scroll_y = new_scroll,
                            ScrollbarAxis::Horizontal => element.scroll_x = new_scroll,
                        }
                    }
                    state.window.request_redraw();
                } else if state.interaction.dragging {
                    // Active element drag: dispatch DragUpdate (pointer captured)
                    if let Some(handler_node) = state.interaction.drag_target {
                        let origin = state.interaction.drag_origin.unwrap_or(pos);
                        let last = state.interaction.drag_last_pos;
                        let event = DragEvent {
                            phase: DragPhase::Update,
                            x: pos.0,
                            y: pos.1,
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
                            let event = DragEvent {
                                phase: DragPhase::Start,
                                x: pos.0,
                                y: pos.1,
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
                        handle_normal_hover(state, pos);
                    }
                } else {
                    handle_normal_hover(state, pos);
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
                            // Check for scrollbar interaction first
                            let sb_pos = state.interaction.last_cursor_pos;
                            if let Some(hit) = scroll::find_scrollbar_at(
                                &state.arena,
                                state.root,
                                sb_pos.0,
                                sb_pos.1,
                            ) {
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
                                        if let Some(element) = state.arena.get_mut(hit.node_id) {
                                            match hit.axis {
                                                ScrollbarAxis::Vertical => {
                                                    element.scroll_y = new_scroll
                                                }
                                                ScrollbarAxis::Horizontal => {
                                                    element.scroll_x = new_scroll
                                                }
                                            }
                                        }
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
                                    state.interaction.active = Some(hovered);
                                    state.interaction.mousedown_target = Some(hovered);
                                    state.needs_restyle = true;
                                    state.window.request_redraw();
                                }

                                let new_focused = find_focusable_ancestor(
                                    &state.arena,
                                    state.interaction.hovered,
                                )
                                .unwrap_or(NodeId::DANGLING);
                                if new_focused != state.interaction.focused {
                                    state.interaction.focused = new_focused;
                                    state.interaction.focus_via_keyboard = false;
                                    update_focus_context(state);
                                    state.needs_restyle = true;
                                    state.window.request_redraw();
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
                                    let event = DragEvent {
                                        phase: DragPhase::End,
                                        x: pos.0,
                                        y: pos.1,
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
                                // Handle checkbox/radio click before generic dispatch.
                                let input_handled = handle_input_click(state, mousedown_target);
                                let pos = state.interaction.last_cursor_pos;
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

                            if state.interaction.active.is_some() {
                                state.interaction.active = None;
                                state.needs_restyle = true;
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

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == winit::event::ElementState::Pressed {
                    // FIRST: check if focused element captures keyboard input
                    let focused_captures = state
                        .arena
                        .get(state.interaction.focused)
                        .map(|e| e.captures_keyboard || e.computed_style.keyboard_capture)
                        .unwrap_or(false);

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
                            } else {
                                // When a command-level modifier (Ctrl/Alt/Meta) is held,
                                // check registered shortcuts before forwarding to the
                                // capture handler. This lets app hotkeys (Ctrl+T, etc.)
                                // work even when the terminal pane is focused. Plain
                                // keys and Shift-only combos bypass this check so normal
                                // typing and Shift+PageUp/Down still reach the terminal.
                                let has_command_modifier = combo
                                    .modifiers
                                    .intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META);

                                let shortcut_handled = if has_command_modifier {
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
                        // Normal keyboard handling: clipboard, then select, then text input, then shortcuts
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
                            if let Some(combo) =
                                key_combo_from_winit(&event.logical_key, &state.modifiers_state)
                            {
                                // Cancel chord on Escape
                                if combo.key == Key::Escape
                                    && state.shortcut_resolver.is_chord_pending()
                                {
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
                    let (delta_x, delta_y) = match delta {
                        winit::event::MouseScrollDelta::LineDelta(x, y) => {
                            let line_height = 40.0 * state.scale_factor * state.zoom_factor;
                            (x * line_height, y * line_height)
                        }
                        winit::event::MouseScrollDelta::PixelDelta(pos) => {
                            (pos.x as f32, pos.y as f32)
                        }
                    };

                    let scroll_target =
                        scroll::find_scroll_container(&state.arena, state.interaction.hovered);

                    if let Some(target_id) = scroll_target {
                        let max_scroll = compute_max_scroll(&state.arena, &state.taffy, target_id);

                        if let Some(element) = state.arena.get_mut(target_id) {
                            let old_x = element.scroll_x;
                            let old_y = element.scroll_y;
                            // scroll wheel delta_y is negative when scrolling down
                            element.scroll_x = (old_x - delta_x).clamp(0.0, max_scroll.0);
                            element.scroll_y = (old_y - delta_y).clamp(0.0, max_scroll.1);
                            if element.scroll_x != old_x || element.scroll_y != old_y {
                                state.window.request_redraw();
                            }
                        }
                    }

                    // Dispatch Scroll event to element handlers. Walk from the
                    // hovered element up to the root, firing the first handler
                    // found (bubble semantics).
                    let pos = state.interaction.last_cursor_pos;
                    let scroll_evt =
                        unshit_core::event::Event::Scroll(unshit_core::event::ScrollEvent {
                            delta_x,
                            delta_y,
                            x: pos.0,
                            y: pos.1,
                        });
                    let mut node = state.interaction.hovered;
                    while let Some(element) = state.arena.get(node) {
                        let mut handled = false;
                        for (et, handler) in &element.handlers {
                            if *et == unshit_core::event::EventType::Scroll {
                                handler(&scroll_evt);
                                handled = true;
                                state.needs_rebuild = true;
                                state.window.request_redraw();
                                break;
                            }
                        }
                        if handled {
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

                // Coalesce RequestRebuild and input-driven redraws into at
                // most one paint per frame_pacer.min_interval. When the
                // pacer says wait, schedule a WaitUntil and bail out so
                // the event loop sleeps until the coalescing deadline.
                match state.frame_pacer.on_redraw_requested(frame_start) {
                    crate::frame_pacer::PaceDecision::PaintNow => {}
                    crate::frame_pacer::PaceDecision::WaitUntil(deadline) => {
                        event_loop
                            .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
                        state.window.request_redraw();
                        return;
                    }
                }

                let mut metrics = FrameMetrics::default();

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
                    let new_tree = (self.app.tree_fn)();
                    let pending_mounts = unshit_core::reconcile::reconcile(
                        &mut state.arena,
                        &mut state.taffy,
                        state.root,
                        &new_tree.root,
                    );
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
                    state.needs_rebuild = false;
                    state.needs_restyle = false;
                    state.needs_relayout = false;

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
                    relayout_pipeline(
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
                        },
                    );
                }

                let t5 = Instant::now();
                state.gpu.render();
                metrics.gpu_render_us = t5.elapsed().as_micros() as u64;

                if state.gpu.any_canvas_needs_repaint() {
                    state.window.request_redraw();
                }

                // Record this paint in the frame pacer so subsequent
                // redraws are gated until the coalescing interval elapses.
                // Must happen before the post-paint dirty check below so the
                // pacer can compute the next eligible paint deadline from an
                // up-to-date `last_paint` timestamp.
                state.frame_pacer.record_paint(frame_start);

                // Unified wake-time calculation. Three classes of follow-up
                // paint can be scheduled here:
                //   1. Animation-driven wakes: cursor blink, CSS keyframe
                //      animations, CSS transitions. Each has a future
                //      `Instant` at which the next frame is needed.
                //   2. Pending-dirty wakes (issue #52 step 2): any state
                //      that was not drained in this paint (needs_rebuild,
                //      flume still has PTY chunks, a node kept PAINT
                //      dirty). These want the *next* vsync, gated by the
                //      frame pacer's coalescing interval.
                //   3. Canvas-driven redraws: `any_canvas_needs_repaint()`
                //      already called `request_redraw` above and does not
                //      participate in this block.
                //
                // We pick the earliest deadline across (1) and (2) so the
                // event loop wakes exactly once per cycle instead of
                // bouncing between two WaitUntil values.
                //
                // When nothing in either class has work we leave
                // `ControlFlow` at its idle default (`Wait`), preserving
                // zero-CPU idle per the step 2 constraint.
                {
                    let now = Instant::now();
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

                    // Post-paint dirty check: if any state is still dirty
                    // (most commonly because a PTY chunk landed in the flume
                    // channel while we were painting and has not yet been
                    // drained by `proxy_wake_up`) the pacer schedules the
                    // next paint at the vsync cadence. See
                    // `FramePacer::should_schedule_next_paint` and
                    // `DirtySignals`.
                    let dirty = collect_dirty_signals(state, &self.event_rx);
                    let mut dirty_request_redraw = false;
                    if let Some(decision) = state.frame_pacer.should_schedule_next_paint(now, dirty)
                    {
                        dirty_request_redraw = true;
                        match decision {
                            crate::frame_pacer::PaceDecision::PaintNow => {
                                // Interval already elapsed; queue next frame
                                // immediately. No deadline update needed.
                            }
                            crate::frame_pacer::PaceDecision::WaitUntil(deadline) => {
                                next_wake = Some(match next_wake {
                                    Some(current) if current <= deadline => current,
                                    _ => deadline,
                                });
                            }
                        }
                    }

                    if let Some(wake) = next_wake {
                        event_loop
                            .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(wake));
                        state.window.request_redraw();
                    } else if dirty_request_redraw {
                        // Dirty + PaintNow path (pacer interval already
                        // elapsed). Leave ControlFlow on its current value
                        // and just queue the next frame.
                        state.window.request_redraw();
                    }
                }

                metrics.total_us = frame_start.elapsed().as_micros() as u64;
                metrics.rss_bytes = get_rss_bytes();

                // Debug-only per-second frame-time probe. Feeds this frame's
                // duration into a rolling window and emits p50/p95/p99
                // quantiles once per second. Release builds skip the whole
                // block via cfg(debug_assertions); see crate::frame_probe.
                #[cfg(debug_assertions)]
                {
                    state
                        .frame_probe
                        .record_frame(std::time::Duration::from_micros(metrics.total_us));
                    if let Some(snap) = state.frame_probe.maybe_emit(Instant::now()) {
                        log::info!("[FRAME] {}", snap);
                    }
                }

                // Log slow frames
                if metrics.total_us > 8333 {
                    log::warn!("[PERF] {}", metrics);
                } else {
                    log::debug!("[PERF] {}", metrics);
                }

                // Fire the on_frame_metrics callback if registered.
                if let Some(ref cb) = self.app.config.on_frame_metrics {
                    cb(&metrics);
                }

                state.last_metrics = metrics;

                state.frame_count += 1;
                let fps_elapsed = state.fps_timer.elapsed();
                if fps_elapsed.as_millis() >= 1000 {
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
fn handle_normal_hover(state: &mut AppState, pos: (f32, f32)) {
    // Check scrollbar hover
    let sb_hit = scroll::find_scrollbar_at(&state.arena, state.root, pos.0, pos.1);
    let old_visual = state.scrollbar_visual;
    state.scrollbar_visual.set_hover(sb_hit.as_ref());
    if state.scrollbar_visual != old_visual {
        state.window.request_redraw();
    }

    let new_hover = hit_test(&state.arena, state.root, pos.0, pos.1).unwrap_or(NodeId::DANGLING);

    if new_hover != state.interaction.hovered {
        state.interaction.hovered = new_hover;
        apply_cursor_icon(&*state.window, &state.arena, new_hover);
        state.needs_restyle = true;
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
                    state.interaction.focused = id;
                    state.interaction.focus_via_keyboard = true;
                    update_focus_context(state);
                    state.needs_restyle = true;
                    state.window.request_redraw();
                }
            }
        }
        "focus.prev" => {
            let new_focused = prev_focusable(&state.arena, state.root, state.interaction.focused);
            if let Some(id) = new_focused {
                if id != state.interaction.focused {
                    state.interaction.focused = id;
                    state.interaction.focus_via_keyboard = true;
                    update_focus_context(state);
                    state.needs_restyle = true;
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

fn compute_max_scroll(
    arena: &NodeArena,
    taffy: &taffy::TaffyTree<layout::TextMeasureCtx>,
    node_id: NodeId,
) -> (f32, f32) {
    let Some(element) = arena.get(node_id) else {
        return (0.0, 0.0);
    };

    let container_w = element.layout_rect.width;
    let container_h = element.layout_rect.height;

    let content_size = element
        .taffy_node
        .and_then(|tn| taffy.layout(tn).ok())
        .map(|layout| (layout.content_size.width, layout.content_size.height))
        .unwrap_or((0.0, 0.0));

    ((content_size.0 - container_w).max(0.0), (content_size.1 - container_h).max(0.0))
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
) {
    if let Some(tn) = arena.get(root).and_then(|e| e.taffy_node) {
        layout::compute_layout(taffy, tn, width, height, font_system, cache);
        layout::read_layout_results(arena, taffy, root, 0.0, 0.0);
    }
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
    fn app_config_on_cell_metrics_defaults_to_none() {
        let config = AppConfig::default();
        assert!(config.on_cell_metrics.is_none());
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
        };
        assert_eq!(m.quad_count, 128);
        assert_eq!(m.glyph_count, 512);
        assert!((m.atlas_fill_ratio - 0.75).abs() < f32::EPSILON);
        assert_eq!(m.gpu_upload_bytes, 8192);
        assert_eq!(m.damage_area_px, 1920 * 1080);
    }
}
