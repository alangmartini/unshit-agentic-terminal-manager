//! Kitchen sink: demonstrates every feature shipped in the #74 roadmap.
//!
//! Features covered:
//!   1. Reconciliation/diffing engine (implicit, powers every frame rebuild)
//!   2. Custom GPU canvas (CustomPainter with WGSL shader)
//!   3. Async subscriptions (live tick counter via Subscription API)
//!   4. Text input widget (search bar with on_change / on_submit)
//!   5. Z-index / overlay system (modal dialog on the Modal layer)
//!   6. Scrollbar indicators (scrollable card grid with overflow: scroll)
//!   7. Keyboard shortcuts (Tab / Shift+Tab focus cycling across cards)
//!   8. Drag interaction primitives (draggable box showing coordinates)
//!   9. CSS transitions (animated hover effects on every card)
//!  10. CSS Grid layout (2-column card grid with span)
//!
//! Run with:
//!   cargo run -p unshit --features async --example kitchen_sink

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use unshit::app::{App, AppConfig, ExternalEvent, Subscription};
use unshit::core::element::*;
use unshit::renderer::canvas::{CustomPainter, PaintContext};

// ---------------------------------------------------------------------------
// Feature 2: Custom GPU painter
// ---------------------------------------------------------------------------

struct GradientPainter {
    pipeline: Mutex<Option<wgpu::RenderPipeline>>,
}

impl GradientPainter {
    fn new() -> Self {
        Self { pipeline: Mutex::new(None) }
    }
}

impl CustomPainter for GradientPainter {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        _rect: LayoutRect,
        sample_count: u32,
    ) {
        let mut lock = self.pipeline.lock().unwrap();
        if lock.is_some() {
            return;
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kitchen_sink gradient"),
            source: wgpu::ShaderSource::Wgsl(GRADIENT_SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kitchen_sink layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kitchen_sink pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        *lock = Some(pipeline);
    }

    fn paint<'pass>(
        &'pass self,
        _ctx: &PaintContext<'_>,
        render_pass: &mut wgpu::RenderPass<'pass>,
    ) {
        let lock = self.pipeline.lock().unwrap();
        let Some(ref pipeline) = *lock else { return };
        render_pass.set_pipeline(pipeline);
        render_pass.draw(0..4, 0..1);
    }

    fn needs_repaint(&self) -> bool {
        true
    }
}

const GRADIENT_SHADER: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
    );
    return vec4<f32>(pos[idx], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / vec2<f32>(400.0, 200.0);
    let r = (sin(uv.x * 6.28 + uv.y * 3.14) + 1.0) * 0.5;
    let g = (cos(uv.y * 6.28 - uv.x * 1.57) + 1.0) * 0.3;
    let b = 0.6 + 0.3 * sin(uv.x * 3.14);
    return vec4<f32>(r * 0.3, g + 0.15, b * 0.7, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// CSS stylesheet
// ---------------------------------------------------------------------------

const CSS: &str = r#"
    :root {
        --bg: rgba(13, 17, 23, 0.95);
        --bg-card: rgba(22, 27, 34, 0.95);
        --bg-card-hover: rgba(30, 37, 46, 0.95);
        --accent: #10b981;
        --accent-light: #34d399;
        --accent-glow: rgba(16, 185, 129, 0.15);
        --text: #e6edf3;
        --text-dim: #8b949e;
        --text-muted: #484f58;
        --border: rgba(255, 255, 255, 0.06);
        --border-accent: rgba(16, 185, 129, 0.3);
        --error: #f87171;
        --warning: #fbbf24;
        --info: #60a5fa;
    }

    /* ---- Shell ---- */
    .root {
        display: flex;
        flex-direction: column;
        width: 100%;
        height: 100%;
        background: var(--bg);
        padding: 20px 24px;
        gap: 16px;
        position: relative;
    }

    /* ---- Header ---- */
    .header {
        display: flex;
        align-items: center;
        gap: 14px;
        flex-shrink: 0;
    }
    .title {
        color: var(--text);
        font-size: 22px;
        font-weight: bold;
    }
    .badge {
        display: flex;
        align-items: center;
        padding: 3px 10px;
        background: var(--accent-glow);
        border-radius: 6px;
        color: var(--accent-light);
        font-size: 11px;
        font-weight: bold;
        letter-spacing: 1px;
    }
    .clock {
        color: var(--accent-light);
        font-size: 14px;
        font-weight: bold;
    }
    .hint {
        color: var(--text-muted);
        font-size: 12px;
    }

    /* ---- Search bar (Feature 4: Text Input) ---- */
    .search-row {
        display: flex;
        align-items: center;
        gap: 12px;
        flex-shrink: 0;
    }
    .search-input {
        flex-grow: 1;
        height: 36px;
        padding: 0px 12px;
        background: rgba(255, 255, 255, 0.04);
        border-radius: 8px;
        border-width: 1px;
        border-color: var(--border);
        color: var(--text);
        font-size: 14px;
        caret-color: var(--accent-light);
        placeholder-color: var(--text-muted);
        transition: border-color 200ms ease-out;
    }
    .search-input:focus {
        border-color: var(--accent);
        outline-color: var(--accent);
        outline-width: 1px;
    }
    .submitted-text {
        color: var(--text-dim);
        font-size: 12px;
        flex-shrink: 0;
    }

    /* ---- Feature 10: CSS Grid card layout ---- */
    .card-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 14px;
        overflow: scroll;
        flex-grow: 1;
        padding: 2px;
    }

    /* ---- Cards with Feature 9: CSS Transitions ---- */
    .card {
        display: flex;
        flex-direction: column;
        background: var(--bg-card);
        border-radius: 12px;
        border-width: 1px;
        border-color: var(--border);
        padding: 16px;
        gap: 10px;
        transition: background 200ms ease-out, border-color 200ms ease-out, box-shadow 300ms ease-out;
    }
    .card:hover {
        background: var(--bg-card-hover);
        border-color: var(--border-accent);
        box-shadow: 0px 0px 20px rgba(16, 185, 129, 0.08);
    }
    .card:focus {
        outline-color: var(--accent);
        outline-width: 2px;
        outline-offset: 2px;
    }

    .card-span-2 {
        grid-column: span 2;
    }

    .card-title-row {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .card-num {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 22px;
        height: 22px;
        background: var(--accent-glow);
        border-radius: 11px;
        color: var(--accent-light);
        font-size: 11px;
        font-weight: bold;
    }
    .card-label {
        color: var(--text);
        font-size: 14px;
        font-weight: bold;
    }
    .card-desc {
        color: var(--text-dim);
        font-size: 12px;
        line-height: 1.4;
    }

    /* ---- Feature 3: Async subscription counter ---- */
    .counter-value {
        color: var(--accent-light);
        font-size: 42px;
        font-weight: bold;
    }
    .counter-unit {
        color: var(--text-muted);
        font-size: 12px;
        letter-spacing: 2px;
        font-weight: bold;
    }

    /* ---- Feature 2: Canvas ---- */
    .canvas-area {
        width: 100%;
        height: 120px;
        border-radius: 8px;
        border-width: 1px;
        border-color: rgba(255, 255, 255, 0.04);
    }

    /* ---- Feature 8: Drag zone ---- */
    .drag-zone {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 6px;
        width: 100%;
        height: 100px;
        background: rgba(255, 255, 255, 0.03);
        border-radius: 8px;
        border-width: 1px;
        border-color: rgba(96, 165, 250, 0.2);
        cursor: grab;
        transition: background 150ms ease-out, border-color 150ms ease-out;
    }
    .drag-zone:hover {
        background: rgba(96, 165, 250, 0.06);
        border-color: rgba(96, 165, 250, 0.4);
    }
    .drag-coords {
        color: var(--info);
        font-size: 13px;
        font-weight: bold;
    }
    .drag-hint {
        color: var(--text-muted);
        font-size: 11px;
    }

    /* ---- Feature 9: Transition showcase ---- */
    .transition-box {
        display: flex;
        align-items: center;
        justify-content: center;
        height: 60px;
        background: rgba(251, 191, 36, 0.1);
        border-radius: 8px;
        border-width: 1px;
        border-color: rgba(251, 191, 36, 0.2);
        color: var(--warning);
        font-size: 13px;
        font-weight: bold;
        opacity: 0.7;
        transition: opacity 400ms ease-in-out, background 400ms ease-in-out, border-color 300ms ease-out;
    }
    .transition-box:hover {
        opacity: 1.0;
        background: rgba(251, 191, 36, 0.25);
        border-color: rgba(251, 191, 36, 0.5);
    }

    /* ---- Feature 5: Modal overlay (z-index / layer) ---- */
    .modal-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 10px 20px;
        background: var(--accent);
        border-radius: 8px;
        color: #000000;
        font-size: 13px;
        font-weight: bold;
        cursor: pointer;
        transition: background 150ms ease-out;
    }
    .modal-btn:hover {
        background: var(--accent-light);
    }
    .modal-btn:focus {
        outline-color: var(--accent-light);
        outline-width: 2px;
        outline-offset: 3px;
    }

    .modal-backdrop {
        position: absolute;
        top: 0px;
        left: 0px;
        width: 100%;
        height: 100%;
        background: rgba(0, 0, 0, 0.6);
        display: flex;
        align-items: center;
        justify-content: center;
        layer: modal;
        render-target: modal;
    }
    .modal-dialog {
        display: flex;
        flex-direction: column;
        width: 400px;
        padding: 24px;
        background: rgba(22, 27, 34, 1.0);
        border-radius: 16px;
        border-width: 1px;
        border-color: var(--border-accent);
        gap: 16px;
        box-shadow: 0px 8px 32px rgba(0, 0, 0, 0.4);
    }
    .modal-title {
        color: var(--text);
        font-size: 18px;
        font-weight: bold;
    }
    .modal-body {
        color: var(--text-dim);
        font-size: 13px;
        line-height: 1.5;
    }
    .modal-close {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 8px 20px;
        background: rgba(248, 113, 113, 0.15);
        border-radius: 8px;
        color: var(--error);
        font-size: 13px;
        font-weight: bold;
        cursor: pointer;
        transition: background 150ms ease-out;
    }
    .modal-close:hover {
        background: rgba(248, 113, 113, 0.3);
    }

    /* ---- Feature 7: Keyboard navigation info ---- */
    .kbd {
        display: flex;
        align-items: center;
        padding: 3px 8px;
        background: rgba(255, 255, 255, 0.06);
        border-radius: 4px;
        border-width: 1px;
        border-color: rgba(255, 255, 255, 0.1);
        color: var(--text-dim);
        font-size: 11px;
        font-weight: bold;
    }
    .shortcut-row {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .shortcut-label {
        color: var(--text-muted);
        font-size: 12px;
    }

    /* ---- Nested scroll list (Feature 6: Scrollbars) ---- */
    .scroll-list {
        display: flex;
        flex-direction: column;
        gap: 6px;
        overflow: scroll;
        height: 120px;
        padding: 4px;
    }
    .scroll-item {
        display: flex;
        align-items: center;
        padding: 8px 12px;
        background: rgba(255, 255, 255, 0.02);
        border-radius: 6px;
        color: var(--text-dim);
        font-size: 12px;
        flex-shrink: 0;
        transition: background 150ms ease-out;
    }
    .scroll-item:hover {
        background: rgba(255, 255, 255, 0.06);
    }

    /* ---- Feature tags ---- */
    .tag-row {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .tag {
        display: flex;
        align-items: center;
        padding: 2px 8px;
        border-radius: 4px;
        font-size: 10px;
        font-weight: bold;
        letter-spacing: 0.5px;
    }
    .tag-green  { background: rgba(16, 185, 129, 0.12); color: var(--accent-light); }
    .tag-blue   { background: rgba(96, 165, 250, 0.12); color: var(--info); }
    .tag-yellow { background: rgba(251, 191, 36, 0.12); color: var(--warning); }
    .tag-red    { background: rgba(248, 113, 113, 0.12); color: var(--error); }

    /* ---- Focusable buttons ---- */
    .focus-btn {
        display: flex;
        align-items: center;
        padding: 6px 14px;
        background: rgba(255, 255, 255, 0.04);
        border-radius: 6px;
        border-width: 1px;
        border-color: var(--border);
        color: var(--text-dim);
        font-size: 12px;
        cursor: pointer;
        transition: background 150ms ease-out, border-color 150ms ease-out;
    }
    .focus-btn:hover {
        background: rgba(255, 255, 255, 0.08);
        border-color: rgba(255, 255, 255, 0.12);
    }
    .focus-btn:focus {
        outline-color: var(--accent);
        outline-width: 2px;
        outline-offset: 2px;
        border-color: var(--accent);
    }

    /* ---- Footer ---- */
    .footer {
        display: flex;
        align-items: center;
        gap: 16px;
        flex-shrink: 0;
    }
"#;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error"),
    )
    .init();

    // ---- Shared state ----
    let ticks = Arc::new(AtomicU64::new(0));
    let show_modal = Arc::new(AtomicBool::new(false));
    let drag_dx = Arc::new(AtomicI32::new(0));
    let drag_dy = Arc::new(AtomicI32::new(0));
    let search_text = Arc::new(Mutex::new(String::new()));
    let submitted = Arc::new(Mutex::new(String::new()));

    // Clones for the tree closure (read side)
    let ticks_r = ticks.clone();
    let modal_r = show_modal.clone();
    let drag_dx_r = drag_dx.clone();
    let drag_dy_r = drag_dy.clone();
    let search_r = search_text.clone();
    let submitted_r = submitted.clone();

    // Clones for callbacks inside the tree closure (write side)
    let modal_open = show_modal.clone();
    let modal_close = show_modal.clone();
    let drag_dx_w = drag_dx.clone();
    let drag_dy_w = drag_dy.clone();
    let search_w = search_text.clone();
    let submitted_w = submitted.clone();

    let painter = Arc::new(GradientPainter::new());

    let mut app = App::new(
        AppConfig {
            title: "Kitchen Sink".to_string(),
            width: 1100,
            height: 820,
            css: CSS.to_string(),
            ..Default::default()
        },
        move || {
            let ticks_val = ticks_r.load(Ordering::Relaxed);
            let modal_visible = modal_r.load(Ordering::Relaxed);
            let dx = drag_dx_r.load(Ordering::Relaxed);
            let dy = drag_dy_r.load(Ordering::Relaxed);
            let search_val = search_r.lock().unwrap().clone();
            let submitted_val = submitted_r.lock().unwrap().clone();

            let mo = modal_open.clone();
            let mc = modal_close.clone();
            let dxw = drag_dx_w.clone();
            let dyw = drag_dy_w.clone();
            let sw = search_w.clone();
            let subw = submitted_w.clone();

            let mut root = ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(header(ticks_val))
                .with_child(search_bar(&submitted_val, sw, subw))
                .with_child(card_grid(&search_val, mo, dxw, dyw, dx, dy))
                .with_child(footer());

            // Feature 5: Conditional modal on the Modal layer
            if modal_visible {
                root = root.with_child(modal_overlay(mc));
            }

            ElementTree { root }
        },
    );

    // Feature 2: Register custom GPU painter
    app.register_canvas("gpu-canvas", painter);

    // Feature 3: Async subscription (live tick counter)
    let ticks_sub = ticks.clone();
    app.set_subscriptions(move || {
        let ticks = ticks_sub.clone();
        vec![Subscription::new("tick", move |_sink| {
            let ticks = ticks.clone();
            Box::pin(async_stream::stream! {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    ticks.fetch_add(1, Ordering::Relaxed);
                    yield ExternalEvent::RequestRebuild;
                }
            })
        })]
    });

    app.run();
}

// ---------------------------------------------------------------------------
// Header (Features 1, 3: reconciliation + async clock)
// ---------------------------------------------------------------------------

fn header(ticks: u64) -> ElementDef {
    let mins = ticks / 60;
    let secs = ticks % 60;
    let clock_text = format!("{:02}:{:02}", mins, secs);

    ElementDef::new(Tag::Div)
        .with_class("header")
        .with_child(ElementDef::new(Tag::Span).with_class("title").with_text("Kitchen Sink"))
        .with_child(ElementDef::new(Tag::Span).with_class("badge").with_text("10 FEATURES"))
        .with_child(ElementDef::new(Tag::Span).with_class("clock").with_text(clock_text))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("hint")
                .with_text("Tab to navigate, hover for transitions"),
        )
}

// ---------------------------------------------------------------------------
// Feature 4: Text input widget
// ---------------------------------------------------------------------------

fn search_bar(
    submitted_val: &str,
    search_w: Arc<Mutex<String>>,
    submitted_w: Arc<Mutex<String>>,
) -> ElementDef {
    let mut row = ElementDef::new(Tag::Div).with_class("search-row").with_child(
        ElementDef::new(Tag::Input)
            .with_class("search-input")
            .with_placeholder("Search features... (type and press Enter)")
            .with_tab_index(1)
            .on_change(move |text| {
                *search_w.lock().unwrap() = text.to_string();
            })
            .on_submit(move |text| {
                *submitted_w.lock().unwrap() = text.to_string();
            }),
    );

    if !submitted_val.is_empty() {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("submitted-text")
                .with_text(format!("Submitted: {}", submitted_val)),
        );
    }

    row
}

// ---------------------------------------------------------------------------
// Feature 10: CSS Grid card layout (+ Feature 6: scrollbar)
// ---------------------------------------------------------------------------

fn card_grid(
    filter: &str,
    modal_open: Arc<AtomicBool>,
    drag_dx_w: Arc<AtomicI32>,
    drag_dy_w: Arc<AtomicI32>,
    dx: i32,
    dy: i32,
) -> ElementDef {
    let filter_lower = filter.to_ascii_lowercase();
    let cards: Vec<(&str, Box<dyn FnOnce() -> ElementDef>)> = vec![
        ("async", Box::new(|| card_async())),
        ("canvas", Box::new(|| card_canvas())),
        ("drag", Box::new(move || card_drag(drag_dx_w, drag_dy_w, dx, dy))),
        ("transition", Box::new(|| card_transitions())),
        ("overlay", Box::new(move || card_overlay(modal_open))),
        ("keyboard", Box::new(|| card_keyboard())),
        ("scroll", Box::new(|| card_scrollbar())),
        ("grid", Box::new(|| card_grid_demo())),
    ];

    let mut grid = ElementDef::new(Tag::Div).with_class("card-grid");

    for (name, builder) in cards {
        if filter_lower.is_empty() || name.contains(&filter_lower) {
            grid = grid.with_child(builder());
        }
    }

    grid
}

// ---------------------------------------------------------------------------
// Card: Async subscriptions (Features 1 + 3)
// ---------------------------------------------------------------------------

fn card_async() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(10)
        .with_child(card_title("1", "Async Subscriptions"))
        .with_child(desc(
            "A background subscription ticks every second. The framework \
             manages its lifecycle automatically. Each tick triggers a \
             tree rebuild through the reconciliation/diffing engine.",
        ))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("counter-unit")
                .with_text("UPTIME (HEADER CLOCK)"),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("Subscription API", "tag-green"))
                .with_child(tag("EventSink", "tag-green"))
                .with_child(tag("reconciliation", "tag-blue")),
        )
}

// ---------------------------------------------------------------------------
// Card: Custom GPU canvas (Feature 2)
// ---------------------------------------------------------------------------

fn card_canvas() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(11)
        .with_child(card_title("2", "Custom GPU Canvas"))
        .with_child(desc(
            "A WGSL fragment shader renders a gradient directly on the GPU \
             via the CustomPainter trait. The framework provides device, queue, \
             and scissor rect isolation.",
        ))
        .with_child(ElementDef::new(Tag::Canvas).with_id("gpu-canvas").with_class("canvas-area"))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("CustomPainter", "tag-blue"))
                .with_child(tag("WGSL", "tag-blue"))
                .with_child(tag("needs_repaint", "tag-yellow")),
        )
}

// ---------------------------------------------------------------------------
// Card: Drag interaction (Feature 8)
// ---------------------------------------------------------------------------

fn card_drag(drag_dx_w: Arc<AtomicI32>, drag_dy_w: Arc<AtomicI32>, dx: i32, dy: i32) -> ElementDef {
    let dx_f = dx as f32 / 10.0;
    let dy_f = dy as f32 / 10.0;
    let coords = if dx == 0 && dy == 0 {
        "Click and drag here".to_string()
    } else {
        format!("dx: {:.1}  dy: {:.1}", dx_f, dy_f)
    };

    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(12)
        .with_child(card_title("3", "Drag Interaction"))
        .with_child(desc(
            "Drag the box below. A 4px threshold prevents accidental drags. \
             DragEvent provides delta, total_delta, and phase (Start/Update/End).",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("drag-zone")
                .on_drag(move |event| {
                    drag_dx_w.store((event.total_delta_x * 10.0) as i32, Ordering::Relaxed);
                    drag_dy_w.store((event.total_delta_y * 10.0) as i32, Ordering::Relaxed);
                })
                .with_child(ElementDef::new(Tag::Span).with_class("drag-coords").with_text(coords))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("drag-hint")
                        .with_text("DragPhase: Start / Update / End"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("DragEvent", "tag-blue"))
                .with_child(tag("4px threshold", "tag-yellow"))
                .with_child(tag("pointer capture", "tag-green")),
        )
}

// ---------------------------------------------------------------------------
// Card: CSS Transitions (Feature 9)
// ---------------------------------------------------------------------------

fn card_transitions() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(13)
        .with_child(card_title("4", "CSS Transitions"))
        .with_child(desc(
            "All cards animate on hover via the transition shorthand. The box \
             below has a 400ms ease-in-out opacity + background transition. \
             Easing uses a Newton-Raphson cubic-bezier solver. Colors interpolate \
             in Oklab space.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("transition-box")
                .with_text("Hover me (opacity + background transition)"),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("cubic-bezier", "tag-yellow"))
                .with_child(tag("Oklab", "tag-yellow"))
                .with_child(tag("ease-in-out", "tag-yellow")),
        )
}

// ---------------------------------------------------------------------------
// Card: Z-index / overlay (Feature 5)
// ---------------------------------------------------------------------------

fn card_overlay(modal_open: Arc<AtomicBool>) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(14)
        .with_child(card_title("5", "Z-Index / Overlay"))
        .with_child(desc(
            "Click the button to open a modal on the Modal layer. The modal uses \
             render-target: modal to escape parent clip rects (portals). \
             7 semantic layers: Background through Debug.",
        ))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("modal-btn")
                .with_text("Open Modal")
                .with_tab_index(15)
                .on_click(move || {
                    modal_open.store(true, Ordering::Relaxed);
                }),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("Layer enum", "tag-red"))
                .with_child(tag("Portal", "tag-red"))
                .with_child(tag("render-target", "tag-red")),
        )
}

// ---------------------------------------------------------------------------
// Card: Keyboard shortcuts (Feature 7)
// ---------------------------------------------------------------------------

fn card_keyboard() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(16)
        .with_child(card_title("6", "Keyboard Shortcuts"))
        .with_child(desc(
            "The shortcut system supports single keys, chords (Ctrl+K, Ctrl+C), \
             and WhenClause context (focused tag/class). Tab/Shift+Tab focus \
             cycling is built in. Try it on these buttons:",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("shortcut-row")
                .with_child(ElementDef::new(Tag::Span).with_class("kbd").with_text("Tab"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("shortcut-label")
                        .with_text("Next focusable"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("shortcut-row")
                .with_child(ElementDef::new(Tag::Span).with_class("kbd").with_text("Shift+Tab"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("shortcut-label")
                        .with_text("Previous focusable"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("focus-btn")
                        .with_text("Button A")
                        .with_tab_index(17),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("focus-btn")
                        .with_text("Button B")
                        .with_tab_index(18),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("focus-btn")
                        .with_text("Button C")
                        .with_tab_index(19),
                ),
        )
}

// ---------------------------------------------------------------------------
// Card: Scrollbar indicators (Feature 6)
// ---------------------------------------------------------------------------

fn card_scrollbar() -> ElementDef {
    let items = [
        "Layout engine (taffy)",
        "GPU rendering (wgpu)",
        "CSS parsing",
        "Border radius",
        "Box shadows",
        "Text rendering (cosmic-text)",
        "Hover effects",
        "Flex grow / shrink",
        "Gap property",
        "Overflow scroll",
        "Nested layouts",
        "Class selectors",
    ];

    let mut list = ElementDef::new(Tag::Div).with_class("scroll-list");
    for item in &items {
        list =
            list.with_child(ElementDef::new(Tag::Span).with_class("scroll-item").with_text(*item));
    }

    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(20)
        .with_child(card_title("7", "Scrollbar Indicators"))
        .with_child(desc(
            "Interactive scrollbars with thumb drag, track click, and hover \
             highlighting. The list below and the outer card grid both scroll.",
        ))
        .with_child(list)
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("overflow: scroll", "tag-green"))
                .with_child(tag("thumb drag", "tag-green"))
                .with_child(tag("track click", "tag-green")),
        )
}

// ---------------------------------------------------------------------------
// Card: CSS Grid demo (Feature 10)
// ---------------------------------------------------------------------------

fn card_grid_demo() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_class("card-span-2")
        .with_tab_index(21)
        .with_child(card_title("8", "CSS Grid Layout"))
        .with_child(desc(
            "This entire card grid uses display: grid with grid-template-columns: 1fr 1fr. \
             This card spans both columns via grid-column: span 2. Supports fr, px, %, auto, \
             minmax(), repeat(), and line/span placement.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("display: grid", "tag-green"))
                .with_child(tag("1fr 1fr", "tag-green"))
                .with_child(tag("grid-column: span 2", "tag-blue"))
                .with_child(tag("repeat()", "tag-yellow"))
                .with_child(tag("minmax()", "tag-yellow"))
                .with_child(tag("fit-content()", "tag-yellow")),
        )
}

// ---------------------------------------------------------------------------
// Modal overlay (Feature 5: z-index / portal)
// ---------------------------------------------------------------------------

fn modal_overlay(close: Arc<AtomicBool>) -> ElementDef {
    ElementDef::new(Tag::Div).with_class("modal-backdrop").with_child(
        ElementDef::new(Tag::Div)
            .with_class("modal-dialog")
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("modal-title")
                    .with_text("Z-Index / Overlay System"),
            )
            .with_child(ElementDef::new(Tag::Span).with_class("modal-body").with_text(
                "This modal renders on the Modal layer (layer: modal) and uses \
                             render-target: modal to portal out of parent clip rects. The \
                             framework provides 7 semantic layers: Background, Content, \
                             Popover, Modal, Overlay, Tooltip, and Debug. Hit testing \
                             respects layer ordering.",
            ))
            .with_child(
                ElementDef::new(Tag::Button)
                    .with_class("modal-close")
                    .with_text("Close Modal")
                    .with_tab_index(30)
                    .on_click(move || {
                        close.store(false, Ordering::Relaxed);
                    }),
            ),
    )
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

fn footer() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("footer")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("hint")
                .with_text("All 10 features from the #74 roadmap in one window"),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("badge").with_text("UNSHIT"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn card_title(num: &str, title: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card-title-row")
        .with_child(ElementDef::new(Tag::Span).with_class("card-num").with_text(num))
        .with_child(ElementDef::new(Tag::Span).with_class("card-label").with_text(title))
}

fn desc(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("card-desc").with_text(s)
}

fn tag(label: &str, class: &str) -> ElementDef {
    let mut el = ElementDef::new(Tag::Span).with_class("tag").with_text(label);
    if !class.is_empty() {
        el = el.with_class(class);
    }
    el
}
