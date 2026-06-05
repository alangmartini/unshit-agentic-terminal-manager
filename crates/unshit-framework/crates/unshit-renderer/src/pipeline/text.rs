use bytemuck::{Pod, Zeroable};
use std::sync::OnceLock;
use wgpu;

use crate::instance_buffer_pool::InstanceBufferPool;
use crate::text_rendering::use_subpixel_text_shader;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GlyphInstance {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub color: [f32; 4],
    pub clip_rect: [f32; 4],
    /// CSS `transform` linear part as a 2x2 DELTA from identity
    /// (`[a-1, b, c, d-1]`); see [`crate::pipeline::quad::QuadInstance::xform`].
    /// Zero is the identity, so a glyph run inherits its element's transform
    /// (rotating / scaling about the element's center) and untransformed runs
    /// pay nothing.
    pub xform: [f32; 4],
    /// Translation part `[e, f]` of the transform affine, in screen pixels.
    pub xform_translate: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

pub struct TextPipeline {
    pub pipeline: wgpu::RenderPipeline,
    /// Pool of per submission glyph instance buffers. See
    /// [`crate::instance_buffer_pool`] for the lifetime protocol.
    pub instance_pool: InstanceBufferPool<GlyphInstance>,
    pub uniform_bind_group: wgpu::BindGroup,
    pub atlas_bind_group: wgpu::BindGroup,
    pub atlas_bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

fn use_debug_solid_text_shader() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TM_DEBUG_SOLID_TEXT").is_some())
}

impl TextPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
        sample_count: u32,
    ) -> Self {
        #[cfg(target_os = "windows")]
        let shader_src = if use_debug_solid_text_shader() {
            include_str!("../shaders/text_debug_solid.wgsl")
        } else if use_subpixel_text_shader() {
            include_str!("../shaders/text_subpixel.wgsl")
        } else {
            include_str!("../shaders/text.wgsl")
        };
        #[cfg(not(target_os = "windows"))]
        let shader_src = if use_debug_solid_text_shader() {
            include_str!("../shaders/text_debug_solid.wgsl")
        } else {
            include_str!("../shaders/text.wgsl")
        };

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("text uniform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text uniform bind group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let atlas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("text atlas layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text atlas bind group"),
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(atlas_sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text pipeline layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &atlas_bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_attrs = wgpu::vertex_attr_array![
            0 => Float32x2,  // pos
            1 => Float32x2,  // size
            2 => Float32x2,  // uv_min
            3 => Float32x2,  // uv_max
            4 => Float32x4,  // color
            5 => Float32x4,  // clip_rect
            6 => Float32x4,  // xform (2x2 linear delta-from-identity: a-1,b,c,d-1)
            7 => Float32x2,  // xform_translate (e, f)
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
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
                    blend: Some({
                        #[cfg(target_os = "windows")]
                        {
                            if use_subpixel_text_shader() {
                                // Premultiplied alpha for subpixel blending
                                wgpu::BlendState {
                                    color: wgpu::BlendComponent {
                                        src_factor: wgpu::BlendFactor::One,
                                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                        operation: wgpu::BlendOperation::Add,
                                    },
                                    alpha: wgpu::BlendComponent {
                                        src_factor: wgpu::BlendFactor::One,
                                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                        operation: wgpu::BlendOperation::Add,
                                    },
                                }
                            } else {
                                wgpu::BlendState::ALPHA_BLENDING
                            }
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            wgpu::BlendState::ALPHA_BLENDING
                        }
                    }),
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

        let initial_capacity = 16384;
        let instance_pool = InstanceBufferPool::<GlyphInstance>::new(
            "glyph instances",
            initial_capacity,
            wgpu::BufferUsages::VERTEX,
        );

        Self {
            pipeline,
            instance_pool,
            uniform_bind_group,
            atlas_bind_group,
            atlas_bind_group_layout,
            uniform_buffer,
        }
    }

    pub fn update_uniforms(&self, queue: &wgpu::Queue, width: f32, height: f32) {
        let uniforms = Uniforms { viewport: [width, height], _pad: [0.0; 2] };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn rebuild_atlas_bind_group(
        &mut self,
        device: &wgpu::Device,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
    ) {
        self.atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text atlas bind group"),
            layout: &self.atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(atlas_sampler),
                },
            ],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    /// The pipeline emits one draw call per layer of the form
    /// `pass.draw(0..6, base..base + count)`, which is only correct when
    /// every vertex attribute is configured with `VertexStepMode::Instance`.
    /// This test guards that contract at the struct level by asserting the
    /// GlyphInstance size matches the wgpu layout expectation.
    #[test]
    fn glyph_instance_has_expected_size() {
        // 2 + 2 + 2 + 2 + 4 + 4 (= 16) + xform 4 + xform_translate 2
        // = 22 floats = 88 bytes.
        assert_eq!(std::mem::size_of::<GlyphInstance>(), 88);
    }

    #[test]
    fn glyph_instance_zeroable_produces_all_zero_bytes() {
        // Verifies the instance can be bulk zero initialized when growing
        // instance buffers, keeping the one draw per layer path allocation
        // free across frames.
        let g = GlyphInstance::zeroed();
        let bytes = bytemuck::bytes_of(&g);
        assert!(bytes.iter().all(|&b| b == 0));
    }
}
