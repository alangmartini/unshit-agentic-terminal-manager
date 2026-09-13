#![cfg(target_os = "windows")]

use unshit_renderer::atlas::{GlyphAtlas, GlyphKey};
use unshit_renderer::pipeline::text::{GlyphInstance, TextPipeline};
use wgpu::util::DeviceExt;

const GLYPH_W: u32 = 13;
const GLYPH_H: u32 = 17;
const TARGET_W: u32 = 256;
const TARGET_H: u32 = 127;

/// A striped bitmap makes a one-texel sampling error visible without
/// depending on installed fonts. Every pixel in each stroke must agree,
/// including across the two triangles making up a glyph quad.
#[test]
fn terminal_glyph_strokes_survive_fractional_baselines() {
    for backend in [wgpu::Backends::VULKAN, wgpu::Backends::DX12] {
        assert_glyph_strokes(backend);
    }
}

fn assert_glyph_strokes(backend: wgpu::Backends) {
    let Some((device, queue)) = request_device(backend) else {
        eprintln!("skipping glyph sampling test: no {backend:?} GPU adapter");
        return;
    };
    for (subpixel, sample_count) in [(false, 1), (true, 1), (false, 4), (true, 4)] {
        let (glyphs, pixels) = render_stripe_glyphs(&device, &queue, subpixel, sample_count);
        assert_stripes_intact(&glyphs, &pixels, subpixel, sample_count);
    }
}

fn request_device(backend: wgpu::Backends) -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backend,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.ok()?;
        eprintln!("adapter: {:?}", adapter.get_info());
        adapter.request_device(&wgpu::DeviceDescriptor::default()).await.ok()
    })
}

/// Renders a row of striped glyphs at varying fractional baselines and reads
/// the target texture back to host memory.
fn render_stripe_glyphs(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    subpixel: bool,
    sample_count: u32,
) -> (Vec<GlyphInstance>, Vec<u8>) {
    let extent = wgpu::Extent3d { width: TARGET_W, height: TARGET_H, depth_or_array_layers: 1 };
    let format =
        if subpixel { wgpu::TextureFormat::Rgba8Unorm } else { wgpu::TextureFormat::R8Unorm };
    let mut atlas = GlyphAtlas::new_with_format(device, 2048, format);
    let data = (0..GLYPH_H)
        .flat_map(|y| {
            vec![
                if y % 2 == 0 { 255 } else { 0 };
                GLYPH_W as usize * atlas.bytes_per_pixel as usize
            ]
        })
        .collect();
    let entry = atlas
        .get_or_insert(
            GlyphKey { font_id: 0, glyph_id: 1, font_size_tenths: 200, subpixel_bin: 0 },
            GLYPH_W,
            GLYPH_H,
            data,
            [0.0; 2],
        )
        .unwrap();
    atlas.upload_pending(queue);
    let pipeline = TextPipeline::new(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        &atlas.texture_view,
        &atlas.sampler,
        sample_count,
        subpixel,
    );
    pipeline.update_uniforms(queue, TARGET_W as f32, TARGET_H as f32);
    let glyphs: Vec<_> = (0..8)
        .map(|i| GlyphInstance {
            pos: [7.5 + i as f32 * 29.0, if i % 2 == 0 { 20.5 } else { 20.0 }],
            size: entry.size,
            uv_min: [entry.uv_rect[0], entry.uv_rect[1]],
            uv_max: [entry.uv_rect[2], entry.uv_rect[3]],
            color: [1.0; 4],
            clip_rect: [0.0, 0.0, TARGET_W as f32, TARGET_H as f32],
            xform: [0.0; 4],
            xform_translate: [0.0, if i % 2 == 0 { 0.0 } else { 0.5 }],
        })
        .collect();
    let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("glyph sampling regression"),
        contents: bytemuck::cast_slice(&glyphs),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let msaa_view = msaa.create_view(&Default::default());
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (TARGET_W * TARGET_H * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: if sample_count > 1 { &msaa_view } else { &view },
                resolve_target: (sample_count > 1).then_some(&view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.uniform_bind_group, &[]);
        pass.set_bind_group(1, &pipeline.atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..6, 0..glyphs.len() as u32);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TARGET_W * 4),
                rows_per_image: Some(TARGET_H),
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let pixels = staging.slice(..).get_mapped_range().unwrap().to_vec();
    (glyphs, pixels)
}

/// Every pixel in each glyph's stripes must match the source bitmap,
/// including across the two triangles making up a glyph quad.
fn assert_stripes_intact(
    glyphs: &[GlyphInstance],
    pixels: &[u8],
    subpixel: bool,
    sample_count: u32,
) {
    for (i, glyph) in glyphs.iter().enumerate() {
        let origin_x = (glyph.pos[0] + 0.5).floor() as u32;
        let origin_y = (glyph.pos[1] + glyph.xform_translate[1] + 0.5).floor() as u32;
        for y in 0..GLYPH_H {
            for x in 0..GLYPH_W {
                let index = (((origin_y + y) * TARGET_W + origin_x + x) * 4) as usize;
                let expected = if y % 2 == 0 { 255u8 } else { 0 };
                assert!(
                    pixels[index].abs_diff(expected) <= 1,
                    "broken stroke: subpixel={subpixel}, samples={sample_count}, glyph={i}, \
                     pixel=({x},{y}), coverage={}, expected={expected}",
                    pixels[index]
                );
            }
        }
    }
}
