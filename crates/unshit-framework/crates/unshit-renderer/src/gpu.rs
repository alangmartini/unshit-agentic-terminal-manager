use crate::atlas::GlyphAtlas;
use crate::batch::{BackdropBoundary, LayeredBatch};
use crate::canvas::PaintContext;
use crate::image_cache::ImageCache;
use crate::persistent_buffer::GpuPersistentBuffers;
use crate::pipeline::backdrop_blur::{gaussian_weights, BackdropBlurPipeline, BlurUniforms};
use crate::pipeline::image::ImagePipeline;
use crate::pipeline::quad::QuadPipeline;
use crate::pipeline::svg::{SvgInstanceUniforms, SvgPipeline};
use crate::pipeline::text::TextPipeline;
use crate::svg_cache::SvgTessCache;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Once;
use wgpu;

static BACKDROP_FALLBACK_LOG: Once = Once::new();

/// Cached probe that decides whether the renderer can support
/// `backdrop-filter`. The check is extremely conservative: the swapchain
/// format must allow both `TEXTURE_BINDING` (so the blur shader can sample
/// the framebuffer) and `RENDER_ATTACHMENT` (so the renderer can draw
/// directly into the offscreen texture). Both usages are available on every
/// wgpu backend the framework currently targets, but we probe anyway to
/// keep the fallback path clean.
fn probe_backdrop_filter_support(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> bool {
    let features = adapter.get_texture_format_features(format);
    let needed = wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::COPY_DST;
    let available = features.allowed_usages.contains(needed);
    if !available {
        BACKDROP_FALLBACK_LOG.call_once(|| {
            log::info!(
                "backdrop-filter unavailable: surface format {:?} does not expose the required usages",
                format
            );
        });
    }
    available
}

/// Determines whether we render to a window surface or an offscreen texture.
pub enum RenderTarget {
    Window { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Headless { texture: wgpu::Texture, width: u32, height: u32 },
}

pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub target: RenderTarget,
    pub quad_pipeline: QuadPipeline,
    pub text_pipeline: TextPipeline,
    pub image_pipeline: ImagePipeline,
    pub svg_pipeline: SvgPipeline,
    pub glyph_atlas: GlyphAtlas,
    pub image_cache: ImageCache,
    pub svg_cache: SvgTessCache,
    pub layered_batch: LayeredBatch,
    pub capture_enabled: bool,
    capture_texture: Option<wgpu::Texture>,
    capture_view: Option<wgpu::TextureView>,

    // Backdrop filter state. All three fields remain `None` on frames that
    // do not use the property, which is what keeps the fast path at zero
    // cost.
    /// Cached check of whether the current device and surface format support
    /// rendering to and sampling from an offscreen texture at the surface
    /// format. If `false`, boundary markers are ignored at render time and
    /// the fallback path draws elements without the blur effect.
    pub backdrop_filter_available: bool,
    /// Offscreen texture used both as the primary render target on frames
    /// with boundaries and as the second ping pong slot during blur passes.
    /// Created lazily the first time a frame emits a boundary.
    pub backdrop_source: Option<wgpu::Texture>,
    /// Second ping pong texture for the separable blur.
    pub backdrop_blurred: Option<wgpu::Texture>,
    /// Lazily constructed blur pipeline. Stays `None` on frames that never
    /// use `backdrop-filter`.
    pub backdrop_blur_pipeline: Option<BackdropBlurPipeline>,

    /// Persistent GPU buffer allocations for canvas elements that opt into
    /// frame-surviving buffers. Populated by callers that also maintain the
    /// CPU-side `PersistentBufferManager`.
    pub gpu_persistent_buffers: GpuPersistentBuffers,
}

impl GpuContext {
    pub async fn new(window: Arc<dyn winit::window::Window>) -> Self {
        let size = window.surface_size();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        // Request the adapter's actual hardware limits so pipelines with many
        // vertex attributes (gradient quad pipeline: 23 attrs, 84 inter-stage
        // components) are accepted. Fall back to defaults if the adapter
        // rejects its own reported limits (software renderers).
        let (device, queue) = match adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("unshit device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                    ..Default::default()
                },
                None,
            )
            .await
        {
            Ok(dq) => dq,
            Err(_) => adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: Some("unshit device"),
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::default(),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .unwrap(),
        };

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let surface_caps = surface.get_capabilities(&adapter);
        // Use a non-sRGB format so blending happens in sRGB space, matching CSS.
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let quad_pipeline = QuadPipeline::new(&device, surface_format);
        #[cfg(target_os = "windows")]
        let glyph_atlas =
            GlyphAtlas::new_with_format(&device, 2048, wgpu::TextureFormat::Rgba8Unorm);
        #[cfg(not(target_os = "windows"))]
        let glyph_atlas = GlyphAtlas::new(&device);
        let text_pipeline = TextPipeline::new(
            &device,
            surface_format,
            &glyph_atlas.texture_view,
            &glyph_atlas.sampler,
        );
        let image_pipeline = ImagePipeline::new(&device, surface_format);
        let svg_pipeline = SvgPipeline::new(&device, surface_format);
        let image_cache = ImageCache::new(&device);

        let backdrop_filter_available = probe_backdrop_filter_support(&adapter, surface_format);

        Self {
            device,
            queue,
            target: RenderTarget::Window { surface, config: surface_config },
            quad_pipeline,
            text_pipeline,
            image_pipeline,
            svg_pipeline,
            glyph_atlas,
            image_cache,
            svg_cache: SvgTessCache::default(),
            layered_batch: LayeredBatch::new(),
            capture_enabled: false,
            capture_texture: None,
            capture_view: None,
            backdrop_filter_available,
            backdrop_source: None,
            backdrop_blurred: None,
            backdrop_blur_pipeline: None,
            gpu_persistent_buffers: GpuPersistentBuffers::new(),
        }
    }

    /// Override the SVG tessellation cache capacity. Useful for apps that
    /// need more than the default 256 entries.
    pub fn set_svg_cache_capacity(&mut self, capacity: usize) {
        self.svg_cache.set_capacity(capacity);
    }

    /// Creates a headless GPU context for offscreen rendering (no window required).
    /// Useful for screenshot testing and headless test harnesses.
    pub async fn new_headless(width: u32, height: u32) -> Self {
        Self::new_headless_with_backend(width, height, None).await
    }

    /// Creates a headless GPU context with an optional backend preference.
    /// Panics if no adapter is available. For a fallible variant, use
    /// `try_new_headless`.
    pub async fn new_headless_with_backend(
        width: u32,
        height: u32,
        preferred: Option<wgpu::Backends>,
    ) -> Self {
        Self::try_new_headless(width, height, preferred)
            .await
            .expect("[unshit-test] GPU init failed: no adapter available")
    }

    /// Like `new_headless_with_backend`, but returns `None` instead of
    /// panicking when no GPU adapter is available. Tries the preferred
    /// backend first, then falls back to a forced software adapter.
    pub async fn try_new_headless(
        width: u32,
        height: u32,
        preferred: Option<wgpu::Backends>,
    ) -> Option<Self> {
        let backends = preferred.unwrap_or(wgpu::Backends::all());

        // Try normal adapter
        if let Some(ctx) = Self::try_request_headless(backends, false, width, height).await {
            return Some(ctx);
        }

        // Fallback: forced software adapter with all backends
        eprintln!("[unshit-test] trying forced fallback adapter");
        Self::try_request_headless(wgpu::Backends::all(), true, width, height).await
    }

    /// Single attempt to create a headless context with the given backends
    /// and fallback adapter preference.
    async fn try_request_headless(
        backends: wgpu::Backends,
        force_fallback: bool,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let instance =
            wgpu::Instance::new(&wgpu::InstanceDescriptor { backends, ..Default::default() });

        let power = if force_fallback {
            wgpu::PowerPreference::LowPower
        } else {
            wgpu::PowerPreference::HighPerformance
        };

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power,
                compatible_surface: None,
                force_fallback_adapter: force_fallback,
            })
            .await;

        let adapter = match adapter {
            Some(a) => a,
            None => {
                eprintln!("[unshit-test] no adapter for backends {:?}", backends);
                return None;
            }
        };

        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("unshit headless device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                },
                None,
            )
            .await
            .ok()?;

        eprintln!("[unshit-test] using adapter: {} (backend: {:?})", info.name, info.backend);
        Some(Self::build_headless_context(
            Arc::new(device),
            Arc::new(queue),
            &adapter,
            width,
            height,
        ))
    }

    /// Shared builder for headless contexts, used by the fallback chain.
    fn build_headless_context(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        adapter: &wgpu::Adapter,
        width: u32,
        height: u32,
    ) -> Self {
        let format = wgpu::TextureFormat::Rgba8Unorm;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let quad_pipeline = QuadPipeline::new(&device, format);
        #[cfg(target_os = "windows")]
        let glyph_atlas =
            GlyphAtlas::new_with_format(&device, 2048, wgpu::TextureFormat::Rgba8Unorm);
        #[cfg(not(target_os = "windows"))]
        let glyph_atlas = GlyphAtlas::new(&device);
        let text_pipeline =
            TextPipeline::new(&device, format, &glyph_atlas.texture_view, &glyph_atlas.sampler);
        let image_pipeline = ImagePipeline::new(&device, format);
        let svg_pipeline = SvgPipeline::new(&device, format);
        let image_cache = ImageCache::new(&device);

        let backdrop_filter_available = probe_backdrop_filter_support(adapter, format);

        Self {
            device,
            queue,
            target: RenderTarget::Headless { texture, width, height },
            quad_pipeline,
            text_pipeline,
            image_pipeline,
            svg_pipeline,
            glyph_atlas,
            image_cache,
            svg_cache: SvgTessCache::default(),
            layered_batch: LayeredBatch::new(),
            capture_enabled: false,
            capture_texture: None,
            capture_view: None,
            backdrop_filter_available,
            backdrop_source: None,
            backdrop_blurred: None,
            backdrop_blur_pipeline: None,
            gpu_persistent_buffers: GpuPersistentBuffers::new(),
        }
    }

    /// Returns the texture format used by this context's render target.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        match &self.target {
            RenderTarget::Window { config, .. } => config.format,
            RenderTarget::Headless { texture, .. } => texture.format(),
        }
    }

    /// Enables frame capture for windowed mode. Creates an offscreen texture
    /// with COPY_SRC that the renderer draws to, then copies to the surface.
    /// Call this before `read_pixels()` when using a windowed context.
    pub fn enable_capture(&mut self) {
        let (w, h) = self.window_size();
        self.recreate_capture_texture(w as u32, h as u32);
        self.capture_enabled = true;
    }

    fn recreate_capture_texture(&mut self, width: u32, height: u32) {
        let format = self.surface_format();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.capture_texture = Some(texture);
        self.capture_view = Some(view);
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        let w = new_size.width;
        let h = new_size.height;
        if w == 0 || h == 0 {
            return;
        }

        match &mut self.target {
            RenderTarget::Window { surface, config } => {
                config.width = w;
                config.height = h;
                surface.configure(&self.device, config);
            }
            RenderTarget::Headless { texture, width, height } => {
                let format = texture.format();
                *texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("offscreen target"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                *width = w;
                *height = h;
            }
        }

        if self.capture_enabled {
            self.recreate_capture_texture(w, h);
        }

        // Drop any cached backdrop ping pong textures so the next frame that
        // actually uses the property reallocates them at the new size. We
        // intentionally leave `backdrop_blur_pipeline` alone: it is size
        // independent and only depends on the surface format.
        if self.backdrop_source.is_some() || self.backdrop_blurred.is_some() {
            self.backdrop_source = None;
            self.backdrop_blurred = None;
        }
    }

    pub fn window_size(&self) -> (f32, f32) {
        match &self.target {
            RenderTarget::Window { config, .. } => (config.width as f32, config.height as f32),
            RenderTarget::Headless { width, height, .. } => (*width as f32, *height as f32),
        }
    }

    pub fn render(&mut self) {
        let (vw, vh) = self.window_size();

        self.quad_pipeline.update_uniforms(&self.queue, vw, vh);
        self.text_pipeline.update_uniforms(&self.queue, vw, vh);
        self.image_pipeline.update_uniforms(&self.queue, vw, vh);
        self.svg_pipeline.update_globals(&self.queue, vw, vh);

        self.glyph_atlas.upload_pending(&self.queue);

        // Ensure GPU side vertex and index buffers exist for every SVG
        // geometry referenced this frame, then upload one instance uniform
        // block per draw call with a known stable ordering.
        let mut svg_instance_buffer: Vec<SvgInstanceUniforms> = Vec::new();
        let mut svg_keys: Vec<(usize, usize, usize)> = Vec::new(); // (layer_idx, draw_idx, geometry_key)
        let mut live_geometries: HashSet<usize> = HashSet::new();
        for (layer_idx, layer_batch) in self.layered_batch.layers.iter().enumerate() {
            for (draw_idx, draw) in layer_batch.svg_draws.iter().enumerate() {
                if let Some(key) = self.svg_pipeline.ensure_geometry(&self.device, &draw.geometry) {
                    svg_keys.push((layer_idx, draw_idx, key));
                    live_geometries.insert(key);
                    svg_instance_buffer.push(SvgInstanceUniforms {
                        translate: draw.translate,
                        scale: draw.scale,
                        clip_rect: draw.clip_rect,
                        color_tint: draw.color_tint,
                        opacity: draw.opacity,
                        _pad: [0.0; 7],
                    });
                }
            }
        }
        self.svg_pipeline.prune_unreferenced(&live_geometries);
        self.svg_pipeline.upload_instances(&self.device, &self.queue, &svg_instance_buffer);

        let (surface_view, surface_output) = match &self.target {
            RenderTarget::Window { surface, config } => {
                let output = match surface.get_current_texture() {
                    Ok(t) => t,
                    Err(wgpu::SurfaceError::Lost) => {
                        surface.configure(&self.device, config);
                        return;
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => panic!("GPU out of memory"),
                    Err(_) => return,
                };
                let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
                (view, Some(output))
            }
            RenderTarget::Headless { texture, .. } => {
                // Create a fresh view handle each frame (cheap, no GPU allocation).
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (view, None)
            }
        };

        // Prepare canvas painters before the render pass
        let format = self.surface_format();
        for layer_batch in &self.layered_batch.layers {
            for cb in &layer_batch.canvas_callbacks {
                cb.painter.prepare(&self.device, &self.queue, format, cb.rect);
            }
        }

        // Backdrop filter gate. We only take the split path when at least
        // one layer actually carries a boundary marker and the surface
        // format supports the required usages. When the gate is closed the
        // existing single pass code path below runs untouched, which keeps
        // normal pages at zero extra cost.
        let use_backdrop_path =
            self.backdrop_filter_available && self.layered_batch.has_backdrop_boundaries();

        if use_backdrop_path {
            self.ensure_backdrop_textures(vw as u32, vh as u32);
            if self.backdrop_blur_pipeline.is_none() {
                self.backdrop_blur_pipeline = Some(BackdropBlurPipeline::new(&self.device, format));
            }
        }

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render encoder"),
        });

        // The target that the walk draws into. When the backdrop path is
        // active we render to `backdrop_source` instead of the surface so
        // the blur shader can sample the partially drawn framebuffer. At
        // the end of the frame the offscreen texture is copied onto the
        // surface. When the backdrop path is not active the target is the
        // surface view (or the capture view, when capture is enabled).
        if use_backdrop_path {
            let view = self
                .backdrop_source
                .as_ref()
                .unwrap()
                .create_view(&wgpu::TextureViewDescriptor::default());
            self.render_with_backdrop_path(&mut encoder, &view, &svg_keys, vw, vh, format);
        } else {
            let render_view: &wgpu::TextureView = if self.capture_enabled {
                self.capture_view.as_ref().unwrap()
            } else {
                &surface_view
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: render_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.051,
                            g: 0.067,
                            b: 0.09,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Running index into `svg_instance_buffer` so each layer's
            // draws point at the correct per instance uniform slot. The
            // layers are walked in the same order both above and here.
            let mut svg_next: usize = 0;

            for (layer_idx, layer_batch) in self.layered_batch.layers.iter().enumerate() {
                let quad_count = layer_batch.quad_instances.len() as u32;
                if quad_count > 0 {
                    self.quad_pipeline.upload_instances(
                        &self.device,
                        &self.queue,
                        &layer_batch.quad_instances,
                    );
                    pass.set_pipeline(&self.quad_pipeline.pipeline);
                    pass.set_bind_group(0, &self.quad_pipeline.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.quad_pipeline.instance_buffer.slice(..));
                    pass.draw(0..6, 0..quad_count);
                }

                // SVG pass. Runs after quads and before text so icon fills
                // draw on top of background quads but under text content.
                if !layer_batch.svg_draws.is_empty() {
                    pass.set_pipeline(&self.svg_pipeline.pipeline);
                    pass.set_bind_group(0, self.svg_pipeline.global_bind_group(), &[]);
                    for _draw in &layer_batch.svg_draws {
                        // Advance to the matching entry in `svg_keys`. We
                        // only emitted entries for draws whose geometry
                        // was non empty and successfully uploaded.
                        if let Some((entry_layer, _draw_idx, geom_key)) =
                            svg_keys.get(svg_next).copied()
                        {
                            if entry_layer == layer_idx {
                                self.svg_pipeline.draw(&mut pass, geom_key, svg_next as u32);
                                svg_next += 1;
                            }
                        }
                    }
                }

                let glyph_count = layer_batch.glyph_instances.len() as u32;
                if glyph_count > 0 {
                    self.text_pipeline.upload_instances(
                        &self.device,
                        &self.queue,
                        &layer_batch.glyph_instances,
                    );
                    pass.set_pipeline(&self.text_pipeline.pipeline);
                    pass.set_bind_group(0, &self.text_pipeline.uniform_bind_group, &[]);
                    pass.set_bind_group(1, &self.text_pipeline.atlas_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.text_pipeline.instance_buffer.slice(..));
                    pass.draw(0..6, 0..glyph_count);
                }

                for image_batch in &layer_batch.image_batches {
                    if let Some(entry) = self.image_cache.get_or_load(
                        &image_batch.path,
                        &self.device,
                        &self.queue,
                        &self.image_pipeline.texture_bind_group_layout,
                    ) {
                        self.image_pipeline.upload_instances(
                            &self.device,
                            &self.queue,
                            &image_batch.instances,
                        );
                        let count = image_batch.instances.len() as u32;
                        pass.set_pipeline(&self.image_pipeline.pipeline);
                        pass.set_bind_group(0, &self.image_pipeline.uniform_bind_group, &[]);
                        pass.set_bind_group(1, &entry.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.image_pipeline.instance_buffer.slice(..));
                        pass.draw(0..6, 0..count);
                    }
                }

                // Canvas painters for this layer
                if !layer_batch.canvas_callbacks.is_empty() {
                    for cb in &layer_batch.canvas_callbacks {
                        let sx = cb.rect.x.max(cb.clip_rect[0]);
                        let sy = cb.rect.y.max(cb.clip_rect[1]);
                        let sr = (cb.rect.x + cb.rect.width).min(cb.clip_rect[0] + cb.clip_rect[2]);
                        let sb =
                            (cb.rect.y + cb.rect.height).min(cb.clip_rect[1] + cb.clip_rect[3]);
                        let sw = (sr - sx).max(0.0);
                        let sh = (sb - sy).max(0.0);
                        if sw > 0.0 && sh > 0.0 {
                            pass.set_scissor_rect(sx as u32, sy as u32, sw as u32, sh as u32);
                            let gpu_buf =
                                cb.node_id.and_then(|id| self.gpu_persistent_buffers.get(id));
                            let ctx = PaintContext {
                                rect: cb.rect,
                                clip_rect: cb.clip_rect,
                                viewport_size: (vw, vh),
                                surface_format: format,
                                device: &self.device,
                                queue: &self.queue,
                                persistent_buffer: gpu_buf,
                            };
                            cb.painter.paint(&ctx, &mut pass);
                        }
                    }
                    // Reset scissor to full viewport
                    pass.set_scissor_rect(0, 0, vw as u32, vh as u32);
                }
            }
        }

        // Without this copy the window would show nothing since we rendered
        // to the capture texture instead of the surface.
        if self.capture_enabled {
            if let (RenderTarget::Window { config, .. }, Some(ref output)) =
                (&self.target, &surface_output)
            {
                // When the backdrop path is active we first need to copy
                // from `backdrop_source` to the capture texture.
                if use_backdrop_path {
                    encoder.copy_texture_to_texture(
                        self.backdrop_source.as_ref().unwrap().as_image_copy(),
                        self.capture_texture.as_ref().unwrap().as_image_copy(),
                        wgpu::Extent3d {
                            width: config.width,
                            height: config.height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                encoder.copy_texture_to_texture(
                    self.capture_texture.as_ref().unwrap().as_image_copy(),
                    output.texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: config.width,
                        height: config.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        } else if use_backdrop_path {
            // Capture is off and we rendered into `backdrop_source`; copy
            // the result back onto the real target so it ends up on screen
            // or in the headless readback texture.
            match &self.target {
                RenderTarget::Window { config, .. } => {
                    if let Some(output) = surface_output.as_ref() {
                        encoder.copy_texture_to_texture(
                            self.backdrop_source.as_ref().unwrap().as_image_copy(),
                            output.texture.as_image_copy(),
                            wgpu::Extent3d {
                                width: config.width,
                                height: config.height,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                }
                RenderTarget::Headless { texture, width, height } => {
                    encoder.copy_texture_to_texture(
                        self.backdrop_source.as_ref().unwrap().as_image_copy(),
                        texture.as_image_copy(),
                        wgpu::Extent3d { width: *width, height: *height, depth_or_array_layers: 1 },
                    );
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        if let Some(output) = surface_output {
            output.present();
        }
    }

    /// Ensure the ping pong textures used by the backdrop blur path are
    /// allocated at the current surface size. Called lazily from `render`
    /// only on frames that actually use the property.
    fn ensure_backdrop_textures(&mut self, width: u32, height: u32) {
        let format = self.surface_format();
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let desc = |label: &'static str| wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let needs_recreate = |t: &Option<wgpu::Texture>| match t {
            Some(tex) => {
                let sz = tex.size();
                sz.width != width || sz.height != height
            }
            None => true,
        };
        if needs_recreate(&self.backdrop_source) {
            self.backdrop_source = Some(self.device.create_texture(&desc("backdrop source")));
        }
        if needs_recreate(&self.backdrop_blurred) {
            self.backdrop_blurred = Some(self.device.create_texture(&desc("backdrop blurred")));
        }
    }

    /// Full render pass walk when at least one layer carries a
    /// `BackdropBoundary`. Equivalent to the single pass code path on the
    /// else branch in `render`, but inserts blur passes at each boundary.
    fn render_with_backdrop_path(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        render_view: &wgpu::TextureView,
        svg_keys: &[(usize, usize, usize)],
        vw: f32,
        vh: f32,
        format: wgpu::TextureFormat,
    ) {
        let vwu = vw.max(0.0) as u32;
        let vhu = vh.max(0.0) as u32;

        // Copy the per layer boundary lists out of `self.layered_batch` so
        // the layer walk can call `&mut self` methods (the blur pass) from
        // inside the loop without fighting the borrow checker. Boundaries
        // are cheap `Copy` structs so this is a single small allocation per
        // frame with backdrop filters active, which is the path this whole
        // function exists for anyway.
        let per_layer_boundaries: Vec<Vec<BackdropBoundary>> =
            self.layered_batch.layers.iter().map(|l| l.backdrop_boundaries.clone()).collect();

        let mut pass_cleared = false;
        let mut svg_next: usize = 0;

        // We need `layer_idx` as a raw index to re borrow
        // `self.layered_batch.layers[layer_idx]` immutably inside the loop
        // between `&mut self` method calls, so the standard `.iter()` form
        // does not work here.
        #[allow(clippy::needless_range_loop)]
        for layer_idx in 0..self.layered_batch.layers.len() {
            let mut quad_cur: u32 = 0;
            let mut glyph_cur: u32 = 0;
            let mut svg_cur: u32 = 0;
            let mut image_cur: u32 = 0;
            let mut canvas_cur: u32 = 0;

            let boundaries = &per_layer_boundaries[layer_idx];
            let mut b_idx: usize = 0;

            loop {
                let (quad_end, glyph_end, svg_end, image_end, canvas_end) =
                    if let Some(b) = boundaries.get(b_idx) {
                        (
                            b.quad_prefix,
                            b.glyph_prefix,
                            b.svg_prefix,
                            b.image_batch_prefix,
                            b.canvas_prefix,
                        )
                    } else {
                        let layer_batch = &self.layered_batch.layers[layer_idx];
                        (
                            layer_batch.quad_instances.len() as u32,
                            layer_batch.glyph_instances.len() as u32,
                            layer_batch.svg_draws.len() as u32,
                            layer_batch.image_batches.len() as u32,
                            layer_batch.canvas_callbacks.len() as u32,
                        )
                    };

                if quad_cur < quad_end {
                    let slice: Vec<_> = self.layered_batch.layers[layer_idx].quad_instances
                        [..quad_end as usize]
                        .to_vec();
                    self.quad_pipeline.upload_instances(&self.device, &self.queue, &slice);
                }
                if glyph_cur < glyph_end {
                    let slice: Vec<_> = self.layered_batch.layers[layer_idx].glyph_instances
                        [..glyph_end as usize]
                        .to_vec();
                    self.text_pipeline.upload_instances(&self.device, &self.queue, &slice);
                }

                // Collect the canvas callbacks for this slice outside the
                // render pass scope so the `Arc<dyn CustomPainter>` inside
                // each `CanvasCallback` outlives `pass`.
                let canvas_slice: Vec<crate::canvas::CanvasCallback> = if canvas_cur < canvas_end {
                    self.layered_batch.layers[layer_idx].canvas_callbacks
                        [canvas_cur as usize..canvas_end as usize]
                        .to_vec()
                } else {
                    Vec::new()
                };

                {
                    let load = if pass_cleared {
                        wgpu::LoadOp::Load
                    } else {
                        pass_cleared = true;
                        wgpu::LoadOp::Clear(wgpu::Color { r: 0.051, g: 0.067, b: 0.09, a: 1.0 })
                    };
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("main pass (backdrop)"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: render_view,
                            resolve_target: None,
                            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    if quad_cur < quad_end {
                        pass.set_pipeline(&self.quad_pipeline.pipeline);
                        pass.set_bind_group(0, &self.quad_pipeline.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.quad_pipeline.instance_buffer.slice(..));
                        pass.draw(0..6, quad_cur..quad_end);
                    }

                    // SVG sub range.
                    if svg_cur < svg_end {
                        pass.set_pipeline(&self.svg_pipeline.pipeline);
                        pass.set_bind_group(0, self.svg_pipeline.global_bind_group(), &[]);
                        while let Some(&(entry_layer, draw_idx, geom_key)) = svg_keys.get(svg_next)
                        {
                            if entry_layer != layer_idx {
                                break;
                            }
                            if (draw_idx as u32) >= svg_end {
                                break;
                            }
                            if (draw_idx as u32) >= svg_cur {
                                self.svg_pipeline.draw(&mut pass, geom_key, svg_next as u32);
                            }
                            svg_next += 1;
                        }
                    }

                    if glyph_cur < glyph_end {
                        pass.set_pipeline(&self.text_pipeline.pipeline);
                        pass.set_bind_group(0, &self.text_pipeline.uniform_bind_group, &[]);
                        pass.set_bind_group(1, &self.text_pipeline.atlas_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.text_pipeline.instance_buffer.slice(..));
                        pass.draw(0..6, glyph_cur..glyph_end);
                    }

                    if image_cur < image_end {
                        let image_count = self.layered_batch.layers[layer_idx].image_batches.len();
                        let end = (image_end as usize).min(image_count);
                        for i in (image_cur as usize)..end {
                            // Clone the path + instances out so we don't
                            // hold a long lived borrow on the batch while
                            // we call `&mut self` methods.
                            let (path, instances): (String, Vec<_>) = {
                                let ib = &self.layered_batch.layers[layer_idx].image_batches[i];
                                (ib.path.clone(), ib.instances.clone())
                            };
                            if let Some(entry) = self.image_cache.get_or_load(
                                &path,
                                &self.device,
                                &self.queue,
                                &self.image_pipeline.texture_bind_group_layout,
                            ) {
                                self.image_pipeline.upload_instances(
                                    &self.device,
                                    &self.queue,
                                    &instances,
                                );
                                let count = instances.len() as u32;
                                pass.set_pipeline(&self.image_pipeline.pipeline);
                                pass.set_bind_group(
                                    0,
                                    &self.image_pipeline.uniform_bind_group,
                                    &[],
                                );
                                pass.set_bind_group(1, &entry.bind_group, &[]);
                                pass.set_vertex_buffer(
                                    0,
                                    self.image_pipeline.instance_buffer.slice(..),
                                );
                                pass.draw(0..6, 0..count);
                            }
                        }
                    }

                    if !canvas_slice.is_empty() {
                        let mut reset_scissor = false;
                        for cb in &canvas_slice {
                            let sx = cb.rect.x.max(cb.clip_rect[0]);
                            let sy = cb.rect.y.max(cb.clip_rect[1]);
                            let sr =
                                (cb.rect.x + cb.rect.width).min(cb.clip_rect[0] + cb.clip_rect[2]);
                            let sb =
                                (cb.rect.y + cb.rect.height).min(cb.clip_rect[1] + cb.clip_rect[3]);
                            let sw = (sr - sx).max(0.0);
                            let sh = (sb - sy).max(0.0);
                            if sw > 0.0 && sh > 0.0 {
                                pass.set_scissor_rect(sx as u32, sy as u32, sw as u32, sh as u32);
                                let gpu_buf =
                                    cb.node_id.and_then(|id| self.gpu_persistent_buffers.get(id));
                                let ctx = PaintContext {
                                    rect: cb.rect,
                                    clip_rect: cb.clip_rect,
                                    viewport_size: (vw, vh),
                                    surface_format: format,
                                    device: &self.device,
                                    queue: &self.queue,
                                    persistent_buffer: gpu_buf,
                                };
                                cb.painter.paint(&ctx, &mut pass);
                                reset_scissor = true;
                            }
                        }
                        if reset_scissor {
                            pass.set_scissor_rect(0, 0, vwu, vhu);
                        }
                    }
                    // pass drops here, closing it.
                }
                drop(canvas_slice);

                quad_cur = quad_end;
                glyph_cur = glyph_end;
                svg_cur = svg_end;
                image_cur = image_end;
                canvas_cur = canvas_end;

                // If there is a boundary here, run the blur passes on the
                // current contents of `backdrop_source` and then loop back
                // up to reopen the main pass and continue drawing.
                if let Some(boundary) = boundaries.get(b_idx).copied() {
                    let _ = boundary.dirty; // future hook for #117 item 8
                    self.run_backdrop_blur(encoder, &boundary, vw, vh);
                    b_idx += 1;
                } else {
                    break;
                }
            }

            while let Some(&(entry_layer, _, _)) = svg_keys.get(svg_next) {
                if entry_layer != layer_idx {
                    break;
                }
                svg_next += 1;
            }
        }
    }

    /// Run the two pass separable Gaussian blur for a single boundary.
    ///
    /// Source texture is `backdrop_source`, which the walk has just drawn
    /// into. The horizontal pass samples `backdrop_source` and writes to
    /// `backdrop_blurred`. The vertical pass samples `backdrop_blurred` and
    /// writes back to `backdrop_source`, so the main walk continues using
    /// the same target. Scissor clips the blur to the element rect.
    fn run_backdrop_blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        boundary: &BackdropBoundary,
        vw: f32,
        vh: f32,
    ) {
        let Some(pipeline) = self.backdrop_blur_pipeline.as_ref() else { return };
        let Some(source) = self.backdrop_source.as_ref() else { return };
        let Some(blurred) = self.backdrop_blurred.as_ref() else { return };

        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let blurred_view = blurred.create_view(&wgpu::TextureViewDescriptor::default());

        let texel = [if vw > 0.0 { 1.0 / vw } else { 0.0 }, if vh > 0.0 { 1.0 / vh } else { 0.0 }];
        let weights = gaussian_weights(boundary.blur_radius.round().max(0.0) as u32);

        // Scissor to the element rectangle so the blur shader only writes
        // into the region behind the filtered element, leaving the rest of
        // the target texture untouched.
        let sx = boundary.rect[0].max(0.0) as u32;
        let sy = boundary.rect[1].max(0.0) as u32;
        let max_w = (vw.max(0.0) as u32).saturating_sub(sx);
        let max_h = (vh.max(0.0) as u32).saturating_sub(sy);
        let sw = (boundary.rect[2].max(0.0) as u32).min(max_w);
        let sh = (boundary.rect[3].max(0.0) as u32).min(max_h);
        if sw == 0 || sh == 0 {
            return;
        }

        // Horizontal pass: sample source, write to blurred.
        let h_uniforms = BlurUniforms {
            direction: [1.0, 0.0],
            radius: boundary.blur_radius.round().max(0.0),
            _pad0: 0.0,
            texel_size: texel,
            _pad1: [0.0; 2],
            weights,
        };
        pipeline.upload_uniforms(&self.queue, &h_uniforms);
        let h_bind_group = pipeline.make_bind_group(&self.device, &source_view);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("backdrop blur horizontal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &h_bind_group, &[]);
            pass.set_scissor_rect(sx, sy, sw, sh);
            pass.draw(0..6, 0..1);
        }

        // Vertical pass: sample blurred, write back to source.
        let v_uniforms = BlurUniforms {
            direction: [0.0, 1.0],
            radius: boundary.blur_radius.round().max(0.0),
            _pad0: 0.0,
            texel_size: texel,
            _pad1: [0.0; 2],
            weights,
        };
        pipeline.upload_uniforms(&self.queue, &v_uniforms);
        let v_bind_group = pipeline.make_bind_group(&self.device, &blurred_view);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("backdrop blur vertical"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &source_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &v_bind_group, &[]);
            pass.set_scissor_rect(sx, sy, sw, sh);
            pass.draw(0..6, 0..1);
        }
    }

    /// Returns `true` if any registered canvas painter requests continuous repainting.
    pub fn any_canvas_needs_repaint(&self) -> bool {
        self.layered_batch
            .layers
            .iter()
            .any(|layer| layer.canvas_callbacks.iter().any(|cb| cb.painter.needs_repaint()))
    }

    /// Reads back RGBA pixel data from the render target.
    /// Works in headless mode (always) and windowed mode (requires `enable_capture()` first).
    /// Returns a Vec<u8> with `width * height * 4` bytes in RGBA order.
    pub fn read_pixels(&self) -> Vec<u8> {
        let (texture, width, height) = match &self.target {
            RenderTarget::Headless { texture, width, height, .. } => (texture, *width, *height),
            RenderTarget::Window { config, .. } => {
                let texture = self
                    .capture_texture
                    .as_ref()
                    .expect("call enable_capture() before read_pixels() in windowed mode");
                (texture, config.width, config.height)
            }
        };

        let bytes_per_row = 4 * width;
        // wgpu requires rows to be aligned to 256 bytes
        let padded_bytes_per_row = (bytes_per_row + 255) & !255;

        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback staging"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback encoder"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        let mapped = slice.get_mapped_range();

        // Strip row padding if present
        if padded_bytes_per_row == bytes_per_row {
            let pixels = mapped.to_vec();
            drop(mapped);
            staging.unmap();
            pixels
        } else {
            let mut pixels = Vec::with_capacity((bytes_per_row * height) as usize);
            for row in 0..height as usize {
                let start = row * padded_bytes_per_row as usize;
                let end = start + bytes_per_row as usize;
                pixels.extend_from_slice(&mapped[start..end]);
            }
            drop(mapped);
            staging.unmap();
            pixels
        }
    }
}
