use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use unshit_core::cell_grid::CellGrid;
use unshit_core::id::{NodeId, NodeRef};
use winit::event_loop::EventLoopProxy;

/// Opaque event type that external sources push into the framework.
pub enum ExternalEvent {
    /// Request a full tree rebuild + re-render.
    RequestRebuild,
    /// Request re-render without rebuilding the tree (repaint only).
    RequestRedraw,
    /// Request an animation repaint. This bypasses redraw coalescing once so
    /// short-lived motion does not stall behind unrelated frame pacing.
    RequestAnimationFrame,
    /// One native compositor heartbeat. The timestamp is captured on the
    /// waiting thread immediately after the clock wakes, before event-loop
    /// scheduling jitter can affect cadence telemetry.
    RequestCompositorFrame { tick_at: Instant },
    /// Ask the application window to become visible, unminimized, focused,
    /// and attention-requesting where the platform allows it.
    ActivateWindow,
    /// Minimize the application window.
    MinimizeWindow,
    /// Toggle the application window between maximized and restored states.
    ToggleMaximizeWindow,
    /// Ask the native window manager to resize the drawable surface. The
    /// resulting surface notification follows the ordinary resize path, so
    /// layout and GPU configuration retain their usual coalescing behavior.
    RequestSurfaceSize { width: u32, height: u32 },
    /// User-defined payload (type-erased).
    Custom(Box<dyn std::any::Any + Send>),
    /// Zero-copy byte payload. Only the Arc refcount is bumped on send.
    Bytes(Arc<[u8]>),
    /// Replace one mounted grid's same-sized content and repaint that node
    /// without rebuilding the element tree. [`EventSink::send`] coalesces
    /// successive patches for the same mounted node before they enter the
    /// external-event queue. A stale or unmounted reference safely falls
    /// back to a rebuild at delivery time.
    GridPatch { node: NodeRef, grid: Box<CellGrid> },
    /// Replace a mounted grid whose cell dimensions changed while its element
    /// box remains layout-independent (for example, a terminal viewport).
    /// This is delivered directly rather than merged with high-rate output
    /// patches so a resize cannot be overwritten by an older grid snapshot.
    GridResizePatch { node: NodeRef, grid: Box<CellGrid> },
    /// Internal notification emitted after [`EventSink`] has accumulated one
    /// or more latest-wins grid patches. Application code should use
    /// [`ExternalEvent::GridPatch`] instead.
    #[doc(hidden)]
    DrainGridPatches,
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

#[derive(Default)]
struct PendingGridPatches {
    patches: HashMap<NodeId, Box<CellGrid>>,
    drain_enqueued: bool,
}

/// Per-application latest-wins storage for high-rate paint-only grid updates.
/// It is shared by every [`EventSink`] made by an [`crate::app::App`].
///
/// The lock deliberately covers only a HashMap insert/drain. It keeps a PTY
/// producer from allocating one queued event per parsed output batch when the
/// UI thread is already waiting to paint; the next drain receives the latest
/// complete grid for each node instead.
#[derive(Default)]
pub(crate) struct GridPatchStore {
    pending: Mutex<PendingGridPatches>,
}

impl GridPatchStore {
    fn push(
        &self,
        node_id: NodeId,
        grid: Box<CellGrid>,
        tx: &flume::Sender<ExternalEvent>,
    ) -> Result<(), SendError> {
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.patches.insert(node_id, grid);
        if !pending.drain_enqueued {
            tx.send(ExternalEvent::DrainGridPatches).map_err(|err| SendError(err.into_inner()))?;
            pending.drain_enqueued = true;
        }
        Ok(())
    }

    pub(crate) fn take(&self) -> HashMap<NodeId, Box<CellGrid>> {
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Clear this under the same lock that producers use. A producer that
        // arrives after the drain takes ownership will enqueue the next wake,
        // so no output update can be stranded between frames.
        pending.drain_enqueued = false;
        std::mem::take(&mut pending.patches)
    }
}

/// Handle given to external producers. `Clone + Send + Sync`.
///
/// Pushing an event wakes the event loop automatically.
/// Create one via [`crate::app::App::event_sink`].
#[derive(Clone)]
pub struct EventSink {
    tx: flume::Sender<ExternalEvent>,
    proxy: Arc<OnceLock<EventLoopProxy>>,
    grid_patches: Arc<GridPatchStore>,
}

impl EventSink {
    #[cfg(test)]
    pub(crate) fn new(
        tx: flume::Sender<ExternalEvent>,
        proxy: Arc<OnceLock<EventLoopProxy>>,
    ) -> Self {
        Self::with_grid_patch_store(tx, proxy, Arc::default())
    }

    pub(crate) fn with_grid_patch_store(
        tx: flume::Sender<ExternalEvent>,
        proxy: Arc<OnceLock<EventLoopProxy>>,
        grid_patches: Arc<GridPatchStore>,
    ) -> Self {
        Self { tx, proxy, grid_patches }
    }

    /// Send an event and wake the UI loop.
    ///
    /// Channel-first, wake-second ordering (per winit docs).
    /// If the event loop has not started yet the event is buffered in the
    /// channel and will be drained on the first `proxy_wake_up`.
    pub fn send(&self, event: ExternalEvent) -> Result<(), SendError> {
        if let ExternalEvent::GridPatch { node, grid } = event {
            return self.send_grid_patch(node, grid);
        }
        self.tx.send(event).map_err(|e| SendError(e.into_inner()))?;
        if let Some(proxy) = self.proxy.get() {
            proxy.wake_up();
        }
        Ok(())
    }

    /// Async-compatible send for use inside tokio/async-std tasks.
    pub async fn send_async(&self, event: ExternalEvent) -> Result<(), SendError> {
        if let ExternalEvent::GridPatch { node, grid } = event {
            // The queue is unbounded, so the synchronous latest-wins insert
            // never waits for capacity. Keeping this path shared with `send`
            // prevents async producers from bypassing output coalescing.
            return self.send_grid_patch(node, grid);
        }
        self.tx.send_async(event).await.map_err(|e| SendError(e.into_inner()))?;
        if let Some(proxy) = self.proxy.get() {
            proxy.wake_up();
        }
        Ok(())
    }

    fn send_grid_patch(&self, node: NodeRef, grid: Box<CellGrid>) -> Result<(), SendError> {
        if let Some(node_id) = node.get() {
            self.grid_patches.push(node_id, grid, &self.tx)?;
        } else {
            // Preserve the documented stale-reference fallback without
            // retaining an unaddressable grid allocation in the store.
            self.tx
                .send(ExternalEvent::RequestRebuild)
                .map_err(|err| SendError(err.into_inner()))?;
        }
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

    /// Request a new native drawable-surface size. The platform may apply the
    /// request asynchronously; consumers should observe layout through their
    /// ordinary resize callbacks rather than assuming an immediate change.
    pub fn request_surface_size(&self, width: u32, height: u32) -> Result<(), SendError> {
        self.send(ExternalEvent::RequestSurfaceSize { width, height })
    }

    /// Replace a terminal-style grid after its rows or columns changed,
    /// without rebuilding unrelated UI.
    pub fn send_grid_resize_patch(
        &self,
        node: NodeRef,
        grid: Box<CellGrid>,
    ) -> Result<(), SendError> {
        self.send(ExternalEvent::GridResizePatch { node, grid })
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
    fn grid_patches_coalesce_to_the_latest_grid_for_a_node() {
        let (sink, rx) = make_sink();
        let node = NodeRef::new();
        node.set(NodeId { index: 9, generation: 3 });

        sink.send(ExternalEvent::GridPatch {
            node: node.clone(),
            grid: Box::new(CellGrid::new(2, 2)),
        })
        .unwrap();
        sink.send(ExternalEvent::GridPatch { node, grid: Box::new(CellGrid::new(3, 4)) }).unwrap();

        assert!(matches!(rx.try_recv().unwrap(), ExternalEvent::DrainGridPatches));
        assert!(rx.try_recv().is_err(), "one wake marker per pending drain");
        let patches = sink.grid_patches.take();
        assert_eq!(patches.len(), 1);
        let grid = patches.values().next().expect("latest grid");
        assert_eq!((grid.rows(), grid.cols()), (3, 4));
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
