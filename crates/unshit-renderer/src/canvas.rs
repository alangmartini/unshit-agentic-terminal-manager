use std::sync::Arc;

use rustc_hash::FxHashMap;
use unshit_core::element::LayoutRect;
use unshit_core::id::NodeId;

use crate::persistent_buffer::{GpuElementBuffers, PersistentBuffer};

/// Context passed to `CustomPainter::paint()`.
pub struct PaintContext<'a> {
    /// The laid-out rectangle of the canvas element in window coordinates.
    pub rect: LayoutRect,
    /// The clip rectangle [x, y, width, height].
    pub clip_rect: [f32; 4],
    /// Window size in physical pixels.
    pub viewport_size: (f32, f32),
    /// The render target's texture format.
    pub surface_format: wgpu::TextureFormat,
    /// GPU device handle.
    pub device: &'a wgpu::Device,
    /// GPU queue handle.
    pub queue: &'a wgpu::Queue,
    /// Persistent GPU buffers for this canvas element, if registered.
    pub persistent_buffer: Option<&'a GpuElementBuffers>,
}

/// Trait for user-defined GPU rendering into a rectangular region.
///
/// All methods take `&self`; use interior mutability (`Mutex`, `Cell`,
/// `AtomicBool`) for mutable state.
pub trait CustomPainter: Send + Sync + 'static {
    /// Called before the render pass. Create/update GPU resources here.
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        rect: LayoutRect,
    );

    /// Called during the render pass. A scissor rect confines output to the
    /// canvas region. Coordinates are in window space (not relative to canvas).
    fn paint<'pass>(&'pass self, ctx: &PaintContext<'_>, render_pass: &mut wgpu::RenderPass<'pass>);

    /// Called before the render pass when a `PersistentBuffer` is registered
    /// for this painter's node. Override to update CPU-side buffer data before
    /// it is uploaded to the GPU.
    fn update_buffer(&self, _buffer: &mut PersistentBuffer, _rect: LayoutRect) {}

    /// Return `true` for continuous repainting (animations).
    fn needs_repaint(&self) -> bool {
        false
    }
}

/// Batch entry: a canvas to render during the GPU pass.
#[derive(Clone)]
pub struct CanvasCallback {
    pub painter: Arc<dyn CustomPainter>,
    pub rect: LayoutRect,
    pub clip_rect: [f32; 4],
    /// The `NodeId` of the canvas element, used to look up persistent buffers.
    pub node_id: Option<NodeId>,
}

/// Maps element IDs to their `CustomPainter` implementations.
pub struct CanvasRegistry {
    painters: FxHashMap<String, Arc<dyn CustomPainter>>,
    node_ids: FxHashMap<String, NodeId>,
}

impl CanvasRegistry {
    pub fn new() -> Self {
        Self { painters: FxHashMap::default(), node_ids: FxHashMap::default() }
    }

    pub fn register(&mut self, id: impl Into<String>, painter: Arc<dyn CustomPainter>) {
        self.painters.insert(id.into(), painter);
    }

    /// Register a painter and associate it with a specific `NodeId` so that
    /// persistent GPU buffers can be looked up by node during rendering.
    pub fn register_with_node(
        &mut self,
        id: impl Into<String>,
        painter: Arc<dyn CustomPainter>,
        node_id: NodeId,
    ) {
        let key: String = id.into();
        self.painters.insert(key.clone(), painter);
        self.node_ids.insert(key, node_id);
    }

    pub fn unregister(&mut self, id: &str) {
        self.painters.remove(id);
        self.node_ids.remove(id);
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn CustomPainter>> {
        self.painters.get(id)
    }

    /// Return the `NodeId` associated with a painter ID, if any.
    pub fn get_node_id(&self, id: &str) -> Option<NodeId> {
        self.node_ids.get(id).copied()
    }
}

impl Default for CanvasRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn test_node(index: u32) -> NodeId {
        NodeId { index, generation: 0 }
    }

    // Minimal painter for testing (no actual GPU work).
    struct NopPainter;

    impl CustomPainter for NopPainter {
        fn prepare(
            &self,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _format: wgpu::TextureFormat,
            _rect: LayoutRect,
        ) {
        }

        fn paint<'pass>(
            &'pass self,
            _ctx: &PaintContext<'_>,
            _render_pass: &mut wgpu::RenderPass<'pass>,
        ) {
        }
    }

    #[test]
    fn canvas_callback_node_id_none_by_default() {
        // CanvasCallback can carry an optional NodeId; verify default is None.
        let cb = CanvasCallback {
            painter: Arc::new(NopPainter),
            rect: LayoutRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
            clip_rect: [0.0, 0.0, 100.0, 100.0],
            node_id: None,
        };
        assert!(cb.node_id.is_none());
    }

    #[test]
    fn canvas_callback_node_id_some() {
        let node = test_node(42);
        let cb = CanvasCallback {
            painter: Arc::new(NopPainter),
            rect: LayoutRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
            clip_rect: [0.0, 0.0, 100.0, 100.0],
            node_id: Some(node),
        };
        assert_eq!(cb.node_id, Some(node));
    }

    #[test]
    fn update_buffer_default_is_noop() {
        use crate::persistent_buffer::PersistentBuffer;

        let painter = NopPainter;
        let mut buf = PersistentBuffer::new(4, 4, 1, 1);
        let original_gen = buf.generation;
        let original_quad_count = buf.quad_count();
        let original_glyph_count = buf.glyph_count();
        let original_needs_upload = buf.needs_full_upload;

        // Default implementation must not modify the buffer.
        painter.update_buffer(&mut buf, LayoutRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 });

        assert_eq!(buf.generation, original_gen);
        assert_eq!(buf.quad_count(), original_quad_count);
        assert_eq!(buf.glyph_count(), original_glyph_count);
        assert_eq!(buf.needs_full_upload, original_needs_upload);
    }

    #[test]
    fn canvas_registry_register_and_get() {
        let mut registry = CanvasRegistry::new();
        let painter: Arc<dyn CustomPainter> = Arc::new(NopPainter);
        registry.register("my-canvas", Arc::clone(&painter));

        assert!(registry.get("my-canvas").is_some());
        assert!(registry.get("other").is_none());
    }

    #[test]
    fn canvas_registry_register_with_node_and_get_node_id() {
        let mut registry = CanvasRegistry::new();
        let node = test_node(7);
        let painter: Arc<dyn CustomPainter> = Arc::new(NopPainter);
        registry.register_with_node("my-canvas", Arc::clone(&painter), node);

        assert!(registry.get("my-canvas").is_some());
        assert_eq!(registry.get_node_id("my-canvas"), Some(node));
        assert_eq!(registry.get_node_id("other"), None);
    }

    #[test]
    fn canvas_registry_unregister_removes_node_id() {
        let mut registry = CanvasRegistry::new();
        let node = test_node(3);
        registry.register_with_node("my-canvas", Arc::new(NopPainter), node);

        registry.unregister("my-canvas");

        assert!(registry.get("my-canvas").is_none());
        assert_eq!(registry.get_node_id("my-canvas"), None);
    }

    #[test]
    fn canvas_registry_plain_register_has_no_node_id() {
        let mut registry = CanvasRegistry::new();
        registry.register("my-canvas", Arc::new(NopPainter));

        assert!(registry.get("my-canvas").is_some());
        assert_eq!(registry.get_node_id("my-canvas"), None);
    }

    /// Verify that a painter can override `update_buffer` and mutate the buffer.
    #[test]
    fn update_buffer_override_mutates() {
        use crate::persistent_buffer::PersistentBuffer;

        struct CountingPainter {
            calls: Mutex<u32>,
        }

        impl CustomPainter for CountingPainter {
            fn prepare(
                &self,
                _device: &wgpu::Device,
                _queue: &wgpu::Queue,
                _format: wgpu::TextureFormat,
                _rect: LayoutRect,
            ) {
            }

            fn paint<'pass>(
                &'pass self,
                _ctx: &PaintContext<'_>,
                _render_pass: &mut wgpu::RenderPass<'pass>,
            ) {
            }

            fn update_buffer(&self, buffer: &mut PersistentBuffer, _rect: LayoutRect) {
                *self.calls.lock().unwrap() += 1;
                // Mark buffer dirty so caller knows we touched it.
                buffer.needs_full_upload = true;
            }
        }

        let painter = CountingPainter { calls: Mutex::new(0) };
        let mut buf = PersistentBuffer::new(2, 2, 1, 1);
        buf.needs_full_upload = false;

        painter.update_buffer(&mut buf, LayoutRect { x: 0.0, y: 0.0, width: 50.0, height: 50.0 });

        assert_eq!(*painter.calls.lock().unwrap(), 1);
        assert!(buf.needs_full_upload);
    }
}
