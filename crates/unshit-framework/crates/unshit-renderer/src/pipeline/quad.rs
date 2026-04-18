use bytemuck::{Pod, Zeroable};
use wgpu;

use crate::instance_buffer_pool::InstanceBufferPool;

/// Maximum number of gradient stops packed into a single `QuadInstance`.
///
/// This cap matches the WGSL shader loop in `shaders/quad.wgsl`. Gradients
/// with more stops are truncated by the batch builder with a one time warn
/// log; the terminal-manager corpus needs at most 4.
pub const MAX_GRADIENT_STOPS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct QuadInstance {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],
    pub border_color: [f32; 4],
    pub border_width: [f32; 4],
    pub border_radius: [f32; 4],
    pub clip_rect: [f32; 4],
    pub shadow_color: [f32; 4],
    pub shadow_offset: [f32; 2],
    pub shadow_params: [f32; 2], // [blur_radius, inset_flag (0.0 outer, 1.0 inset)]
    pub shadow_spread: [f32; 2], // [spread_radius, 0.0 reserved]
    /// Up to 8 gradient stop colors in linear RGBA. Unused slots are zero.
    pub gradient_stop_colors: [[f32; 4]; MAX_GRADIENT_STOPS],
    /// Gradient stop positions in [0, 1]. Packed as two `vec4` rows so the
    /// vertex attribute layout stays on 16 byte boundaries.
    pub gradient_stop_positions: [f32; MAX_GRADIENT_STOPS],
    /// `gradient_params.w` is a tagged stop count:
    /// * `0` means solid color (fall back to `color`).
    /// * Positive value (`>= 2`) means an N stop linear gradient with
    ///   that many valid stops starting from index 0. `.x` holds the
    ///   linear gradient angle in radians.
    /// * Negative value means an N stop radial gradient. The magnitude
    ///   (`|w|`) is the stop count. Radial specific center and radii
    ///   travel in `gradient_extra`.
    pub gradient_params: [f32; 4],
    /// Radial gradient auxiliary data: `[center_x, center_y, radius_x, radius_y]`
    /// in element local pixels. `radius_y <= 0` is a guard: the shader
    /// short circuits to the last stop color to avoid a division by zero.
    /// For a true circle the CPU sets `radius_x == radius_y`; the shader
    /// also checks the `shape_is_circle` flag which is encoded in the
    /// sign of `gradient_params.y` (`< 0` means circle, `>= 0` means
    /// ellipse).
    pub gradient_extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

pub struct QuadPipeline {
    pub pipeline: wgpu::RenderPipeline,
    /// Pool of per submission instance buffers. One `PooledBuffer` is
    /// acquired per frame and released in the `on_submitted_work_done`
    /// callback for that frame's submit. See
    /// [`crate::instance_buffer_pool`] for the lifetime protocol.
    pub instance_pool: InstanceBufferPool<QuadInstance>,
    pub bind_group: wgpu::BindGroup,
    pub bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl QuadPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, sample_count: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/quad.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_attrs = wgpu::vertex_attr_array![
            0 => Float32x2,  // pos
            1 => Float32x2,  // size
            2 => Float32x4,  // color
            3 => Float32x4,  // border_color
            4 => Float32x4,  // border_width
            5 => Float32x4,  // border_radius
            6 => Float32x4,  // clip_rect
            7 => Float32x4,  // shadow_color
            8 => Float32x2,  // shadow_offset
            9 => Float32x2,  // shadow_params
            10 => Float32x2, // shadow_spread
            11 => Float32x4, // gradient_stop_colors[0]
            12 => Float32x4, // gradient_stop_colors[1]
            13 => Float32x4, // gradient_stop_colors[2]
            14 => Float32x4, // gradient_stop_colors[3]
            15 => Float32x4, // gradient_stop_colors[4]
            16 => Float32x4, // gradient_stop_colors[5]
            17 => Float32x4, // gradient_stop_colors[6]
            18 => Float32x4, // gradient_stop_colors[7]
            19 => Float32x4, // gradient_stop_positions[0..4]
            20 => Float32x4, // gradient_stop_positions[4..8]
            21 => Float32x4, // gradient_params
            22 => Float32x4, // gradient_extra (radial center and radii)
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<QuadInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attrs,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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

        let initial_capacity = 4096;
        let instance_pool = InstanceBufferPool::<QuadInstance>::new(
            "quad instances",
            initial_capacity,
            wgpu::BufferUsages::VERTEX,
        );

        Self { pipeline, instance_pool, bind_group, bind_group_layout, uniform_buffer }
    }

    pub fn update_uniforms(&self, queue: &wgpu::Queue, width: f32, height: f32) {
        let uniforms = Uniforms { viewport: [width, height], _pad: [0.0; 2] };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}
