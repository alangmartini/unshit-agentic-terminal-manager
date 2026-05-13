use std::sync::{Arc, OnceLock};
use winit::event_loop::EventLoopProxy;

/// Opaque event type that external sources push into the framework.
pub enum ExternalEvent {
    /// Request a full tree rebuild + re-render.
    RequestRebuild,
    /// Request re-render without rebuilding the tree (repaint only).
    RequestRedraw,
    /// Ask the application window to become visible, unminimized, focused,
    /// and attention-requesting where the platform allows it.
    ActivateWindow,
    /// Minimize the application window.
    MinimizeWindow,
    /// Toggle the application window between maximized and restored states.
    ToggleMaximizeWindow,
    /// User-defined payload (type-erased).
    Custom(Box<dyn std::any::Any + Send>),
    /// Zero-copy byte payload. Only the Arc refcount is bumped on send.
    Bytes(Arc<[u8]>),
    /// Hot-reload: a new stylesheet was parsed from a watched CSS file.
    #[cfg(feature = "hot-reload")]
    StylesheetReload(Box<unshit_core::style::parse::CompiledStylesheet>),
}

/// Error returned when the receiver has been dropped (event loop shut down).
pub struct SendError(pub ExternalEvent);

impl std::fmt::Debug for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendError").finish_non_exhaustive()
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "event loop has been shut down")
    }
}

impl std::error::Error for SendError {}

/// Handle given to external producers. `Clone + Send + Sync`.
///
/// Pushing an event wakes the event loop automatically.
/// Create one via [`crate::app::App::event_sink`].
#[derive(Clone)]
pub struct EventSink {
    tx: flume::Sender<ExternalEvent>,
    proxy: Arc<OnceLock<EventLoopProxy>>,
}

impl EventSink {
    pub(crate) fn new(
        tx: flume::Sender<ExternalEvent>,
        proxy: Arc<OnceLock<EventLoopProxy>>,
    ) -> Self {
        Self { tx, proxy }
    }

    /// Send an event and wake the UI loop.
    ///
    /// Channel-first, wake-second ordering (per winit docs).
    /// If the event loop has not started yet the event is buffered in the
    /// channel and will be drained on the first `proxy_wake_up`.
    pub fn send(&self, event: ExternalEvent) -> Result<(), SendError> {
        self.tx.send(event).map_err(|e| SendError(e.into_inner()))?;
        if let Some(proxy) = self.proxy.get() {
            proxy.wake_up();
        }
        Ok(())
    }

    /// Async-compatible send for use inside tokio/async-std tasks.
    pub async fn send_async(&self, event: ExternalEvent) -> Result<(), SendError> {
        self.tx.send_async(event).await.map_err(|e| SendError(e.into_inner()))?;
        if let Some(proxy) = self.proxy.get() {
            proxy.wake_up();
        }
        Ok(())
    }

    /// Send a byte payload without copying. Only bumps the Arc refcount.
    pub fn send_bytes(&self, data: Arc<[u8]>) -> Result<(), SendError> {
        self.send(ExternalEvent::Bytes(data))
    }

    /// Minimize the application window.
    pub fn minimize_window(&self) -> Result<(), SendError> {
        self.send(ExternalEvent::MinimizeWindow)
    }

    /// Toggle the application window between maximized and restored states.
    pub fn toggle_maximize_window(&self) -> Result<(), SendError> {
        self.send(ExternalEvent::ToggleMaximizeWindow)
    }

    /// Async variant of send_bytes.
    pub async fn send_bytes_async(&self, data: Arc<[u8]>) -> Result<(), SendError> {
        self.send_async(ExternalEvent::Bytes(data)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sink() -> (EventSink, flume::Receiver<ExternalEvent>) {
        let (tx, rx) = flume::unbounded();
        let proxy = Arc::new(OnceLock::new());
        (EventSink::new(tx, proxy), rx)
    }

    #[test]
    fn bytes_variant_constructs() {
        let data: Arc<[u8]> = Arc::from(b"hello".as_ref());
        let _event = ExternalEvent::Bytes(data);
    }

    #[test]
    fn minimize_window_enqueues_window_control_event() {
        let (sink, rx) = make_sink();

        sink.minimize_window().unwrap();

        assert!(matches!(rx.try_recv().unwrap(), ExternalEvent::MinimizeWindow));
    }

    #[test]
    fn toggle_maximize_window_enqueues_window_control_event() {
        let (sink, rx) = make_sink();

        sink.toggle_maximize_window().unwrap();

        assert!(matches!(rx.try_recv().unwrap(), ExternalEvent::ToggleMaximizeWindow));
    }

    #[test]
    fn send_bytes_delivers_correct_variant() {
        let (sink, rx) = make_sink();
        let data: Arc<[u8]> = Arc::from(b"zero-copy".as_ref());
        sink.send_bytes(data.clone()).unwrap();
        let event = rx.try_recv().unwrap();
        match event {
            ExternalEvent::Bytes(received) => {
                assert_eq!(&*received, b"zero-copy");
            }
            _ => panic!("expected Bytes variant"),
        }
    }

    #[test]
    fn send_bytes_same_pointer() {
        let (sink, rx) = make_sink();
        let data: Arc<[u8]> = Arc::from(b"ptr-check".as_ref());
        let ptr_before = Arc::as_ptr(&data);
        sink.send_bytes(data).unwrap();
        let event = rx.try_recv().unwrap();
        match event {
            ExternalEvent::Bytes(received) => {
                // Same underlying allocation — no copy was made.
                assert_eq!(Arc::as_ptr(&received), ptr_before);
            }
            _ => panic!("expected Bytes variant"),
        }
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn stylesheet_reload_variant_constructs() {
        let stylesheet =
            unshit_core::style::parse::CompiledStylesheet::parse(".foo { color: red; }");
        let _event = ExternalEvent::StylesheetReload(Box::new(stylesheet));
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn stylesheet_reload_can_be_sent() {
        let (sink, rx) = make_sink();
        let stylesheet = unshit_core::style::parse::CompiledStylesheet::parse(
            ".bar { background-color: blue; }",
        );
        sink.send(ExternalEvent::StylesheetReload(Box::new(stylesheet))).unwrap();
        let event = rx.try_recv().unwrap();
        match event {
            ExternalEvent::StylesheetReload(sheet) => {
                // Verify parse returned something with at least one rule.
                assert!(!sheet.rules.is_empty());
            }
            _ => panic!("expected StylesheetReload variant"),
        }
    }
}
