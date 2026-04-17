use cosmic_text::{FontSystem, SwashCache};
use unshit_core::dirty::DirtyFlags;
use unshit_core::element::{SelectOption, SelectState, Tag, *};
use unshit_core::event::*;
use unshit_core::id::NodeId;
use unshit_core::layout::{self, TextMeasureCache, TextMeasureCtx};
use unshit_core::scroll::ScrollbarVisualState;
use unshit_core::style::cascade;
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::style::pseudo::PseudoSideTable;
use unshit_core::tree::NodeArena;
use unshit_renderer::batch::Rasterizer;
use unshit_renderer::batch::{self, BatchCache, ShapeCache, ShapedTextCache};
#[cfg(target_os = "windows")]
use unshit_renderer::dw_rasterizer::DwRasterizer;
use unshit_renderer::gpu::GpuContext;

use crate::trace::TraceRecorder;

pub struct TestHarness {
    pub(crate) arena: NodeArena,
    pub(crate) taffy: taffy::TaffyTree<TextMeasureCtx>,
    pub(crate) root: NodeId,
    pub(crate) stylesheet: CompiledStylesheet,
    pub(crate) font_system: FontSystem,
    pub(crate) swash_cache: SwashCache,
    #[cfg(target_os = "windows")]
    pub(crate) dw_rasterizer: DwRasterizer,
    pub(crate) shaped_cache: ShapedTextCache,
    pub(crate) batch_cache: BatchCache,
    pub(crate) shape_cache: ShapeCache,
    pub(crate) gpu: Option<GpuContext>,
    pub(crate) interaction: InteractionState,
    pub(crate) scale_factor: f32,
    pub(crate) measure_cache: TextMeasureCache,
    pub(crate) needs_restyle: bool,
    pub(crate) needs_relayout: bool,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) scrollbar_visual: ScrollbarVisualState,
    pub(crate) pseudo_table: PseudoSideTable,
    pub(crate) trace: TraceRecorder,
}

impl TestHarness {
    /// Create a new headless test harness.
    ///
    /// Parses the given CSS, builds the element tree from `tree_fn`,
    /// resolves styles, and runs the initial layout pass.
    pub fn new(css: &str, tree_fn: impl Fn() -> ElementTree, width: f32, height: f32) -> Self {
        let stylesheet = CompiledStylesheet::parse(css);
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();

        let element_tree = tree_fn();
        let root =
            build_tree_from_def(&element_tree.root, &mut arena, &mut taffy, NodeId::DANGLING);

        let interaction = InteractionState::default();

        resolve_all_styles(
            &mut arena,
            &stylesheet,
            root,
            interaction.hovered,
            interaction.active,
            interaction.focused,
            interaction.focus_via_keyboard,
        );

        let mut pseudo_table = PseudoSideTable::new();
        unshit_core::build::resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            interaction.hovered,
            interaction.active,
            interaction.focused,
            &mut pseudo_table,
        );

        let mut measure_cache = TextMeasureCache::new();
        run_layout_pipeline(
            &mut arena,
            &mut taffy,
            root,
            &mut font_system,
            width,
            height,
            &mut measure_cache,
        );

        let mut trace = TraceRecorder::new();
        if crate::test_app::env_is_truthy("UNSHIT_TEST_TRACE") {
            trace.enable();
        }

        Self {
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
            gpu: None,
            interaction,
            scale_factor: 1.0,
            measure_cache,
            needs_restyle: false,
            needs_relayout: false,
            width,
            height,
            scrollbar_visual: ScrollbarVisualState::default(),
            pseudo_table,
            trace,
        }
    }

    /// Reconcile the tree against a new definition, then restyle and relayout.
    pub fn rebuild(&mut self, tree_fn: impl Fn() -> ElementTree) {
        let new_tree = tree_fn();
        unshit_core::reconcile::reconcile(
            &mut self.arena,
            &mut self.taffy,
            self.root,
            &new_tree.root,
        );

        // Reconcile may deallocate nodes that interaction state points to
        if self.arena.get(self.interaction.hovered).is_none() {
            self.interaction.hovered = NodeId::DANGLING;
        }
        if let Some(active) = self.interaction.active {
            if self.arena.get(active).is_none() {
                self.interaction.active = None;
            }
        }
        if self.arena.get(self.interaction.focused).is_none() {
            self.interaction.focused = NodeId::DANGLING;
        }

        // Post-reconcile cascade: use the dirty-flag short-circuit to skip
        // clean subtrees. This is safe because the reconciler has already set
        // STYLE and SUBTREE_STYLE on every node that changed.
        unshit_core::build::resolve_dirty_styles_with_transitions(
            &mut self.arena,
            &self.stylesheet,
            self.root,
            self.interaction.hovered,
            self.interaction.active,
            self.interaction.focused,
            self.interaction.focus_via_keyboard,
            None,
            None,
        );
        unshit_core::build::resolve_pseudo_elements(
            &mut self.arena,
            &mut self.taffy,
            &self.stylesheet,
            self.root,
            self.interaction.hovered,
            self.interaction.active,
            self.interaction.focused,
            &mut self.pseudo_table,
        );
        if (self.scale_factor - 1.0).abs() >= 0.001 {
            scale_all_styles(&mut self.arena, self.root, self.scale_factor);
        }
        run_layout_pipeline(
            &mut self.arena,
            &mut self.taffy,
            self.root,
            &mut self.font_system,
            self.width,
            self.height,
            &mut self.measure_cache,
        );
    }

    /// Advance one frame: re-resolve styles and/or re-layout if dirty.
    pub fn step(&mut self) {
        self.trace.tick_frame();
        if self.needs_restyle {
            resolve_all_styles(
                &mut self.arena,
                &self.stylesheet,
                self.root,
                self.interaction.hovered,
                self.interaction.active,
                self.interaction.focused,
                self.interaction.focus_via_keyboard,
            );

            unshit_core::build::resolve_pseudo_elements(
                &mut self.arena,
                &mut self.taffy,
                &self.stylesheet,
                self.root,
                self.interaction.hovered,
                self.interaction.active,
                self.interaction.focused,
                &mut self.pseudo_table,
            );

            if (self.scale_factor - 1.0).abs() >= 0.001 {
                scale_all_styles(&mut self.arena, self.root, self.scale_factor);
            }

            mark_layout_dirty(&mut self.arena, self.root);

            run_layout_pipeline(
                &mut self.arena,
                &mut self.taffy,
                self.root,
                &mut self.font_system,
                self.width,
                self.height,
                &mut self.measure_cache,
            );

            self.needs_restyle = false;
            self.needs_relayout = false;
        } else if self.needs_relayout {
            run_layout_pipeline(
                &mut self.arena,
                &mut self.taffy,
                self.root,
                &mut self.font_system,
                self.width,
                self.height,
                &mut self.measure_cache,
            );

            self.needs_relayout = false;
        }
    }

    /// Returns the root node ID.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Returns a reference to the compiled stylesheet.
    ///
    /// Useful for asserting on parsed `@font-face` rules or custom
    /// properties in tests.
    pub fn stylesheet(&self) -> &CompiledStylesheet {
        &self.stylesheet
    }

    /// Returns a mutable reference to the cosmic-text `FontSystem` backing
    /// this harness. Exposed for tests that need to load custom fonts or
    /// inspect the font database.
    pub fn font_system_mut(&mut self) -> &mut cosmic_text::FontSystem {
        &mut self.font_system
    }

    /// Returns a reference to the cosmic-text `FontSystem` backing this
    /// harness.
    pub fn font_system(&self) -> &cosmic_text::FontSystem {
        &self.font_system
    }

    /// Returns a reference to the node arena for direct inspection.
    pub fn arena(&self) -> &NodeArena {
        &self.arena
    }

    /// Returns a mutable reference to the node arena for direct mutation
    /// (e.g. setting scroll offsets in tests).
    pub fn arena_mut(&mut self) -> &mut NodeArena {
        &mut self.arena
    }

    /// Returns a mutable reference to the taffy tree (for direct reconciliation tests).
    pub fn taffy_mut(&mut self) -> &mut taffy::TaffyTree<TextMeasureCtx> {
        &mut self.taffy
    }

    /// Returns the currently hovered element's NodeId.
    pub fn hovered(&self) -> NodeId {
        self.interaction.hovered
    }

    /// Returns the currently active (pressed) element, if any.
    pub fn active(&self) -> Option<NodeId> {
        self.interaction.active
    }

    /// Returns the currently focused element's NodeId.
    pub fn focused(&self) -> NodeId {
        self.interaction.focused
    }

    /// Returns the current text selection, if any.
    pub fn text_selection(&self) -> Option<&TextSelection> {
        self.interaction.text_selection.as_ref()
    }

    /// Returns the class list of the currently hovered element.
    pub fn hovered_classes(&self) -> Vec<String> {
        self.arena.get(self.interaction.hovered).map(|e| e.classes.to_vec()).unwrap_or_default()
    }

    /// Set the DPI scale factor and mark for restyle.
    pub fn set_scale_factor(&mut self, scale: f32) {
        self.scale_factor = scale;
        self.needs_restyle = true;
    }

    /// Enable GPU rendering for pixel-level testing (Tier 2).
    /// Creates a headless GpuContext with an offscreen render target.
    ///
    /// Panics if no GPU adapter is available. For a non-panicking
    /// alternative, use `try_with_gpu()`.
    pub fn with_gpu(mut self) -> Self {
        let preferred = parse_backend_env();
        self.gpu = Some(pollster::block_on(GpuContext::new_headless_with_backend(
            self.width as u32,
            self.height as u32,
            preferred,
        )));
        self
    }

    /// Try to enable GPU rendering. Returns `true` if a GPU context was
    /// successfully created, `false` otherwise. Does not panic.
    ///
    /// Respects the `UNSHIT_TEST_BACKEND` environment variable.
    pub fn try_with_gpu(&mut self) -> bool {
        let preferred = parse_backend_env();
        let w = self.width as u32;
        let h = self.height as u32;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pollster::block_on(GpuContext::try_new_headless(w, h, preferred))
        }));
        match result {
            Ok(Some(ctx)) => {
                self.gpu = Some(ctx);
                true
            }
            Ok(None) => {
                eprintln!("[unshit-test] GPU not available, pixel tests will be skipped");
                false
            }
            Err(_) => {
                eprintln!(
                    "[unshit-test] GPU init panicked (likely pipeline creation \
                     failed), pixel tests will be skipped"
                );
                false
            }
        }
    }

    /// Panics with a descriptive message if no GPU context is attached.
    /// Call this at the start of tests that require pixel rendering.
    pub fn require_gpu(&self) {
        if self.gpu.is_some() {
            return;
        }
        let suffix = if std::env::var("UNSHIT_TEST_GPU_REQUIRED").as_deref() == Ok("1") {
            " (UNSHIT_TEST_GPU_REQUIRED=1)"
        } else {
            ""
        };
        panic!(
            "[unshit-test] GPU context required for this test but none is attached{}. \
             Call with_gpu() or try_with_gpu() first.",
            suffix,
        );
    }

    /// Render the current frame to the GPU and return RGBA pixel data.
    /// Panics if `with_gpu()` was not called.
    pub fn render(&mut self) -> Vec<u8> {
        let gpu = self.gpu.as_mut().expect("call with_gpu() before render()");
        gpu.layered_batch.clear();
        self.batch_cache.begin_frame();
        let mut rasterizer = Rasterizer {
            swash: &mut self.swash_cache,
            #[cfg(target_os = "windows")]
            dw: &self.dw_rasterizer,
        };
        batch::build_render_batch(
            &self.arena,
            self.root,
            &mut gpu.layered_batch,
            &mut gpu.glyph_atlas,
            &mut self.font_system,
            &mut rasterizer,
            &mut self.measure_cache,
            &mut self.shaped_cache,
            &mut gpu.svg_cache,
            &mut self.shape_cache,
            self.interaction.text_selection.as_ref(),
            None,
            &self.scrollbar_visual,
            self.interaction.focused,
            &mut self.batch_cache,
        );
        self.batch_cache.commit_frame();
        batch::clear_paint_flags_subtree(&mut self.arena, self.root);
        gpu.render();
        gpu.read_pixels()
    }

    /// Borrow the underlying `GpuContext` for inspection. Panics if the
    /// harness was not created with `with_gpu`.
    pub fn gpu_ref(&self) -> &GpuContext {
        self.gpu.as_ref().expect("call with_gpu() before gpu_ref()")
    }

    /// Check if GPU is attached.
    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    // -- Trace recording API --------------------------------------------------

    /// Enable trace recording. All subsequent actions and assertions will be
    /// captured in the trace timeline.
    pub fn enable_trace(&mut self) {
        self.trace.enable();
    }

    /// Enable screenshot capture at each traced step. Requires GPU to be
    /// attached via `with_gpu()`.
    pub fn enable_trace_screenshots(&mut self) {
        self.trace.enable_screenshots();
    }

    /// Save the recorded trace to `{output_dir}/{test_name}/trace.json`.
    pub fn save_trace(&mut self, test_name: &str) {
        if !self.trace.is_enabled() {
            return;
        }
        self.trace.save(test_name);
    }

    /// Returns a reference to the trace recorder.
    pub fn trace(&self) -> &TraceRecorder {
        &self.trace
    }

    /// Returns a mutable reference to the trace recorder.
    pub fn trace_mut(&mut self) -> &mut TraceRecorder {
        &mut self.trace
    }
}

// ---------------------------------------------------------------------------
// Helper functions (replicate app.rs logic without GPU/window dependencies)
// ---------------------------------------------------------------------------

#[allow(clippy::only_used_in_recursion)]
pub(crate) fn build_tree_from_def(
    def: &ElementDef,
    arena: &mut NodeArena,
    taffy: &mut taffy::TaffyTree<TextMeasureCtx>,
    parent: NodeId,
) -> NodeId {
    let mut element = Element::new(def.tag);
    element.parent = parent;
    element.classes = def.classes.clone();
    element.id = def.id.clone();
    element.key = def.key.clone();
    element.content = def.content.clone();
    element.on_click = def.on_click.clone();
    element.tab_index = def.tab_index;
    element.captures_keyboard = def.captures_keyboard;
    element.on_context_menu = def.on_context_menu.clone();
    element.on_drag = def.on_drag.clone();
    element.on_resize = def.on_resize.clone();
    element.resize_axis = def.resize_axis;
    element.on_pane_resize = def.on_pane_resize.clone();
    element.placeholder = def.placeholder.clone();
    element.on_change = def.on_change.clone();
    element.on_submit = def.on_submit.clone();
    element.memo_key = def.memo_key;
    element.name = def.name.clone();
    element.input_state.input_type = def.input_type;
    if let Some(min) = def.min {
        element.input_state.min = min;
    }
    if let Some(max) = def.max {
        element.input_state.max = max;
    }
    if let Some(step) = def.step {
        element.input_state.step = step;
    }
    element.input_state.checked = def.checked;

    // For select elements: populate SelectState from the def's options list.
    if def.tag == Tag::Select {
        let options: Vec<SelectOption> = def
            .options
            .iter()
            .map(|(v, l)| SelectOption { value: v.clone(), label: l.clone() })
            .collect();
        let selected_index = def.selected_index.unwrap_or(0);
        element.select_state =
            Some(SelectState { open: false, selected_index, highlighted_index: None, options });
    }

    let node_id = arena.alloc(element);

    // For select elements, do not add option children as arena nodes.
    if def.tag == Tag::Select {
        return node_id;
    }

    let mut prev_child = NodeId::DANGLING;
    for child_def in &def.children {
        // Skip Tag::Option children (they are consumed by the select's state)
        if child_def.tag == Tag::Option {
            continue;
        }
        let child_id = build_tree_from_def(child_def, arena, taffy, node_id);

        if let Some(child) = arena.get_mut(child_id) {
            child.prev_sibling = prev_child;
        }

        if prev_child.is_dangling() {
            if let Some(parent_elem) = arena.get_mut(node_id) {
                parent_elem.first_child = child_id;
            }
        } else if let Some(prev) = arena.get_mut(prev_child) {
            prev.next_sibling = child_id;
        }

        if let Some(parent_elem) = arena.get_mut(node_id) {
            parent_elem.last_child = child_id;
        }

        prev_child = child_id;
    }

    node_id
}

pub(crate) fn resolve_all_styles(
    arena: &mut NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
) {
    let new_style = cascade::resolve_style_fv(
        arena,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
    );
    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        element.computed_style = new_style;
    }

    for child_id in children {
        resolve_all_styles(
            arena,
            stylesheet,
            child_id,
            hovered,
            active,
            focused,
            focus_via_keyboard,
        );
    }
}

pub(crate) fn scale_all_styles(arena: &mut NodeArena, node_id: NodeId, scale: f32) {
    if (scale - 1.0).abs() < 0.001 {
        return;
    }

    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        element.computed_style.scale_by(scale);
    }

    for child_id in children {
        scale_all_styles(arena, child_id, scale);
    }
}

pub(crate) fn mark_layout_dirty(arena: &mut NodeArena, node_id: NodeId) {
    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        element.dirty |= DirtyFlags::LAYOUT;
    }

    for child_id in children {
        mark_layout_dirty(arena, child_id);
    }
}

pub(crate) fn run_layout_pipeline(
    arena: &mut NodeArena,
    taffy: &mut taffy::TaffyTree<TextMeasureCtx>,
    root: NodeId,
    font_system: &mut FontSystem,
    width: f32,
    height: f32,
    cache: &mut TextMeasureCache,
) {
    layout::sync_element_to_taffy(arena, taffy, root, font_system, width, height);
    if let Some(tn) = arena.get(root).and_then(|e| e.taffy_node) {
        layout::compute_layout(taffy, tn, width, height, font_system, cache);
        layout::read_layout_results(arena, taffy, root, 0.0, 0.0);
    }
    layout::clear_dirty_flags(arena, root);
    unshit_core::build::dispatch_resize_callbacks(arena, root);
}

/// Parse `UNSHIT_TEST_BACKEND` into a `wgpu::Backends` value.
///
/// Recognised values (case-insensitive): `vulkan`, `dx12`, `metal`, `gl`,
/// `software`. Unrecognised or absent values return `None`, which tells the
/// GPU init code to try all backends.
fn parse_backend_env() -> Option<wgpu::Backends> {
    let val = std::env::var("UNSHIT_TEST_BACKEND").ok()?;
    match val.to_ascii_lowercase().as_str() {
        "vulkan" => Some(wgpu::Backends::VULKAN),
        "dx12" => Some(wgpu::Backends::DX12),
        "metal" => Some(wgpu::Backends::METAL),
        "gl" => Some(wgpu::Backends::GL),
        "software" => {
            // "software" is not a wgpu backend flag; it signals
            // that the caller wants GL (typically with llvmpipe).
            Some(wgpu::Backends::GL)
        }
        other => {
            eprintln!(
                "[unshit-test] unknown UNSHIT_TEST_BACKEND value '{}', using auto-detect",
                other,
            );
            None
        }
    }
}
