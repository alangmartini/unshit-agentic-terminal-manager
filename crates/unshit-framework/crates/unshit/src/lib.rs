pub use unshit_app as app;
pub use unshit_core as core;
pub use unshit_macros::view;
pub use unshit_renderer as renderer;

pub mod prelude {
    #[cfg(feature = "async")]
    pub use crate::app::Subscription;
    pub use crate::app::{
        App, AppConfig, ClipboardContext, ClipboardError, EventSink, ExternalEvent,
    };
    pub use crate::core::prelude::*;
    pub use crate::renderer::canvas::{CanvasRegistry, CustomPainter, PaintContext};
    pub use crate::view;
}
