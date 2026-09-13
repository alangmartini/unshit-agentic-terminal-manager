#![cfg(target_os = "windows")]

use unshit_renderer::atlas::{GlyphAtlas, GlyphKey};

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
    use unshit_renderer::pipeline::text::{GlyphInstance, TextPipeline};
    use wgpu::util::DeviceExt;

    let Some((device, queue)) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backend,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.ok()?;
        eprintln!("adapter: {:?}", adapter.get_info());
        adapter.request_device(&wgpu::DeviceDescriptor::default()).await.ok()
    }) else {
        eprintln!("skipping glyph sampling test: no {backend:?} GPU adapter");
        return;
    };
    let (width, height) = (256u32, 127u32);
    let extent = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    for (subpixel, sample_count) in [(false, 1), (true, 1), (false, 4), (true, 4)] {
        let format =
            if subpixel { wgpu::TextureFormat::Rgba8Unorm } else { wgpu::TextureFormat::R8Unorm };
        let mut atlas = GlyphAtlas::new_with_format(&device, 2048, format);
        let data = (0..17)
            .flat_map(|y| {
                vec![if y % 2 == 0 { 255 } else { 0 }; 13 * atlas.bytes_per_pixel as usize]
            })
            .collect();
        let entry = atlas
            .get_or_insert(
                GlyphKey { font_id: 0, glyph_id: 1, font_size_tenths: 200, subpixel_bin: 0 },
                13,
                17,
                data,
                [0.0; 2],
            )
            .unwrap();
        atlas.upload_pending(&queue);
        let pipeline = TextPipeline::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            &atlas.texture_view,
            &atlas.sampler,
            sample_count,
            subpixel,
        );
        pipeline.update_uniforms(&queue, width as f32, height as f32);
        let glyphs: Vec<_> = (0..8)
            .map(|i| GlyphInstance {
                pos: [7.5 + i as f32 * 29.0, if i % 2 == 0 { 20.5 } else { 20.0 }],
                size: entry.size,
                uv_min: [entry.uv_rect[0], entry.uv_rect[1]],
                uv_max: [entry.uv_rect[2], entry.uv_rect[3]],
                color: [1.0; 4],
                clip_rect: [0.0, 0.0, width as f32, height as f32],
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
            size: (width * height * 4) as u64,
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
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = staging.slice(..).get_mapped_range().unwrap();
        for (i, glyph) in glyphs.iter().enumerate() {
            for y in 0..17 {
                for x in 0..13 {
                    let index = (((21 + y) * width + glyph.pos[0].round() as u32 + x) * 4) as usize;
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
}
