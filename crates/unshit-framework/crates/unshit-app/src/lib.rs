pub mod animation_waker;
pub mod app;
pub mod clipboard;
pub mod event_sink;
pub mod font;
pub mod frame_pacer;
pub mod frame_probe;
#[cfg(feature = "input-latency-histogram")]
pub mod input_latency;
pub mod notification;
#[cfg(feature = "async")]
pub mod runtime;
pub mod scroll_motion;
pub mod shortcut;
#[cfg(feature = "async")]
pub mod subscription;
pub mod window;

pub use app::{
    App, AppConfig, FrameMetrics, GridAnimationHook, GridTick, ScrollGridPatch, ScrollTelemetry,
    ScrollTelemetryCallback, ScrollTelemetryPhase, ScrollTuning, DEFAULT_SMOOTH_SCROLL_DURATION_MS,
    DEFAULT_WHEEL_LINE_SCROLL_PX,
};
pub use clipboard::{ClipboardContent, ClipboardContext, ClipboardError, ClipboardFormat};
pub use event_sink::{EventSink, ExternalEvent, SendError};
pub use font::{
    check_fallback_chain, load_custom_fonts, FallbackChain, FontLoadReport, FontSource,
};
pub use frame_pacer::{FramePacer, PaceDecision};
pub use frame_probe::{FrameProbe, FrameQuantiles};
#[cfg(feature = "input-latency-histogram")]
pub use input_latency::{InputLatencySnapshot, InputLatencyTracker};
pub use notification::{AttentionUrgency, BellConfig, BellState, BellStyle};
#[cfg(feature = "async")]
pub use runtime::AsyncRuntime;
pub use scroll_motion::ScrollMotion;
pub use shortcut::ShortcutResolver;
#[cfg(feature = "async")]
pub use subscription::Subscription;
