//! Backdrop filter blur pipeline.
//!
//! Runs the two pass separable Gaussian blur shader in
//! `shaders/backdrop_blur.wgsl`. The pipeline is created lazily the first
//! time a frame emits a `BackdropBoundary`. A frame that never uses the
//! `backdrop-filter` CSS property never instantiates this type, which is
//! what keeps the renderer fast path truly zero cost.

use bytemuck::{Pod, Zeroable};
use wgpu;

/// Maximum supported blur radius in pixels. Matches the CSS parser clamp so
/// the shader uniform array stays bounded.
pub const MAX_BLUR_RADIUS: u32 = 64;

/// Uniform block uploaded once per blur invocation.
///
/// Layout mirrors the WGSL `BlurUniforms` struct. The weights array carries
/// `radius + 1` centered Gaussian weights (the center weight is at index
/// zero, then symmetric side weights). Seventeen `vec4<f32>` slots cover 68
/// entries, which is the first multiple of four at or past the clamp.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct BlurUniforms {
    pub direction: [f32; 2],
    pub radius: f32,
    pub _pad0: f32,
    pub texel_size: [f32; 2],
    pub _pad1: [f32; 2],
    pub weights: [[f32; 4]; 17],
}

impl BlurUniforms {
    pub fn zeroed() -> Self {
        Self {
            direction: [0.0; 2],
            radius: 0.0,
            _pad0: 0.0,
            texel_size: [0.0; 2],
            _pad1: [0.0; 2],
            weights: [[0.0; 4]; 17],
        }
    }
}

/// Build a Gaussian kernel of integer radius.
///
/// Produces a 68 entry buffer (the maximum supported by the shader) where
/// the first `radius + 1` slots hold the normalized weights. The discrete
/// Gaussian is generated with standard deviation `sigma = radius / 2.0` so a
/// radius 6 kernel reaches near zero at the edge, matching the visual
/// behavior of CSS `blur(6px)`.
pub fn gaussian_weights(radius: u32) -> [[f32; 4]; 17] {
    let mut flat = [0.0f32; 68];
    let r = radius as i32;
    if r == 0 {
        flat[0] = 1.0;
    } else {
        let sigma = (radius as f32) * 0.5;
        let two_sigma_sq = 2.0 * sigma * sigma;
        let mut sum = 0.0f32;
        for i in 0..=r {
            let w = (-((i * i) as f32) / two_sigma_sq).exp();
            flat[i as usize] = w;
            sum += if i == 0 { w } else { 2.0 * w };
        }
        if sum > 0.0 {
            for slot in flat.iter_mut().take((r + 1) as usize) {
                *slot /= sum;
            }
        }
    }
    let mut out = [[0.0f32; 4]; 17];
    for (i, slot) in flat.iter().enumerate() {
        out[i / 4][i % 4] = *slot;
    }
    out
}

/// Lazily constructed pipeline state for the backdrop blur fragment shader.
pub struct BackdropBlurPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub uniform_layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    pub uniform_buffer: wgpu::Buffer,
}

impl BackdropBlurPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop blur shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/backdrop_blur.wgsl").into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop blur layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop blur pipeline layout"),
            bind_group_layouts: &[&uniform_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("backdrop blur pipeline"),
            layout: Some(&pipeline_layout),
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
                    // Blur passes fully overwrite the destination texel.
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("backdrop blur sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("backdrop blur uniforms"),
            size: std::mem::size_of::<BlurUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self { pipeline, uniform_layout, sampler, uniform_buffer }
    }

    /// Create a bind group with the given source texture view. Call this once
    /// per blur pass (once per direction) since the source alternates between
    /// the two ping pong textures.
    pub fn make_bind_group(
        &self,
        device: &wgpu::Device,
        source_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop blur bind group"),
            layout: &self.uniform_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    pub fn upload_uniforms(&self, queue: &wgpu::Queue, uniforms: &BlurUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_weights_radius_zero_is_single_tap() {
        let w = gaussian_weights(0);
        assert_eq!(w[0][0], 1.0);
        // Everything past the center must be exactly zero.
        for i in 1..68 {
            assert_eq!(w[i / 4][i % 4], 0.0);
        }
    }

    #[test]
    fn gaussian_weights_sum_to_one() {
        let w = gaussian_weights(6);
        let center = w[0][0];
        let mut sum = center;
        for i in 1..=6 {
            sum += 2.0 * w[i / 4][i % 4];
        }
        assert!((sum - 1.0).abs() < 1e-5, "weights should sum to 1, got {}", sum);
    }

    #[test]
    fn gaussian_weights_center_is_largest() {
        let w = gaussian_weights(4);
        let center = w[0][0];
        for i in 1..=4 {
            assert!(w[i / 4][i % 4] < center, "center weight should dominate");
        }
    }

    #[test]
    fn gaussian_weights_max_radius() {
        // Must not panic and must fit within the 17 slot buffer.
        let w = gaussian_weights(MAX_BLUR_RADIUS);
        let center = w[0][0];
        let mut sum = center;
        for i in 1..=MAX_BLUR_RADIUS as usize {
            sum += 2.0 * w[i / 4][i % 4];
        }
        assert!((sum - 1.0).abs() < 1e-4);
    }
}
