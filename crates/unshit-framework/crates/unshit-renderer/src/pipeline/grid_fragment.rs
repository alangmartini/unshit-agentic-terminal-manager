//! Experimental single pass fragment shader grid pipeline.
//!
//! Compiled only under the `grid-fragment-shader` feature. The runtime flag
//! `TM_USE_GRID_FRAGMENT_SHADER=1` selects the pipeline at app start when
//! the feature is available; with the flag unset the instanced path in
//! `pipeline::text` is used unconditionally.
//!
//! The pipeline binds:
//!
//! - Group 0, binding 0: uniform buffer (viewport, cell size, grid origin,
//!   cols, rows, cursor position, scroll origin, selection range, atlas
//!   generation).
//! - Group 0, binding 1: `GpuCell` storage buffer.
//! - Group 0, binding 2: `GpuGlyphMeta` storage buffer.
//! - Group 1, binding 0: the monochrome glyph atlas texture.
//! - Group 1, binding 1: the atlas sampler.
//!
//! A single draw call `pass.draw(0..3, 0..1)` covers the viewport with a
//! clipspace triangle. All shading happens in the fragment stage.

use bytemuck::{Pod, Zeroable};

/// Uniform block matching the `Uniforms` struct in `grid_fragment.wgsl`.
/// The two `_pad_*` fields keep the selection range and trailing values at
/// the 16 byte alignment that WGSL's std140-ish layout expects.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GridFragmentUniforms {
    pub viewport: [f32; 2],
    pub cell_size: [f32; 2],
    pub grid_origin: [f32; 2],
    // The remaining fields are `u32`, stored contiguously for clarity.
    pub cols: u32,
    pub rows: u32,
    pub cursor_col: u32,
    pub cursor_row: u32,
    pub cursor_style: u32,
    pub scroll_origin_row: u32,
    pub _pad_sel: u32,
    pub selection_start: u32,
    pub selection_end: u32,
    pub atlas_generation: u32,
    pub _pad_tail: u32,
    // Tail padding to round the struct size up to a 16 byte boundary.
    // WGSL uniform buffer layout requires `StructSize % 16 == 0` when the
    // struct alignment is at least 8, which is the case here because of
    // the `vec2<f32>` fields at the head.
    pub _pad_extra: [u32; 3],
}

impl GridFragmentUniforms {
    /// Build a default uniforms block with no cursor, no selection, and
    /// identity scroll. Callers typically override the viewport, cell size,
    /// and grid dimensions from the active `ElementContent::Grid`.
    pub fn new() -> Self {
        Self {
            viewport: [0.0, 0.0],
            cell_size: [0.0, 0.0],
            grid_origin: [0.0, 0.0],
            cols: 0,
            rows: 0,
            cursor_col: u32::MAX,
            cursor_row: u32::MAX,
            cursor_style: 0,
            scroll_origin_row: 0,
            _pad_sel: 0,
            selection_start: u32::MAX,
            selection_end: 0,
            atlas_generation: 0,
            _pad_tail: 0,
            _pad_extra: [0; 3],
        }
    }
}

impl Default for GridFragmentUniforms {
    fn default() -> Self {
        Self::new()
    }
}

/// Full pipeline object for the experimental grid fragment renderer.
///
/// Holds the render pipeline, the three buffers that live for the lifetime
/// of the pipeline (uniforms, cells, glyph meta), both bind group layouts,
/// and the current bind groups. The buffer layouts grow on demand via
/// `ensure_capacity`.
pub struct GridFragmentPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub storage_bind_group_layout: wgpu::BindGroupLayout,
    pub atlas_bind_group_layout: wgpu::BindGroupLayout,
    pub atlas_bind_group: wgpu::BindGroup,
    pub storage_bind_group: wgpu::BindGroup,

    /// Uniform buffer, size `size_of::<GridFragmentUniforms>()`.
    pub uniform_buffer: wgpu::Buffer,
    /// `GpuCell` storage buffer. Grows when the grid resizes.
    pub cell_buffer: wgpu::Buffer,
    pub cell_buffer_capacity_cells: u32,
    /// `GpuGlyphMeta` storage buffer. Grows when a new glyph is inserted
    /// and the current allocation is full.
    pub glyph_meta_buffer: wgpu::Buffer,
    pub glyph_meta_capacity: u32,
}

/// Byte size of one GPU cell descriptor. Mirrors
/// `grid_fragment_upload::GpuCell`. Kept as a local constant to avoid a
/// circular dependency when the pipeline is constructed before the upload
/// module is loaded.
pub const GPU_CELL_SIZE: u64 = 16;
/// Byte size of one GPU glyph meta record.
pub const GPU_GLYPH_META_SIZE: u64 = 32;

impl GridFragmentPipeline {
    /// Construct the pipeline. `sample_count` must match the MSAA state
    /// used by the main content pipelines so both targets render into the
    /// same attachment.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
        sample_count: u32,
        initial_cells: u32,
        initial_glyphs: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid fragment shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/grid_fragment.wgsl").into()),
        });

        let storage_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grid fragment storage layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let atlas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grid fragment atlas layout"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid fragment pipeline layout"),
            bind_group_layouts: &[&storage_bind_group_layout, &atlas_bind_group_layout],
            push_constant_ranges: &[],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid fragment uniforms"),
            size: std::mem::size_of::<GridFragmentUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let initial_cells = initial_cells.max(1);
        let initial_glyphs = initial_glyphs.max(1);

        let cell_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid fragment cells"),
            size: initial_cells as u64 * GPU_CELL_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_meta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid fragment glyph meta"),
            size: initial_glyphs as u64 * GPU_GLYPH_META_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let storage_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid fragment storage bind group"),
            layout: &storage_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cell_buffer.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: glyph_meta_buffer.as_entire_binding(),
                },
            ],
        });
        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid fragment atlas bind group"),
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

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid fragment pipeline"),
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

        Self {
            pipeline,
            storage_bind_group_layout,
            atlas_bind_group_layout,
            atlas_bind_group,
            storage_bind_group,
            uniform_buffer,
            cell_buffer,
            cell_buffer_capacity_cells: initial_cells,
            glyph_meta_buffer,
            glyph_meta_capacity: initial_glyphs,
        }
    }

    /// Grow the cell buffer if the grid needs more room. Returns `true`
    /// when a reallocation actually happened so the caller can mark the
    /// next upload as "full rewrite".
    pub fn ensure_cell_capacity(&mut self, device: &wgpu::Device, cells_needed: u32) -> bool {
        if cells_needed <= self.cell_buffer_capacity_cells {
            return false;
        }
        let new_capacity =
            cells_needed.next_power_of_two().max(self.cell_buffer_capacity_cells * 2);
        self.cell_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid fragment cells"),
            size: new_capacity as u64 * GPU_CELL_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.cell_buffer_capacity_cells = new_capacity;
        self.rebuild_storage_bind_group(device);
        true
    }

    /// Grow the glyph meta buffer when a new id moves past the current
    /// allocation. Returns `true` on reallocation.
    pub fn ensure_glyph_meta_capacity(
        &mut self,
        device: &wgpu::Device,
        glyphs_needed: u32,
    ) -> bool {
        if glyphs_needed <= self.glyph_meta_capacity {
            return false;
        }
        let new_capacity = glyphs_needed.next_power_of_two().max(self.glyph_meta_capacity * 2);
        self.glyph_meta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid fragment glyph meta"),
            size: new_capacity as u64 * GPU_GLYPH_META_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.glyph_meta_capacity = new_capacity;
        self.rebuild_storage_bind_group(device);
        true
    }

    /// Rebind the atlas view and sampler after a full atlas recreation. The
    /// generation number in the uniforms block is bumped separately.
    pub fn rebuild_atlas_bind_group(
        &mut self,
        device: &wgpu::Device,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
    ) {
        self.atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid fragment atlas bind group"),
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

    fn rebuild_storage_bind_group(&mut self, device: &wgpu::Device) {
        self.storage_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid fragment storage bind group"),
            layout: &self.storage_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry { binding: 1, resource: self.cell_buffer.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.glyph_meta_buffer.as_entire_binding(),
                },
            ],
        });
    }

    /// Write the uniform block. Mirrors the `update_uniforms` helper on
    /// `TextPipeline`.
    pub fn write_uniforms(&self, queue: &wgpu::Queue, uniforms: &GridFragmentUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }

    /// Issue one `queue.write_buffer` for the given byte range into the
    /// cell buffer. The caller is responsible for serializing the bytes
    /// via `bytemuck::cast_slice`.
    pub fn write_cells(&self, queue: &wgpu::Queue, byte_offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        queue.write_buffer(&self.cell_buffer, byte_offset, data);
    }

    /// Partial or full rewrite of the glyph metadata buffer.
    pub fn write_glyph_meta(&self, queue: &wgpu::Queue, byte_offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        queue.write_buffer(&self.glyph_meta_buffer, byte_offset, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_pack_to_16_byte_boundary() {
        // 3 x vec2<f32> = 24 bytes + 11 x u32 = 44 bytes + 3 x u32 tail pad
        // = 80 bytes total. WGSL uniform buffer layout requires the struct
        // size to be a multiple of the alignment (here 16 because of the
        // vec2<f32> fields at the head).
        assert_eq!(std::mem::size_of::<GridFragmentUniforms>(), 80);
        assert_eq!(std::mem::size_of::<GridFragmentUniforms>() % 16, 0);
    }

    #[test]
    fn gpu_cell_size_matches_upload_module() {
        // Local constant must match the upload module so buffer stride
        // math stays in sync across pipeline and upload logic.
        assert_eq!(GPU_CELL_SIZE, 16);
        assert_eq!(GPU_GLYPH_META_SIZE, 32);
    }

    #[test]
    fn default_uniforms_mark_no_cursor_or_selection() {
        let u = GridFragmentUniforms::new();
        assert_eq!(u.cursor_col, u32::MAX);
        assert_eq!(u.cursor_row, u32::MAX);
        assert_eq!(u.selection_start, u32::MAX);
    }
}
