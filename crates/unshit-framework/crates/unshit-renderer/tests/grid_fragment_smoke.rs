//! Smoke tests for the experimental fragment shader grid pipeline.
//!
//! Feature gated behind `grid-fragment-shader`. Tests require a real wgpu
//! adapter; they skip silently when no GPU is available so CI without a
//! GPU does not fail.
//!
//! Parity tests between the instanced path and the fragment path are
//! intentionally deferred. This scaffold verifies the pipeline compiles,
//! the WGSL shader parses, and the buffer upload helpers are consistent.
//! A later PR will add pixel level parity coverage.

#![cfg(feature = "grid-fragment-shader")]

use std::sync::atomic::{AtomicUsize, Ordering};

use unshit_core::cell_grid::{Cell, CellGrid};
use unshit_renderer::grid_fragment_upload::{
    damaged_row_write_ranges, encode_row, pack_color_rgba, GpuCell, GpuGlyphMeta, GridFragmentState,
};
use unshit_renderer::pipeline::grid_fragment::{
    GridFragmentPipeline, GridFragmentUniforms, GPU_CELL_SIZE, GPU_GLYPH_META_SIZE,
};

fn try_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("grid-fragment smoke test device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        },
        None,
    ))
    .ok()?;
    Some((device, queue))
}

macro_rules! require_gpu {
    ($name:ident, ($device:pat, $queue:pat), $body:block) => {{
        static SKIPPED: AtomicUsize = AtomicUsize::new(0);
        match try_gpu() {
            Some(($device, $queue)) => $body,
            None => {
                if SKIPPED.fetch_add(1, Ordering::Relaxed) == 0 {
                    eprintln!("[grid_fragment_smoke] skipping {}: no adapter", stringify!($name));
                }
            }
        }
    }};
}

/// Build a throwaway dummy atlas view / sampler pair. We do not actually
/// sample from it in these smoke tests; the pipeline construction just
/// needs a resolvable binding.
fn dummy_atlas(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dummy atlas"),
        size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("dummy sampler"),
        ..Default::default()
    });
    (texture, view, sampler)
}

#[test]
fn pipeline_constructs_without_panic() {
    // Verifies the WGSL shader parses, the bind group layouts are accepted
    // by the validator, and the storage buffer bindings do not exceed the
    // adapter's declared limits. Any of these would explode the pipeline
    // construction.
    require_gpu!(pipeline_constructs_without_panic, (device, _queue), {
        let (_tex, view, sampler) = dummy_atlas(&device);
        let _pipeline = GridFragmentPipeline::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            &view,
            &sampler,
            1,
            /* initial_cells = */ 16,
            /* initial_glyphs = */ 16,
        );
    });
}

#[test]
fn uniform_buffer_size_matches_shader_layout() {
    require_gpu!(uniform_buffer_size_matches_shader_layout, (device, _queue), {
        let (_tex, view, sampler) = dummy_atlas(&device);
        let pipeline = GridFragmentPipeline::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            &view,
            &sampler,
            1,
            1,
            1,
        );
        // Must be exactly one uniform block.
        assert_eq!(
            pipeline.uniform_buffer.size(),
            std::mem::size_of::<GridFragmentUniforms>() as u64,
        );
    });
}

#[test]
fn cell_buffer_grows_on_resize() {
    // `ensure_cell_capacity` doubles the backing buffer when the grid
    // grows past the current allocation, matching the instance pool
    // growth policy elsewhere in the renderer.
    require_gpu!(cell_buffer_grows_on_resize, (device, _queue), {
        let (_tex, view, sampler) = dummy_atlas(&device);
        let mut pipeline = GridFragmentPipeline::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            &view,
            &sampler,
            1,
            16,
            1,
        );
        assert_eq!(pipeline.cell_buffer_capacity_cells, 16);
        let grew = pipeline.ensure_cell_capacity(&device, 20);
        assert!(grew);
        assert!(pipeline.cell_buffer_capacity_cells >= 20);
    });
}

#[test]
fn cell_buffer_does_not_grow_when_within_capacity() {
    require_gpu!(cell_buffer_does_not_grow_when_within_capacity, (device, _queue), {
        let (_tex, view, sampler) = dummy_atlas(&device);
        let mut pipeline = GridFragmentPipeline::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            &view,
            &sampler,
            1,
            64,
            1,
        );
        let before = pipeline.cell_buffer_capacity_cells;
        let grew = pipeline.ensure_cell_capacity(&device, 40);
        assert!(!grew);
        assert_eq!(pipeline.cell_buffer_capacity_cells, before);
    });
}

#[test]
fn write_cells_and_uniforms_without_validation_errors() {
    // Smoke: upload a row of cells and a uniform block, then submit an
    // empty encoder to flush. A validation error would surface as a
    // panic through wgpu's logger.
    require_gpu!(write_cells_and_uniforms_without_validation_errors, (device, queue), {
        let (_tex, view, sampler) = dummy_atlas(&device);
        let pipeline = GridFragmentPipeline::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            &view,
            &sampler,
            1,
            32,
            4,
        );

        let uniforms = GridFragmentUniforms {
            viewport: [64.0, 64.0],
            cell_size: [8.0, 16.0],
            grid_origin: [0.0, 0.0],
            cols: 8,
            rows: 2,
            ..GridFragmentUniforms::new()
        };
        pipeline.write_uniforms(&queue, &uniforms);

        let cells: Vec<GpuCell> = (0..16)
            .map(|i| GpuCell {
                glyph_id: u32::MAX,
                fg_rgba: pack_color_rgba(unshit_core::style::types::Color::WHITE),
                bg_rgba: pack_color_rgba(unshit_core::style::types::Color::BLACK),
                flags: i as u32,
            })
            .collect();
        pipeline.write_cells(&queue, 0, bytemuck::cast_slice(&cells));

        let metas: Vec<GpuGlyphMeta> = (0..4)
            .map(|_| GpuGlyphMeta {
                atlas_uv_min: [0.0, 0.0],
                atlas_uv_max: [1.0, 1.0],
                pixel_offset: [0.0, 0.0],
                pixel_size: [8.0, 16.0],
            })
            .collect();
        pipeline.write_glyph_meta(&queue, 0, bytemuck::cast_slice(&metas));

        queue.submit(std::iter::empty());
        device.poll(wgpu::Maintain::Wait);
    });
}

#[test]
#[ignore = "parity deferred: follow up PR will diff instanced vs fragment paths pixel for pixel"]
fn parity_instanced_vs_fragment_plain_ascii() {
    // Scope discipline: this is a stretch goal from the issue test plan.
    // Full parity requires rendering the same text through both
    // pipelines into matching framebuffers and comparing bytes. Left as
    // a deferred follow up. See issue #87 for the remaining parity
    // cases (wide CJK, cursor block and bar, selection overlay,
    // scrollback offset, atlas generation bump).
}

#[test]
fn state_flow_matches_expected_plan_structure() {
    // GPU free smoke that walks the upload-plan state machine. No
    // adapter required; guards the invariant that the GridFragmentState
    // types are re exported from the crate root.
    let grid = CellGrid::new(2, 3);
    let mut state = GridFragmentState::new();
    let plan = state.prepare_frame(&grid, None, 0, |_| None);
    assert_eq!(plan.rows.len(), 2, "first frame emits every row");
    assert!(plan.resized);

    // Verify the byte ranges align with the cell struct size.
    for (range, cells) in &plan.rows {
        assert_eq!(range.byte_len, (cells.len() * std::mem::size_of::<GpuCell>()) as u64);
    }
}

#[test]
fn damaged_row_range_round_trip_without_gpu() {
    // Unit style sanity that `encode_row` + `damaged_row_write_ranges`
    // produce consistent byte counts without needing wgpu. Useful as a
    // first thing to run on CI to smoke the module without waiting on
    // a GPU to appear.
    let mut grid = CellGrid::new(1, 4);
    grid.set_cell(0, 0, Cell::with_char('h'));
    let ranges = damaged_row_write_ranges(&grid);
    assert_eq!(ranges.len(), 1);
    let row = encode_row(&grid, 0, None, |_| None);
    assert_eq!(ranges[0].byte_len, (row.len() * std::mem::size_of::<GpuCell>()) as u64);

    // Guardrails: the buffer stride exported by the pipeline matches
    // the upload module's cell size.
    assert_eq!(GPU_CELL_SIZE as usize, std::mem::size_of::<GpuCell>());
    assert_eq!(GPU_GLYPH_META_SIZE as usize, std::mem::size_of::<GpuGlyphMeta>());
}
