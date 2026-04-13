//! Demonstrates the CustomPainter API: a user-defined GPU painter draws a
//! color-cycling triangle inside a canvas element.
//!
//! Run with: cargo run -p unshit --example canvas

use std::sync::{Arc, Mutex};

use unshit::app::{App, AppConfig};
use unshit::core::element::*;
use unshit::renderer::canvas::{CustomPainter, PaintContext};

/// A simple painter that renders a triangle inside a canvas element.
/// Demonstrates pipeline creation in `prepare()`, drawing in `paint()`,
/// and continuous repainting via `needs_repaint()`.
struct TrianglePainter {
    pipeline: Mutex<Option<wgpu::RenderPipeline>>,
}

impl TrianglePainter {
    fn new() -> Self {
        Self { pipeline: Mutex::new(None) }
    }
}

impl CustomPainter for TrianglePainter {
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
            label: Some("canvas triangle shader"),
            source: wgpu::ShaderSource::Wgsl(TRIANGLE_SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas triangle layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas triangle pipeline"),
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
                topology: wgpu::PrimitiveTopology::TriangleList,
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
        let Some(ref pipeline) = *lock else {
            return;
        };
        render_pass.set_pipeline(pipeline);
        render_pass.draw(0..3, 0..1);
    }

    fn needs_repaint(&self) -> bool {
        true
    }
}

const TRIANGLE_SHADER: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>( 0.0,  0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
    );
    return vec4<f32>(pos[idx], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Simple color based on fragment position
    let r = (sin(pos.x * 0.01) + 1.0) * 0.5;
    let g = (cos(pos.y * 0.01) + 1.0) * 0.5;
    return vec4<f32>(r, g, 0.6, 1.0);
}
"#;

fn main() {
    env_logger::init();

    let painter = Arc::new(TrianglePainter::new());

    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.95);
            align-items: center;
            justify-content: center;
            gap: 16px;
        }
        .title {
            color: #c9d1d9;
            font-size: 18px;
        }
        .canvas-area {
            width: 400px;
            height: 300px;
            border: 1px solid #30363d;
        }
        .hint {
            color: #484f58;
            font-size: 13px;
        }
    "#;

    let mut app = App::new(
        AppConfig {
            title: "canvas example".to_string(),
            width: 600,
            height: 500,
            css: css.to_string(),
            ..Default::default()
        },
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Span).with_class("title").with_text("Custom GPU Canvas"),
                )
                .with_child(ElementDef::new(Tag::Canvas).with_id("demo").with_class("canvas-area"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("hint")
                        .with_text("Triangle rendered via CustomPainter trait"),
                ),
        },
    );

    app.register_canvas("demo", painter);
    app.run();
}
