//! GPU pipeline for pre tessellated SVG geometry.
//!
//! Unlike the quad and image pipelines, an SVG draw has its own vertex and
//! index buffers per cached geometry. This pipeline uploads those buffers on
//! first use, caches them behind the `SvgGeometry` handle (via a separate
//! `GeometryGpu` lookup owned by the pipeline), and draws with a dynamic
//! offset uniform buffer for per instance parameters.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::svg_tess::{SvgGeometry, SvgVertex};

/// Uniform block passed for the whole frame (just the viewport).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalUniforms {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// Per draw uniform block written into a dynamically offset uniform buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SvgInstanceUniforms {
    pub translate: [f32; 2],  // 0..8
    pub scale: [f32; 2],      // 8..16
    pub clip_rect: [f32; 4],  // 16..32
    pub color_tint: [f32; 4], // 32..48
    pub opacity: f32,         // 48..52
    // WGSL vec3<f32> has alignment 16, so the shader pads 52..64 before
    // the `_pad` field, then adds 4 bytes of struct tail padding to reach
    // 80. We emit 7 floats of padding to match the 80-byte WGSL layout.
    pub _pad: [f32; 7], // 52..80
}

impl Default for SvgInstanceUniforms {
    fn default() -> Self {
        Self {
            translate: [0.0; 2],
            scale: [1.0; 2],
            clip_rect: [0.0, 0.0, 9999.0, 9999.0],
            color_tint: [1.0; 4],
            opacity: 1.0,
            _pad: [0.0; 7],
        }
    }
}

/// Uniform block stride once padded to the minimum dynamic offset alignment.
const INSTANCE_UNIFORM_STRIDE: u64 = 256;

/// GPU side resources for a single tessellated geometry.
struct GeometryGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

pub struct SvgPipeline {
    pub pipeline: wgpu::RenderPipeline,

    global_bind_group: wgpu::BindGroup,
    global_uniform_buffer: wgpu::Buffer,

    instance_bind_group_layout: wgpu::BindGroupLayout,
    instance_bind_group: wgpu::BindGroup,
    instance_uniform_buffer: wgpu::Buffer,
    instance_capacity: usize,

    // Lookup from an `Arc<SvgGeometry>` identity (pointer value) to the
    // uploaded GPU buffers. Keeping this map on the pipeline means the
    // caller only has to hand us an `Arc<SvgGeometry>` and we take care of
    // upload on demand.
    geometry_gpu: HashMap<usize, GeometryGpu>,
}

impl SvgPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, sample_count: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("svg shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/svg.wgsl").into()),
        });

        let global_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg globals"),
            size: std::mem::size_of::<GlobalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let global_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("svg global bgl"),
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

        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("svg global bg"),
            layout: &global_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: global_uniform_buffer.as_entire_binding(),
            }],
        });

        let instance_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("svg instance bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            SvgInstanceUniforms,
                        >() as u64),
                    },
                    count: None,
                }],
            });

        let initial_instance_capacity = 64usize;
        let instance_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg instance uniforms"),
            size: INSTANCE_UNIFORM_STRIDE * initial_instance_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instance_bind_group =
            make_instance_bind_group(device, &instance_bind_group_layout, &instance_uniform_buffer);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("svg pipeline layout"),
            bind_group_layouts: &[&global_bind_group_layout, &instance_bind_group_layout],
            push_constant_ranges: &[],
        });

        let attrs = wgpu::vertex_attr_array![
            0 => Float32x2, // position
            1 => Float32x4, // color
            2 => Float32,   // coverage (analytical AA)
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("svg pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SvgVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
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

        Self {
            pipeline,
            global_bind_group,
            global_uniform_buffer,
            instance_bind_group_layout,
            instance_bind_group,
            instance_uniform_buffer,
            instance_capacity: initial_instance_capacity,
            geometry_gpu: HashMap::new(),
        }
    }

    pub fn update_globals(&self, queue: &wgpu::Queue, width: f32, height: f32) {
        let uniforms = GlobalUniforms { viewport: [width, height], _pad: [0.0; 2] };
        queue.write_buffer(&self.global_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Drop cached GPU buffers for geometries no longer referenced. Called
    /// each frame to keep memory bounded when icons are swapped out.
    pub fn prune_unreferenced(&mut self, live: &std::collections::HashSet<usize>) {
        self.geometry_gpu.retain(|k, _| live.contains(k));
    }

    /// Upload a new instance uniform block, resizing the buffer if needed.
    ///
    /// Returns the byte offset to use with `set_bind_group` dynamic offsets.
    pub fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[SvgInstanceUniforms],
    ) {
        if instances.is_empty() {
            return;
        }
        let needed = instances.len();
        if needed > self.instance_capacity {
            let new_capacity = needed.next_power_of_two().max(64);
            self.instance_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("svg instance uniforms"),
                size: INSTANCE_UNIFORM_STRIDE * new_capacity as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_capacity = new_capacity;
            self.instance_bind_group = make_instance_bind_group(
                device,
                &self.instance_bind_group_layout,
                &self.instance_uniform_buffer,
            );
        }

        // Write each instance at its own aligned offset.
        let mut scratch = vec![0u8; INSTANCE_UNIFORM_STRIDE as usize];
        for (i, instance) in instances.iter().enumerate() {
            let bytes = bytemuck::bytes_of(instance);
            scratch[..bytes.len()].copy_from_slice(bytes);
            for b in &mut scratch[bytes.len()..] {
                *b = 0;
            }
            queue.write_buffer(
                &self.instance_uniform_buffer,
                (i as u64) * INSTANCE_UNIFORM_STRIDE,
                &scratch,
            );
        }
    }

    /// Ensure a GPU buffer exists for the given cached geometry, uploading
    /// on first use. Returns the map key so the renderer can look up the
    /// buffers during the render pass.
    pub fn ensure_geometry(
        &mut self,
        device: &wgpu::Device,
        geometry: &Arc<SvgGeometry>,
    ) -> Option<usize> {
        if geometry.is_empty() {
            return None;
        }
        let key = Arc::as_ptr(geometry) as usize;
        if self.geometry_gpu.contains_key(&key) {
            return Some(key);
        }

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("svg vertex"),
            contents: bytemuck::cast_slice(&geometry.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("svg index"),
            contents: bytemuck::cast_slice(&geometry.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        self.geometry_gpu.insert(
            key,
            GeometryGpu { vertex_buffer, index_buffer, index_count: geometry.indices.len() as u32 },
        );
        Some(key)
    }

    pub fn global_bind_group(&self) -> &wgpu::BindGroup {
        &self.global_bind_group
    }

    pub fn instance_bind_group(&self) -> &wgpu::BindGroup {
        &self.instance_bind_group
    }

    pub fn instance_stride() -> u64 {
        INSTANCE_UNIFORM_STRIDE
    }

    /// Record a single draw call for the given geometry key and instance
    /// index. The caller must have already set the pipeline and the global
    /// bind group.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        key: usize,
        instance_index: u32,
    ) -> bool {
        let Some(entry) = self.geometry_gpu.get(&key) else { return false };
        let offset = (instance_index as u64) * INSTANCE_UNIFORM_STRIDE;
        pass.set_bind_group(1, &self.instance_bind_group, &[offset as u32]);
        pass.set_vertex_buffer(0, entry.vertex_buffer.slice(..));
        pass.set_index_buffer(entry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..entry.index_count, 0, 0..1);
        true
    }
}

fn make_instance_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("svg instance bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<SvgInstanceUniforms>() as u64),
            }),
        }],
    })
}
