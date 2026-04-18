use std::sync::Arc;
use std::time::Duration;

use cosmic_text::{FontSystem, SwashCache};
use unshit_renderer::batch::Rasterizer;
#[cfg(target_os = "windows")]
use unshit_renderer::dw_rasterizer::DwRasterizer;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::pump_events::EventLoopExtPumpEvents;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

#[cfg(target_os = "windows")]
use winit::event_loop::EventLoopBuilder;
#[cfg(target_os = "windows")]
use winit::platform::windows::EventLoopBuilderExtWindows;

use unshit_core::element::*;
use unshit_core::event::*;
use unshit_core::id::NodeId;
use unshit_core::layout::{TextMeasureCache, TextMeasureCtx};
use unshit_core::scroll::ScrollbarVisualState;
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::tree::NodeArena;
use unshit_renderer::batch::{self, BatchCache, ShapeCache, ShapedTextCache};
use unshit_renderer::gpu::GpuContext;

use crate::query::{matches_selector, snapshot_from};
use unshit_core::build::{
    build_tree_from_def, mark_layout_dirty, resolve_all_styles, run_layout_pipeline,
    scale_all_styles,
};

/// A windowed integration test harness that creates a real OS window,
/// initializes GPU rendering on it, and supports OS-level input injection.
///
/// Unlike `TestHarness` (which is headless), this opens a visible window
/// and renders to a real surface. It uses winit's `pump_app_events` to
/// step the event loop without blocking, enabling frame-by-frame control
/// from test code.
pub struct WindowedTest {
    // winit event loop (must be owned for pump_app_events)
    event_loop: EventLoop,

    // App state (populated after first pump creates the window)
    state: Option<WindowedState>,

    // Config captured at construction, consumed during init
    init: Option<WindowedInit>,
}

struct WindowedInit {
    css: String,
    tree_fn: Box<dyn Fn() -> ElementTree>,
    width: u32,
    height: u32,
}

struct WindowedState {
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
    shaped_cache: ShapedTextCache,
    batch_cache: BatchCache,
    shape_cache: ShapeCache,
    interaction: InteractionState,
    measure_cache: TextMeasureCache,
    scale_factor: f32,
    needs_restyle: bool,
    needs_relayout: bool,
    width: u32,
    height: u32,
    scrollbar_visual: ScrollbarVisualState,
}

/// Temporary handler passed to pump_app_events for initialization.
/// On `resumed()`, it creates the window, GPU context, and full app state.
struct InitHandler<'a> {
    state: &'a mut Option<WindowedState>,
    init: &'a mut Option<WindowedInit>,
}

impl ApplicationHandler for InitHandler<'_> {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let Some(init) = self.init.take() else {
            return;
        };

        let attrs = WindowAttributes::default()
            .with_title("unshit-test windowed")
            .with_surface_size(LogicalSize::new(init.width, init.height));

        let window: Arc<dyn Window> = Arc::from(event_loop.create_window(attrs).unwrap());
        let scale_factor = window.scale_factor() as f32;

        let gpu = pollster::block_on(GpuContext::new(window.clone()));

        let stylesheet = CompiledStylesheet::parse(&init.css);
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();

        let element_tree = (init.tree_fn)();
        let root =
            build_tree_from_def(&element_tree.root, &mut arena, &mut taffy, NodeId::DANGLING);

        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
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

        *self.state = Some(WindowedState {
            window,
            gpu,
            arena,
            taffy,
            root,
            stylesheet,
            font_system,
            swash_cache,
            #[cfg(target_os = "windows")]
            dw_rasterizer: DwRasterizer::new("Consolas"),
            shaped_cache: ShapedTextCache::new(),
            batch_cache: BatchCache::new(),
            shape_cache: ShapeCache::new(),
            interaction: InteractionState::default(),
            measure_cache,
            scale_factor,
            needs_restyle: false,
            needs_relayout: false,
            width: init.width,
            height: init.height,
            scrollbar_visual: ScrollbarVisualState::default(),
        });
    }

    fn window_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
    }
}

struct PumpHandler<'a> {
    state: &'a mut WindowedState,
}

impl ApplicationHandler for PumpHandler<'_> {
    fn can_create_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::PointerMoved { position, .. } => {
                let pos = (position.x as f32, position.y as f32);
                self.state.interaction.last_cursor_pos = pos;

                let new_hover = hit_test(&self.state.arena, self.state.root, pos.0, pos.1)
                    .unwrap_or(NodeId::DANGLING);

                if new_hover != self.state.interaction.hovered {
                    self.state.interaction.hovered = new_hover;
                    self.state.needs_restyle = true;
                }
            }
            WindowEvent::PointerButton { state: button_state, button, .. } => {
                use winit::event::ElementState;
                if button.mouse_button() == Some(winit::event::MouseButton::Left) {
                    match button_state {
                        ElementState::Pressed => {
                            if !self.state.interaction.hovered.is_dangling() {
                                self.state.interaction.active =
                                    Some(self.state.interaction.hovered);
                                self.state.needs_restyle = true;
                            }
                        }
                        ElementState::Released => {
                            if self.state.interaction.active.is_some() {
                                self.state.interaction.active = None;
                                self.state.needs_restyle = true;
                            }
                        }
                    }
                }
            }
            WindowEvent::SurfaceResized(new_size) => {
                self.state.gpu.resize(new_size);
                self.state.needs_relayout = true;
            }
            _ => {}
        }
    }
}

impl WindowedTest {
    /// Create a new windowed test harness.
    ///
    /// This creates the event loop immediately but defers window creation
    /// until the first `pump()` call, which triggers the `resumed()` callback.
    pub fn new(
        css: &str,
        tree_fn: impl Fn() -> ElementTree + 'static,
        width: u32,
        height: u32,
    ) -> Self {
        // Use any_thread(true) on Windows so tests can create the event loop
        // from non-main threads (Rust's test harness spawns worker threads).
        #[cfg(target_os = "windows")]
        let event_loop = EventLoopBuilder::default().with_any_thread(true).build().unwrap();

        #[cfg(not(target_os = "windows"))]
        let event_loop = EventLoop::new().unwrap();

        let mut test = Self {
            event_loop,
            state: None,
            init: Some(WindowedInit {
                css: css.to_string(),
                tree_fn: Box::new(tree_fn),
                width,
                height,
            }),
        };

        // Pump once to trigger resumed() and create the window
        test.init_window();
        test
    }

    /// Pump the event loop once to trigger the `resumed()` callback
    /// and create the window + GPU context.
    fn init_window(&mut self) {
        let mut handler = InitHandler { state: &mut self.state, init: &mut self.init };

        self.event_loop.pump_app_events(Some(Duration::from_millis(100)), &mut handler);

        assert!(self.state.is_some(), "WindowedTest: failed to create window during init pump");
    }

    /// Inject a mouse move to the given coordinates (relative to the window's
    /// client area). Uses OS-level SetCursorPos, then pumps the event loop
    /// to let winit deliver the CursorMoved event.
    pub fn inject_mouse_move(&mut self, x: f32, y: f32) {
        let state = self.state.as_ref().expect("not initialized");
        let pos = state.window.outer_position().unwrap_or_default();
        let screen_x = pos.x + x as i32;
        let screen_y = pos.y + y as i32;
        crate::os_input::set_cursor_pos(screen_x, screen_y);
        // OS needs time to synthesize and deliver the WM_MOUSEMOVE message
        std::thread::sleep(Duration::from_millis(16));
    }

    /// Inject a left mouse button press via OS-level SendInput.
    pub fn inject_mouse_down(&mut self) {
        crate::os_input::send_mouse_down();
        std::thread::sleep(Duration::from_millis(16));
    }

    /// Inject a left mouse button release via OS-level SendInput.
    pub fn inject_mouse_up(&mut self) {
        crate::os_input::send_mouse_up();
        std::thread::sleep(Duration::from_millis(16));
    }

    /// Inject a full click: mouse down, pump, mouse up, pump.
    pub fn inject_click(&mut self, x: f32, y: f32) {
        self.inject_mouse_move(x, y);
        self.pump(1);
        self.inject_mouse_down();
        self.pump(1);
        self.inject_mouse_up();
        self.pump(1);
    }

    /// Inject a mouse wheel event. `delta` is in multiples of WHEEL_DELTA (120).
    pub fn inject_mouse_wheel(&mut self, delta: i32) {
        crate::os_input::send_mouse_wheel(delta);
        std::thread::sleep(Duration::from_millis(16));
    }

    /// Pump the event loop for `frames` iterations, processing OS events and
    /// re-running the style/layout/render pipeline each frame.
    pub fn pump(&mut self, frames: usize) {
        for _ in 0..frames {
            self.pump_one_frame();
        }
    }

    fn pump_one_frame(&mut self) {
        // Split borrows: take state out temporarily so we can pass
        // &mut state to PumpHandler while also calling pump_app_events
        // on self.event_loop.
        let mut state = self.state.take().expect("not initialized");
        {
            let mut handler = PumpHandler { state: &mut state };
            self.event_loop.pump_app_events(Some(Duration::from_millis(1)), &mut handler);
        }

        if state.needs_restyle {
            resolve_all_styles(
                &mut state.arena,
                &state.stylesheet,
                state.root,
                state.interaction.hovered,
                state.interaction.active,
                state.interaction.focused,
            );
            scale_all_styles(&mut state.arena, state.root, state.scale_factor);
            mark_layout_dirty(&mut state.arena, state.root);

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

            state.needs_restyle = false;
            state.needs_relayout = false;
        } else if state.needs_relayout {
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
            state.needs_relayout = false;
        }

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
            None,
            &state.scrollbar_visual,
            state.interaction.focused,
            &mut state.batch_cache,
            None,
        );
        state.batch_cache.commit_frame();
        // Mirror production: advance the double buffered shape caches so
        // windowed tests exercise the same per frame eviction semantics as
        // `unshit-app` does in its main render pump (see
        // `crates/unshit-app/src/app.rs` next to `batch_cache.commit_frame`).
        state.shaped_cache.finish_frame(state.gpu.glyph_atlas.generation);
        state.shape_cache.finish_frame();
        batch::clear_paint_flags_subtree(&mut state.arena, state.root);
        state.gpu.render();

        state.window.request_redraw();

        self.state = Some(state);
    }

    /// Returns the currently hovered element's NodeId.
    pub fn hovered(&self) -> NodeId {
        self.state.as_ref().map(|s| s.interaction.hovered).unwrap_or(NodeId::DANGLING)
    }

    /// Returns the currently active (pressed) element, if any.
    pub fn active(&self) -> Option<NodeId> {
        self.state.as_ref().and_then(|s| s.interaction.active)
    }

    /// Returns a reference to the node arena for inspection.
    pub fn arena(&self) -> &NodeArena {
        &self.state.as_ref().expect("not initialized").arena
    }

    /// Returns the root NodeId.
    pub fn root(&self) -> NodeId {
        self.state.as_ref().expect("not initialized").root
    }

    /// Returns a reference to the window.
    pub fn window(&self) -> &dyn Window {
        &*self.state.as_ref().expect("not initialized").window
    }

    /// Returns the window dimensions.
    pub fn size(&self) -> (u32, u32) {
        let s = self.state.as_ref().expect("not initialized");
        (s.width, s.height)
    }

    /// Check whether the windowed test is properly initialized.
    pub fn is_initialized(&self) -> bool {
        self.state.is_some()
    }

    /// Query an element by simple selector (same syntax as TestHarness::query).
    pub fn query(&self, selector: &str) -> Option<crate::ElementSnapshot> {
        let state = self.state.as_ref()?;
        for (node_id, element) in state.arena.iter() {
            if matches_selector(selector, element) {
                return Some(snapshot_from(node_id, element));
            }
        }
        None
    }

    /// Query all elements matching a simple selector.
    pub fn query_all(&self, selector: &str) -> Vec<crate::ElementSnapshot> {
        let Some(state) = self.state.as_ref() else {
            return Vec::new();
        };
        state
            .arena
            .iter()
            .filter(|(_, element)| matches_selector(selector, element))
            .map(|(node_id, element)| snapshot_from(node_id, element))
            .collect()
    }
}
