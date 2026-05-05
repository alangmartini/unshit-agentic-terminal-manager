use crate::atlas::GlyphAtlas;
use crate::batch::{BackdropBoundary, DrawKind, LayeredBatch};
use crate::canvas::PaintContext;
use crate::image_cache::ImageCache;
use crate::instance_buffer_pool::PooledBuffer;
use crate::persistent_buffer::GpuPersistentBuffers;
use crate::pipeline::backdrop_blur::{gaussian_weights, BackdropBlurPipeline, BlurUniforms};
#[cfg(feature = "grid-fragment-shader")]
use crate::pipeline::grid_fragment_pass::GridFragmentPass;
use crate::pipeline::image::ImageInstance;
use crate::pipeline::image::ImagePipeline;
use crate::pipeline::quad::{QuadInstance, QuadPipeline};
use crate::pipeline::svg::{SvgInstanceUniforms, SvgPipeline};
use crate::pipeline::text::{GlyphInstance, TextPipeline};
use crate::svg_cache::SvgTessCache;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use unshit_core::trace::{append_terminal_trace_line, terminal_trace_enabled};
use wgpu;

/// Per-(format, target_usages) dedup for the backdrop-filter fallback log.
/// `Once` would dedup the entire process and silently swallow a second
/// genuinely-different fallback (e.g. a window target then a headless
/// target with different capabilities). Keyed by (format, target_usages)
/// so each distinct combination is logged at most once.
type BackdropFallbackKey = (wgpu::TextureFormat, wgpu::TextureUsages);
static BACKDROP_FALLBACK_LOG: OnceLock<Mutex<HashSet<BackdropFallbackKey>>> = OnceLock::new();

fn log_backdrop_fallback_once(
    format: wgpu::TextureFormat,
    format_usages: wgpu::TextureUsages,
    target_usages: wgpu::TextureUsages,
) {
    let seen = BACKDROP_FALLBACK_LOG.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = seen.lock().unwrap_or_else(|p| p.into_inner());
    if guard.insert((format, target_usages)) {
        log::info!(
            "backdrop-filter unavailable: required usages missing (format={:?}, format_usages={:?}, target_usages={:?})",
            format,
            format_usages,
            target_usages,
        );
    }
}

/// Per layer, per image batch draw plan. Each entry is
/// `(slot, count)`: `slot` indexes into
/// `GpuContext::current_image_instance_buffers` (or equals `usize::MAX`
/// when the batch was skipped) and `count` is the instance count to
/// draw.
type ImageLayerPlan = Vec<Vec<(usize, u32)>>;

#[cfg(target_os = "windows")]
fn use_subpixel_text_shader() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TM_FORCE_SUBPIXEL_TEXT").is_some())
}

fn trace_text_draw_ranges() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        terminal_trace_enabled() && std::env::var_os("TM_TRACE_TEXT_RANGES").is_some()
    })
}

/// MSAA sample count for the main content pipelines. Set to 1 to disable.
const MSAA_SAMPLE_COUNT: u32 = 4;

fn parse_backend_env_value(value: &str) -> Option<wgpu::Backends> {
    match value.to_ascii_lowercase().as_str() {
        "vulkan" | "vk" => Some(wgpu::Backends::VULKAN),
        "dx12" | "d3d12" | "directx12" => Some(wgpu::Backends::DX12),
        "metal" => Some(wgpu::Backends::METAL),
        "gl" | "opengl" => Some(wgpu::Backends::GL),
        _ => None,
    }
}

fn renderer_backends() -> wgpu::Backends {
    for key in ["UNSHIT_RENDER_BACKEND", "WGPU_BACKEND"] {
        if let Ok(value) = std::env::var(key) {
            if let Some(backends) = parse_backend_env_value(&value) {
                log::info!("renderer backend forced by {key}={value}");
                return backends;
            }
            log::warn!("ignoring unsupported {key}={value}; expected vulkan, dx12, metal, or gl");
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Prefer Vulkan on Windows because some D3D12 adapters expose the
        // WebGPU minimum inter-stage component limit, which is too low for
        // the current quad shader's gradient payload. `UNSHIT_RENDER_BACKEND`
        // can still force D3D12 when a system needs it.
        wgpu::Backends::VULKAN
    }

    #[cfg(not(target_os = "windows"))]
    {
        wgpu::Backends::all()
    }
}

/// Compute the usage flags for the window `SurfaceConfiguration`. Request
/// only usages the surface advertises, plus the required render target usage.
/// Some backends exposed through RDP/RemoteApp only support rendering to the
/// swapchain texture, so optional copy usages must be negotiated.
fn surface_config_usages(surface_usages: wgpu::TextureUsages) -> wgpu::TextureUsages {
    let mut usages = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if surface_usages.contains(wgpu::TextureUsages::COPY_SRC) {
        usages |= wgpu::TextureUsages::COPY_SRC;
    }
    if surface_usages.contains(wgpu::TextureUsages::COPY_DST) {
        usages |= wgpu::TextureUsages::COPY_DST;
    }
    usages
}

/// Texture-format capabilities the backdrop-filter path needs. The format
/// must permit the offscreen ping-pong textures to be created with all of
/// these usages (`ensure_backdrop_textures` allocates them with
/// `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC | COPY_DST`).
const BACKDROP_FORMAT_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

/// Usages the configured render target must expose. The target only
/// participates in the backdrop path as the destination of the final
/// `copy_texture_to_texture`, so `COPY_DST` is the only flag we require
/// from it. Everything else (`TEXTURE_BINDING`, `RENDER_ATTACHMENT`,
/// `COPY_SRC`) belongs to the offscreen ping-pong textures, which are
/// allocated independently from the format.
const BACKDROP_TARGET_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::COPY_DST;

/// Pure decision function: backdrop-filter is available iff the format
/// permits every flag the ping-pong textures need AND the configured
/// target permits the flags the final copy needs. Split out from
/// `probe_backdrop_filter_support` so the AND-of-two-bitsets logic is
/// unit-testable without a real adapter.
fn probe_backdrop_filter_support_inner(
    format_usages: wgpu::TextureUsages,
    target_usages: wgpu::TextureUsages,
) -> bool {
    format_usages.contains(BACKDROP_FORMAT_USAGES) && target_usages.contains(BACKDROP_TARGET_USAGES)
}

/// Probe that decides whether the renderer can support `backdrop-filter`.
/// `target_usages` carries the configured render target's usages (the
/// window swapchain's `SurfaceConfiguration::usage` for window targets,
/// the offscreen texture's descriptor flags for headless targets). The
/// path is disabled when either the format cannot host the ping-pong
/// textures or the target cannot accept the final copy.
fn probe_backdrop_filter_support(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    target_usages: wgpu::TextureUsages,
) -> bool {
    let format_usages = adapter.get_texture_format_features(format).allowed_usages;
    let available = probe_backdrop_filter_support_inner(format_usages, target_usages);
    if !available {
        log_backdrop_fallback_once(format, format_usages, target_usages);
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

    /// Multisampled texture for MSAA. Content pipelines render into this
    /// texture and wgpu resolves to the single-sample surface at pass end.
    msaa_texture: Option<wgpu::Texture>,
    msaa_view: Option<wgpu::TextureView>,

    /// Non-MSAA (sample_count=1) pipeline duplicates for the backdrop blur
    /// path, created lazily. The backdrop path renders to single-sample
    /// textures so it cannot use the MSAA content pipelines.
    backdrop_quad_pipeline: Option<QuadPipeline>,
    backdrop_text_pipeline: Option<TextPipeline>,
    backdrop_image_pipeline: Option<ImagePipeline>,
    backdrop_svg_pipeline: Option<SvgPipeline>,

    /// Pooled instance buffers holding this frame's data. Acquired
    /// during `prepare`, referenced during the render pass, and
    /// released in the `on_submitted_work_done` callback after
    /// `queue.submit`. Images need one pooled buffer per batch because
    /// batch writes happen mid render pass and cannot safely share one
    /// buffer.
    current_quad_instance_buffer: Option<PooledBuffer<QuadInstance>>,
    current_glyph_instance_buffer: Option<PooledBuffer<GlyphInstance>>,
    current_image_instance_buffers: Vec<PooledBuffer<ImageInstance>>,

    /// Consumer for [`GridDrawRecord`](crate::batch::GridDrawRecord)s
    /// produced by the batch walk when the experimental fragment shader
    /// grid path is active. Step 2 wiring stub for issue #96: currently
    /// tracks stats only; Step 4 will issue draw calls.
    #[cfg(feature = "grid-fragment-shader")]
    pub grid_fragment_pass: GridFragmentPass,
}

impl GpuContext {
    pub async fn new(window: Arc<dyn winit::window::Window>) -> Self {
        let size = window.surface_size();
        let backends = renderer_backends();

        let instance =
            wgpu::Instance::new(&wgpu::InstanceDescriptor { backends, ..Default::default() });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();
        log::info!("renderer adapter selected: {:?}", adapter.get_info());

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

        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(surface_caps.alpha_modes[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: surface_config_usages(surface_caps.usages),
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let quad_pipeline = QuadPipeline::new(&device, surface_format, MSAA_SAMPLE_COUNT);
        #[cfg(target_os = "windows")]
        let glyph_atlas = GlyphAtlas::new_with_format(
            &device,
            2048,
            if use_subpixel_text_shader() {
                wgpu::TextureFormat::Rgba8Unorm
            } else {
                wgpu::TextureFormat::R8Unorm
            },
        );
        #[cfg(not(target_os = "windows"))]
        let glyph_atlas = GlyphAtlas::new(&device);
        let text_pipeline = TextPipeline::new(
            &device,
            surface_format,
            &glyph_atlas.texture_view,
            &glyph_atlas.sampler,
            MSAA_SAMPLE_COUNT,
        );
        let image_pipeline = ImagePipeline::new(&device, surface_format, MSAA_SAMPLE_COUNT);
        let svg_pipeline = SvgPipeline::new(&device, surface_format, MSAA_SAMPLE_COUNT);
        let image_cache = ImageCache::new(&device);

        let (msaa_texture, msaa_view) = if MSAA_SAMPLE_COUNT > 1 {
            let (t, v) = Self::create_msaa_texture(
                &device,
                surface_format,
                size.width.max(1),
                size.height.max(1),
            );
            (Some(t), Some(v))
        } else {
            (None, None)
        };

        let backdrop_filter_available =
            probe_backdrop_filter_support(&adapter, surface_format, surface_config.usage);

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
            msaa_texture,
            msaa_view,
            backdrop_quad_pipeline: None,
            backdrop_text_pipeline: None,
            backdrop_image_pipeline: None,
            backdrop_svg_pipeline: None,
            current_quad_instance_buffer: None,
            current_glyph_instance_buffer: None,
            current_image_instance_buffers: Vec::new(),
            #[cfg(feature = "grid-fragment-shader")]
            grid_fragment_pass: GridFragmentPass::new(),
        }
    }

    /// Override the SVG tessellation cache capacity. Useful for apps that
    /// need more than the default 256 entries.
    pub fn set_svg_cache_capacity(&mut self, capacity: usize) {
        self.svg_cache.set_capacity(capacity);
    }

    fn create_msaa_texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("MSAA texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: MSAA_SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Lazily create non-MSAA pipeline duplicates for the backdrop blur path.
    /// Called only when a frame uses backdrop-filter and MSAA is active.
    fn ensure_backdrop_pipelines(&mut self) {
        if self.backdrop_quad_pipeline.is_some() {
            return;
        }
        let format = self.surface_format();
        self.backdrop_quad_pipeline = Some(QuadPipeline::new(&self.device, format, 1));
        self.backdrop_text_pipeline = Some(TextPipeline::new(
            &self.device,
            format,
            &self.glyph_atlas.texture_view,
            &self.glyph_atlas.sampler,
            1,
        ));
        self.backdrop_image_pipeline = Some(ImagePipeline::new(&self.device, format, 1));
        self.backdrop_svg_pipeline =
            Some(self.svg_pipeline.create_backdrop_variant(&self.device, format));
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
        // The quad pipeline packs 23 vertex attributes (gradient stops
        // take 8 slots plus the common geometry and color slots), which
        // exceeds the wgpu default `max_vertex_attributes = 16`. Use
        // the adapter's real limits so the pipeline passes validation.
        // Fall back to defaults only if the adapter rejects its own
        // reported limits (software renderers).
        //
        // Gated on `TM_HEADLESS_ADAPTER_LIMITS` so pre existing tests
        // that rely on the quad pipeline panicking during construction
        // (and being silently skipped by `try_with_gpu`) keep doing so
        // until they are audited and fixed individually. The instance
        // pool integration tests set the env var to opt into the real
        // limits because they exercise the pool through a full render,
        // which requires the pipeline to build.
        let use_adapter_limits = std::env::var_os("TM_HEADLESS_ADAPTER_LIMITS").is_some();
        let limits = if use_adapter_limits { adapter.limits() } else { wgpu::Limits::default() };
        let (device, queue) = match adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("unshit headless device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
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
                        label: Some("unshit headless device"),
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::default(),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .ok()?,
        };

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

        // `COPY_DST` is required when the backdrop filter path copies the
        // `backdrop_source` texture onto the offscreen target at the end of
        // the render pass (see the branches in `render` around
        // `backdrop_source.as_ref().unwrap().as_image_copy()`). Kept in a
        // local so the same value is fed to the texture descriptor and to
        // `probe_backdrop_filter_support` below; declaring them separately
        // would let the two drift.
        let headless_target_usages = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: headless_target_usages,
            view_formats: &[],
        });
        let quad_pipeline = QuadPipeline::new(&device, format, MSAA_SAMPLE_COUNT);
        #[cfg(target_os = "windows")]
        let glyph_atlas = GlyphAtlas::new_with_format(
            &device,
            2048,
            if use_subpixel_text_shader() {
                wgpu::TextureFormat::Rgba8Unorm
            } else {
                wgpu::TextureFormat::R8Unorm
            },
        );
        #[cfg(not(target_os = "windows"))]
        let glyph_atlas = GlyphAtlas::new(&device);
        let text_pipeline = TextPipeline::new(
            &device,
            format,
            &glyph_atlas.texture_view,
            &glyph_atlas.sampler,
            MSAA_SAMPLE_COUNT,
        );
        let image_pipeline = ImagePipeline::new(&device, format, MSAA_SAMPLE_COUNT);
        let svg_pipeline = SvgPipeline::new(&device, format, MSAA_SAMPLE_COUNT);
        let image_cache = ImageCache::new(&device);

        let (msaa_texture, msaa_view) = if MSAA_SAMPLE_COUNT > 1 {
            let (t, v) = Self::create_msaa_texture(&device, format, width, height);
            (Some(t), Some(v))
        } else {
            (None, None)
        };

        let backdrop_filter_available =
            probe_backdrop_filter_support(adapter, format, headless_target_usages);

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
            msaa_texture,
            msaa_view,
            backdrop_quad_pipeline: None,
            backdrop_text_pipeline: None,
            backdrop_image_pipeline: None,
            backdrop_svg_pipeline: None,
            current_quad_instance_buffer: None,
            current_glyph_instance_buffer: None,
            current_image_instance_buffers: Vec::new(),
            #[cfg(feature = "grid-fragment-shader")]
            grid_fragment_pass: GridFragmentPass::new(),
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
                    // See `build_headless_context` for why `COPY_DST` is
                    // required.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
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

        if MSAA_SAMPLE_COUNT > 1 {
            let format = self.surface_format();
            let (t, v) = Self::create_msaa_texture(&self.device, format, w, h);
            self.msaa_texture = Some(t);
            self.msaa_view = Some(v);
        }
    }

    pub fn window_size(&self) -> (f32, f32) {
        match &self.target {
            RenderTarget::Window { config, .. } => (config.width as f32, config.height as f32),
            RenderTarget::Headless { width, height, .. } => (*width as f32, *height as f32),
        }
    }

    /// Rebuild text atlas bind groups so pipelines sample from the current
    /// atlas texture/view after atlas recreation events.
    pub fn refresh_glyph_atlas_bind_groups(&mut self) {
        self.text_pipeline.rebuild_atlas_bind_group(
            &self.device,
            &self.glyph_atlas.texture_view,
            &self.glyph_atlas.sampler,
        );
        if let Some(pipeline) = self.backdrop_text_pipeline.as_mut() {
            pipeline.rebuild_atlas_bind_group(
                &self.device,
                &self.glyph_atlas.texture_view,
                &self.glyph_atlas.sampler,
            );
        }
    }

    /// Upload one combined quad buffer and one combined glyph buffer for all
    /// layers, then return each layer's base offset into those global buffers.
    ///
    /// The buffers are acquired from the per pipeline `InstanceBufferPool`,
    /// not owned by the pipelines. The caller is responsible for
    /// releasing them inside `queue.on_submitted_work_done` after
    /// `queue.submit` so the GPU is done reading before the buffer
    /// re enters the free list.
    fn upload_content_instance_buffers(&mut self) -> (Vec<u32>, Vec<u32>) {
        let mut quad_bases = Vec::with_capacity(self.layered_batch.layers.len());
        let mut glyph_bases = Vec::with_capacity(self.layered_batch.layers.len());

        let total_quads: usize =
            self.layered_batch.layers.iter().map(|layer| layer.quad_instances.len()).sum();
        let total_glyphs: usize =
            self.layered_batch.layers.iter().map(|layer| layer.glyph_instances.len()).sum();

        let mut all_quads = Vec::with_capacity(total_quads);
        let mut all_glyphs = Vec::with_capacity(total_glyphs);

        for layer in &self.layered_batch.layers {
            quad_bases.push(all_quads.len() as u32);
            glyph_bases.push(all_glyphs.len() as u32);
            all_quads.extend_from_slice(&layer.quad_instances);
            all_glyphs.extend_from_slice(&layer.glyph_instances);
        }

        if !all_quads.is_empty() {
            let pooled = self.quad_pipeline.instance_pool.acquire(&self.device, all_quads.len());
            pooled.write(&self.queue, &all_quads);
            self.current_quad_instance_buffer = Some(pooled);
        } else {
            self.current_quad_instance_buffer = None;
        }
        if !all_glyphs.is_empty() {
            let pooled = self.text_pipeline.instance_pool.acquire(&self.device, all_glyphs.len());
            pooled.write(&self.queue, &all_glyphs);
            self.current_glyph_instance_buffer = Some(pooled);
        } else {
            self.current_glyph_instance_buffer = None;
        }

        (quad_bases, glyph_bases)
    }

    /// Fetch the currently acquired quad buffer for draw recording.
    /// Call sites guard with `quad_count > 0`, which implies the pool
    /// produced a buffer in `upload_content_instance_buffers`; the
    /// `expect` traps renderer bugs (e.g. counts and uploads drifting
    /// out of sync) instead of silently skipping draws.
    fn current_quad_buffer(&self) -> &wgpu::Buffer {
        self.current_quad_instance_buffer
            .as_ref()
            .map(|p| p.as_buffer())
            .expect("quad pool buffer must be acquired before draw")
    }

    /// Fetch the currently acquired glyph buffer for draw recording.
    /// See [`Self::current_quad_buffer`] for the invariant.
    fn current_glyph_buffer(&self) -> &wgpu::Buffer {
        self.current_glyph_instance_buffer
            .as_ref()
            .map(|p| p.as_buffer())
            .expect("glyph pool buffer must be acquired before draw")
    }

    /// Pre acquire one pooled buffer per image batch across all layers
    /// so the render pass never triggers a mid pass pool `acquire` or
    /// `queue.write_buffer` that would race with in flight draws.
    ///
    /// Image batches are walked in the same fixed order the render pass
    /// uses, so the nth batch in the returned `Vec` corresponds 1:1 with
    /// the nth batch encountered inside the pass.
    fn upload_image_instance_buffers(&mut self) -> ImageLayerPlan {
        // Clear any leftover pooled buffers from the previous frame.
        // Normally these are released by `on_submitted_work_done`, but
        // a frame with zero images followed by a frame with images
        // would otherwise pile up if we did not defend against it.
        self.current_image_instance_buffers.clear();

        let mut per_layer: ImageLayerPlan = Vec::with_capacity(self.layered_batch.layers.len());

        for layer_idx in 0..self.layered_batch.layers.len() {
            let batch_count = self.layered_batch.layers[layer_idx].image_batches.len();
            let mut per_batch = Vec::with_capacity(batch_count);
            for i in 0..batch_count {
                let (path, raw_instances, object_fit, object_position) = {
                    let ib = &self.layered_batch.layers[layer_idx].image_batches[i];
                    (ib.path.clone(), ib.instances.clone(), ib.object_fit, ib.object_position)
                };
                let entry = self.image_cache.get_or_load(
                    &path,
                    &self.device,
                    &self.queue,
                    &self.image_pipeline.texture_bind_group_layout,
                );
                let Some(entry) = entry else {
                    per_batch.push((usize::MAX, 0));
                    continue;
                };
                let instances = apply_object_fit(
                    &raw_instances,
                    object_fit,
                    object_position,
                    entry.width as f32,
                    entry.height as f32,
                );
                let count = instances.len() as u32;
                if count == 0 {
                    per_batch.push((usize::MAX, 0));
                    continue;
                }
                let pooled =
                    self.image_pipeline.instance_pool.acquire(&self.device, instances.len());
                pooled.write(&self.queue, &instances);
                let slot = self.current_image_instance_buffers.len();
                self.current_image_instance_buffers.push(pooled);
                per_batch.push((slot, count));
            }
            per_layer.push(per_batch);
        }
        per_layer
    }

    /// Look up a pooled image buffer previously acquired by
    /// `upload_image_instance_buffers`.
    fn image_instance_buffer(&self, slot: usize) -> Option<&wgpu::Buffer> {
        if slot == usize::MAX {
            None
        } else {
            self.current_image_instance_buffers.get(slot).map(|p| p.as_buffer())
        }
    }

    /// Take ownership of every pooled buffer held by this frame so the
    /// caller can hand them into `Queue::on_submitted_work_done`. After
    /// this returns `GpuContext` holds no references to the buffers,
    /// making it safe to start the next frame's prepare phase.
    fn take_pooled_frame_buffers(
        &mut self,
    ) -> (
        Option<PooledBuffer<QuadInstance>>,
        Option<PooledBuffer<GlyphInstance>>,
        Vec<PooledBuffer<ImageInstance>>,
        Option<PooledBuffer<crate::pipeline::svg::SvgInstanceSlot>>,
    ) {
        let quads = self.current_quad_instance_buffer.take();
        let glyphs = self.current_glyph_instance_buffer.take();
        let images = std::mem::take(&mut self.current_image_instance_buffers);
        let svg = self.svg_pipeline.take_current_instance_buffer();
        (quads, glyphs, images, svg)
    }

    pub fn render(&mut self) {
        let (vw, vh) = self.window_size();

        if trace_text_draw_ranges() {
            for (layer_idx, layer_batch) in self.layered_batch.layers.iter().enumerate() {
                let glyph_spans = layer_batch
                    .draw_spans
                    .iter()
                    .filter(|span| matches!(span.kind, DrawKind::Glyph))
                    .map(|span| format!("{}+{}", span.start, span.count))
                    .collect::<Vec<_>>()
                    .join("|");
                append_terminal_trace_line(&format!(
                    "terminal-trace stage=gpu_glyph_ranges layer={} glyphs={} quads={} spans={} fallback={}",
                    layer_idx,
                    layer_batch.glyph_instances.len(),
                    layer_batch.quad_instances.len(),
                    glyph_spans,
                    layer_batch.draw_spans.is_empty(),
                ));
            }
        }

        self.quad_pipeline.update_uniforms(&self.queue, vw, vh);
        self.text_pipeline.update_uniforms(&self.queue, vw, vh);
        self.image_pipeline.update_uniforms(&self.queue, vw, vh);
        self.svg_pipeline.update_globals(&self.queue, vw, vh);

        #[cfg(feature = "grid-fragment-shader")]
        {
            self.grid_fragment_pass.begin_frame();
            for layer_batch in &self.layered_batch.layers {
                self.grid_fragment_pass.process(&layer_batch.grid_records);
            }
        }

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
        let (quad_bases, glyph_bases) = self.upload_content_instance_buffers();
        let image_layer_plan = self.upload_image_instance_buffers();

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

        // Backdrop filter gate (checked before prepare so canvas painters
        // receive the correct sample count).
        let format = self.surface_format();
        let use_backdrop_path =
            self.backdrop_filter_available && self.layered_batch.has_backdrop_boundaries();

        // Prepare canvas painters before the render pass
        let paint_sample_count =
            if use_backdrop_path && MSAA_SAMPLE_COUNT > 1 { 1 } else { MSAA_SAMPLE_COUNT };
        for layer_batch in &self.layered_batch.layers {
            for cb in &layer_batch.canvas_callbacks {
                cb.painter.prepare(&self.device, &self.queue, format, cb.rect, paint_sample_count);
            }
        }

        if use_backdrop_path {
            self.ensure_backdrop_textures(vw as u32, vh as u32);
            if self.backdrop_blur_pipeline.is_none() {
                self.backdrop_blur_pipeline = Some(BackdropBlurPipeline::new(&self.device, format));
            }
            if MSAA_SAMPLE_COUNT > 1 {
                self.ensure_backdrop_pipelines();
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
            self.render_with_backdrop_path(
                &mut encoder,
                &view,
                &svg_keys,
                &quad_bases,
                &glyph_bases,
                &image_layer_plan,
                vw,
                vh,
                format,
            );
        } else {
            let render_view: &wgpu::TextureView = if self.capture_enabled {
                self.capture_view.as_ref().unwrap()
            } else {
                &surface_view
            };
            // When MSAA is active, render into the multisampled texture and
            // let wgpu resolve into the single-sample surface at pass end.
            let (pass_view, pass_resolve): (&wgpu::TextureView, Option<&wgpu::TextureView>) =
                if let Some(ref msaa_v) = self.msaa_view {
                    (msaa_v, Some(render_view))
                } else {
                    (render_view, None)
                };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: pass_view,
                    resolve_target: pass_resolve,
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
                let quad_base = quad_bases[layer_idx];
                let glyph_base = glyph_bases[layer_idx];
                if layer_batch.draw_spans.is_empty() {
                    // Fallback: existing render order (all quads, SVG, text, images, canvas)
                    let quad_count = layer_batch.quad_instances.len() as u32;
                    if quad_count > 0 {
                        pass.set_pipeline(&self.quad_pipeline.pipeline);
                        pass.set_bind_group(0, &self.quad_pipeline.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.current_quad_buffer().slice(..));
                        pass.draw(0..6, quad_base..quad_base + quad_count);
                    }

                    // SVG pass.
                    if !layer_batch.svg_draws.is_empty() {
                        pass.set_pipeline(&self.svg_pipeline.pipeline);
                        pass.set_bind_group(0, self.svg_pipeline.global_bind_group(), &[]);
                        for _draw in &layer_batch.svg_draws {
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
                        pass.set_pipeline(&self.text_pipeline.pipeline);
                        pass.set_bind_group(0, &self.text_pipeline.uniform_bind_group, &[]);
                        pass.set_bind_group(1, &self.text_pipeline.atlas_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.current_glyph_buffer().slice(..));
                        pass.draw(0..6, glyph_base..glyph_base + glyph_count);
                    }

                    let layer_image_plan = &image_layer_plan[layer_idx];
                    for (batch_idx, image_batch) in layer_batch.image_batches.iter().enumerate() {
                        let (slot, count) = layer_image_plan[batch_idx];
                        if count == 0 {
                            continue;
                        }
                        let Some(entry) = self.image_cache.get(&image_batch.path) else {
                            continue;
                        };
                        let Some(buffer) = self.image_instance_buffer(slot) else { continue };
                        pass.set_pipeline(&self.image_pipeline.pipeline);
                        pass.set_bind_group(0, &self.image_pipeline.uniform_bind_group, &[]);
                        pass.set_bind_group(1, &entry.bind_group, &[]);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..6, 0..count);
                    }

                    // Canvas painters for this layer
                    if !layer_batch.canvas_callbacks.is_empty() {
                        for cb in &layer_batch.canvas_callbacks {
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
                                    sample_count: MSAA_SAMPLE_COUNT,
                                };
                                cb.painter.paint(&ctx, &mut pass);
                            }
                        }
                        // Reset scissor to full viewport
                        pass.set_scissor_rect(0, 0, vw as u32, vh as u32);
                    }
                } else {
                    // Interleaved rendering path: upload all instances once,
                    // then draw spans sequentially to preserve painter's algorithm
                    // occlusion for overlapping elements.
                    let mut current_kind: Option<DrawKind> = None;
                    for span in &layer_batch.draw_spans {
                        if span.count == 0 {
                            continue;
                        }
                        match span.kind {
                            DrawKind::Quad => {
                                if current_kind != Some(DrawKind::Quad) {
                                    pass.set_pipeline(&self.quad_pipeline.pipeline);
                                    pass.set_bind_group(0, &self.quad_pipeline.bind_group, &[]);
                                    pass.set_vertex_buffer(0, self.current_quad_buffer().slice(..));
                                    current_kind = Some(DrawKind::Quad);
                                }
                                let start = quad_base + span.start;
                                pass.draw(0..6, start..start + span.count);
                            }
                            DrawKind::Glyph => {
                                if current_kind != Some(DrawKind::Glyph) {
                                    pass.set_pipeline(&self.text_pipeline.pipeline);
                                    pass.set_bind_group(
                                        0,
                                        &self.text_pipeline.uniform_bind_group,
                                        &[],
                                    );
                                    pass.set_bind_group(
                                        1,
                                        &self.text_pipeline.atlas_bind_group,
                                        &[],
                                    );
                                    pass.set_vertex_buffer(
                                        0,
                                        self.current_glyph_buffer().slice(..),
                                    );
                                    current_kind = Some(DrawKind::Glyph);
                                }
                                let start = glyph_base + span.start;
                                pass.draw(0..6, start..start + span.count);
                            }
                        }
                    }

                    // SVG, Image, Canvas render after interleaved quads+text
                    if !layer_batch.svg_draws.is_empty() {
                        pass.set_pipeline(&self.svg_pipeline.pipeline);
                        pass.set_bind_group(0, self.svg_pipeline.global_bind_group(), &[]);
                        for _draw in &layer_batch.svg_draws {
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

                    let layer_image_plan = &image_layer_plan[layer_idx];
                    for (batch_idx, image_batch) in layer_batch.image_batches.iter().enumerate() {
                        let (slot, count) = layer_image_plan[batch_idx];
                        if count == 0 {
                            continue;
                        }
                        let Some(entry) = self.image_cache.get(&image_batch.path) else {
                            continue;
                        };
                        let Some(buffer) = self.image_instance_buffer(slot) else { continue };
                        pass.set_pipeline(&self.image_pipeline.pipeline);
                        pass.set_bind_group(0, &self.image_pipeline.uniform_bind_group, &[]);
                        pass.set_bind_group(1, &entry.bind_group, &[]);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..6, 0..count);
                    }

                    if !layer_batch.canvas_callbacks.is_empty() {
                        for cb in &layer_batch.canvas_callbacks {
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
                                    sample_count: MSAA_SAMPLE_COUNT,
                                };
                                cb.painter.paint(&ctx, &mut pass);
                            }
                        }
                        pass.set_scissor_rect(0, 0, vw as u32, vh as u32);
                    }
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

        // Take every pooled instance buffer held by this frame and hand
        // them into the submit completion callback. The buffers return
        // to their respective pool free lists only after the GPU is
        // done reading them, preventing the CPU/GPU race that Zed fixed
        // with the same pattern in their Metal renderer (`zed:crates/
        // gpui_macos/src/metal_renderer.rs:478-484`). Without this
        // handoff, any frame that outruns the GPU would write over a
        // buffer still being read, producing visible glitches.
        let (quads, glyphs, images, svg) = self.take_pooled_frame_buffers();
        self.queue.on_submitted_work_done(move || {
            drop(quads);
            drop(glyphs);
            drop(images);
            drop(svg);
        });

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
        quad_bases: &[u32],
        glyph_bases: &[u32],
        image_layer_plan: &ImageLayerPlan,
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
            let quad_base = quad_bases[layer_idx];
            let glyph_base = glyph_bases[layer_idx];
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
                    // When MSAA is active, pick the non-MSAA pipeline
                    // duplicates for the backdrop path (single-sample
                    // target). Bind groups and buffers come from the main
                    // pipelines since the layouts are structurally identical.
                    let quad_rp = if MSAA_SAMPLE_COUNT > 1 {
                        &self.backdrop_quad_pipeline.as_ref().unwrap().pipeline
                    } else {
                        &self.quad_pipeline.pipeline
                    };
                    let text_rp = if MSAA_SAMPLE_COUNT > 1 {
                        &self.backdrop_text_pipeline.as_ref().unwrap().pipeline
                    } else {
                        &self.text_pipeline.pipeline
                    };
                    let svg_rp = if MSAA_SAMPLE_COUNT > 1 {
                        &self.backdrop_svg_pipeline.as_ref().unwrap().pipeline
                    } else {
                        &self.svg_pipeline.pipeline
                    };

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

                    // Check if draw_spans are available for interleaved rendering.
                    let layer_spans = &self.layered_batch.layers[layer_idx].draw_spans;
                    if !layer_spans.is_empty() {
                        // Interleaved rendering within this sub-range.
                        let mut current_kind: Option<DrawKind> = None;
                        for span in layer_spans {
                            if span.count == 0 {
                                continue;
                            }
                            let span_end = span.start + span.count;
                            match span.kind {
                                DrawKind::Quad => {
                                    // Only draw the portion of this span that falls
                                    // within the current sub-range [quad_cur, quad_end).
                                    let lo = span.start.max(quad_cur);
                                    let hi = span_end.min(quad_end);
                                    if lo < hi {
                                        if current_kind != Some(DrawKind::Quad) {
                                            pass.set_pipeline(quad_rp);
                                            pass.set_bind_group(
                                                0,
                                                &self.quad_pipeline.bind_group,
                                                &[],
                                            );
                                            pass.set_vertex_buffer(
                                                0,
                                                self.current_quad_buffer().slice(..),
                                            );
                                            current_kind = Some(DrawKind::Quad);
                                        }
                                        pass.draw(0..6, quad_base + lo..quad_base + hi);
                                    }
                                }
                                DrawKind::Glyph => {
                                    let lo = span.start.max(glyph_cur);
                                    let hi = span_end.min(glyph_end);
                                    if lo < hi {
                                        if current_kind != Some(DrawKind::Glyph) {
                                            pass.set_pipeline(text_rp);
                                            pass.set_bind_group(
                                                0,
                                                &self.text_pipeline.uniform_bind_group,
                                                &[],
                                            );
                                            pass.set_bind_group(
                                                1,
                                                &self.text_pipeline.atlas_bind_group,
                                                &[],
                                            );
                                            pass.set_vertex_buffer(
                                                0,
                                                self.current_glyph_buffer().slice(..),
                                            );
                                            current_kind = Some(DrawKind::Glyph);
                                        }
                                        pass.draw(0..6, glyph_base + lo..glyph_base + hi);
                                    }
                                }
                            }
                        }
                    } else {
                        // Fallback: original all-quads-then-all-glyphs order.
                        if quad_cur < quad_end {
                            pass.set_pipeline(quad_rp);
                            pass.set_bind_group(0, &self.quad_pipeline.bind_group, &[]);
                            pass.set_vertex_buffer(0, self.current_quad_buffer().slice(..));
                            pass.draw(0..6, quad_base + quad_cur..quad_base + quad_end);
                        }
                        if glyph_cur < glyph_end {
                            pass.set_pipeline(text_rp);
                            pass.set_bind_group(0, &self.text_pipeline.uniform_bind_group, &[]);
                            pass.set_bind_group(1, &self.text_pipeline.atlas_bind_group, &[]);
                            pass.set_vertex_buffer(0, self.current_glyph_buffer().slice(..));
                            pass.draw(0..6, glyph_base + glyph_cur..glyph_base + glyph_end);
                        }
                    }

                    // SVG sub range.
                    if svg_cur < svg_end {
                        pass.set_pipeline(svg_rp);
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

                    if image_cur < image_end {
                        let image_count = self.layered_batch.layers[layer_idx].image_batches.len();
                        let end = (image_end as usize).min(image_count);
                        let layer_image_plan = &image_layer_plan[layer_idx];
                        for i in (image_cur as usize)..end {
                            let (slot, count) = layer_image_plan[i];
                            if count == 0 {
                                continue;
                            }
                            let image_batch =
                                &self.layered_batch.layers[layer_idx].image_batches[i];
                            let Some(entry) = self.image_cache.get(&image_batch.path) else {
                                continue;
                            };
                            let Some(buffer) = self.image_instance_buffer(slot) else {
                                continue;
                            };
                            if MSAA_SAMPLE_COUNT > 1 {
                                pass.set_pipeline(
                                    &self.backdrop_image_pipeline.as_ref().unwrap().pipeline,
                                );
                            } else {
                                pass.set_pipeline(&self.image_pipeline.pipeline);
                            }
                            pass.set_bind_group(0, &self.image_pipeline.uniform_bind_group, &[]);
                            pass.set_bind_group(1, &entry.bind_group, &[]);
                            pass.set_vertex_buffer(0, buffer.slice(..));
                            pass.draw(0..6, 0..count);
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
                                    sample_count: 1,
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

/// Transform image instances based on CSS `object-fit` and `object-position`.
///
/// For `Fill`, instances are returned unchanged (image stretches to fill the box).
/// For other modes, `pos` and `size` are adjusted so the image maintains its
/// intrinsic aspect ratio while fitting/covering the layout box.
fn apply_object_fit(
    instances: &[ImageInstance],
    object_fit: unshit_core::style::types::ObjectFit,
    object_position: unshit_core::style::types::ObjectPosition,
    img_w: f32,
    img_h: f32,
) -> Vec<ImageInstance> {
    use unshit_core::style::types::ObjectFit;

    if object_fit == ObjectFit::Fill || img_w == 0.0 || img_h == 0.0 {
        return instances.to_vec();
    }

    instances
        .iter()
        .map(|inst| {
            let box_w = inst.size[0];
            let box_h = inst.size[1];
            let img_ratio = img_w / img_h;
            let box_ratio = box_w / box_h;

            let (draw_w, draw_h) = match object_fit {
                ObjectFit::Contain => {
                    if img_ratio > box_ratio {
                        (box_w, box_w / img_ratio)
                    } else {
                        (box_h * img_ratio, box_h)
                    }
                }
                ObjectFit::Cover => {
                    if img_ratio > box_ratio {
                        (box_h * img_ratio, box_h)
                    } else {
                        (box_w, box_w / img_ratio)
                    }
                }
                ObjectFit::None => (img_w, img_h),
                ObjectFit::ScaleDown => {
                    let (cw, ch) = if img_ratio > box_ratio {
                        (box_w, box_w / img_ratio)
                    } else {
                        (box_h * img_ratio, box_h)
                    };
                    // Use intrinsic size if smaller than contain result.
                    if img_w <= cw && img_h <= ch {
                        (img_w, img_h)
                    } else {
                        (cw, ch)
                    }
                }
                ObjectFit::Fill => unreachable!(),
            };

            let offset_x = (box_w - draw_w) * (object_position.x / 100.0);
            let offset_y = (box_h - draw_h) * (object_position.y / 100.0);

            ImageInstance {
                pos: [inst.pos[0] + offset_x, inst.pos[1] + offset_y],
                size: [draw_w, draw_h],
                ..*inst
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: clicking the window close button raised
    /// `Texture with '<Surface Texture>' label do not contain required usage
    /// flags TextureUsages(COPY_DST)` because the swapchain was configured
    /// without `COPY_DST` and the backdrop-filter path on the close-confirm
    /// dialog overlay (`backdrop-filter: blur(4px)` in `assets/styles.css`)
    /// then issued `copy_texture_to_texture` onto the surface texture.
    #[test]
    fn surface_config_usages_adds_copy_dst_when_supported() {
        let caps = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        let usages = surface_config_usages(caps);
        assert!(usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(usages.contains(wgpu::TextureUsages::COPY_SRC));
        assert!(usages.contains(wgpu::TextureUsages::COPY_DST));
    }

    /// On platforms whose surface refuses `COPY_DST` we must NOT request
    /// it; `surface.configure()` would error and the renderer relies on
    /// `probe_backdrop_filter_support` to disable the effect cleanly.
    #[test]
    fn surface_config_usages_omits_copy_dst_when_unsupported() {
        let caps = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let usages = surface_config_usages(caps);
        assert!(usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(usages.contains(wgpu::TextureUsages::COPY_SRC));
        assert!(!usages.contains(wgpu::TextureUsages::COPY_DST));
    }

    #[test]
    fn surface_config_usages_always_includes_base_usages() {
        let usages = surface_config_usages(wgpu::TextureUsages::empty());
        assert!(usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
    }

    #[test]
    fn surface_config_usages_omits_copy_src_when_unsupported() {
        let caps = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let usages = surface_config_usages(caps);
        assert_eq!(usages, wgpu::TextureUsages::RENDER_ATTACHMENT);
    }

    /// Pins the additive-only contract: even when the input caps include
    /// usages outside the swapchain's needs (e.g. `STORAGE_BINDING`,
    /// `TEXTURE_BINDING`), the helper must only enable supported copy flags
    /// from the extra-flag set.
    #[test]
    fn surface_config_usages_does_not_pass_through_unrelated_flags() {
        let usages = surface_config_usages(wgpu::TextureUsages::all());
        let allowed = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        assert_eq!(
            usages, allowed,
            "surface_config_usages must be additive on supported copy flags only; \
             unrelated flags from caps must not leak into the swapchain config"
        );
    }

    #[test]
    fn probe_inner_true_when_both_requirements_met() {
        assert!(probe_backdrop_filter_support_inner(
            BACKDROP_FORMAT_USAGES,
            BACKDROP_TARGET_USAGES
        ));
    }

    #[test]
    fn probe_inner_false_when_format_is_missing_a_required_flag() {
        let format = BACKDROP_FORMAT_USAGES - wgpu::TextureUsages::TEXTURE_BINDING;
        assert!(!probe_backdrop_filter_support_inner(format, BACKDROP_TARGET_USAGES));
    }

    /// Direct regression: the original bug was a target configured
    /// without `COPY_DST` while the format advertised it. The probe must
    /// return false in that case so the renderer disables the backdrop
    /// path instead of panicking when it tries to copy onto the swapchain.
    #[test]
    fn probe_inner_false_when_target_is_missing_copy_dst() {
        let target = BACKDROP_TARGET_USAGES - wgpu::TextureUsages::COPY_DST;
        assert!(!probe_backdrop_filter_support_inner(BACKDROP_FORMAT_USAGES, target));
    }

    #[test]
    fn probe_inner_false_when_neither_supplies_any_flag() {
        assert!(!probe_backdrop_filter_support_inner(
            wgpu::TextureUsages::empty(),
            wgpu::TextureUsages::empty()
        ));
    }

    /// Extra superset bits on either side must not change the decision:
    /// the probe asks "do you contain everything I need", not "are you
    /// exactly this set".
    #[test]
    fn probe_inner_ignores_unrelated_extra_flags() {
        let extra = wgpu::TextureUsages::STORAGE_BINDING;
        assert!(probe_backdrop_filter_support_inner(
            BACKDROP_FORMAT_USAGES | extra,
            BACKDROP_TARGET_USAGES | extra
        ));
    }

    /// Wires the helper into the probe end to end: when the platform
    /// advertises every backdrop-format flag on the surface, feeding that
    /// into `surface_config_usages` and then into the probe must say
    /// "available". Catches refactors that keep `surface_config_usages`
    /// correct on its own but drop `COPY_DST` on the way to the probe.
    #[test]
    fn helper_output_satisfies_probe_when_caps_advertise_full_support() {
        let surface_caps = BACKDROP_FORMAT_USAGES;
        let configured_target = surface_config_usages(surface_caps);
        assert!(probe_backdrop_filter_support_inner(BACKDROP_FORMAT_USAGES, configured_target));
    }

    /// Inverse of the above: when the surface refuses `COPY_DST`, the
    /// helper output must propagate that refusal so the probe says
    /// "unavailable" rather than "yes, go ahead and copy".
    #[test]
    fn helper_output_disables_probe_when_caps_lack_copy_dst() {
        let surface_caps = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let configured_target = surface_config_usages(surface_caps);
        assert!(!probe_backdrop_filter_support_inner(BACKDROP_FORMAT_USAGES, configured_target));
    }
}
