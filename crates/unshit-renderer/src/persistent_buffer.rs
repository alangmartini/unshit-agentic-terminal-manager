//! Persistent GPU buffer management for high-throughput rendering.
//!
//! Elements that opt into persistent buffer rendering maintain GPU-side
//! instance buffers that survive across frames. Instead of rebuilding all
//! batch data each frame, the renderer patches only the damage regions,
//! yielding significant speedups for content that changes rapidly (e.g.
//! terminal grids, live editors, data tables).
//!
//! # Design
//!
//! This is an optimization layer, not a parallel rendering path. Elements
//! declare support via `persistent_buffer: true`, and the batch builder
//! checks this flag. If an element has a persistent buffer and no structural
//! changes, the batch builder skips it and references the existing buffer
//! data. Only damage regions get patched via partial buffer writes.

use bytemuck::Pod;
use rustc_hash::FxHashMap;
use std::mem;
use unshit_core::damage::{DamageRegion, DamageTracker};
use unshit_core::id::NodeId;

use crate::pipeline::quad::QuadInstance;
use crate::pipeline::text::GlyphInstance;

/// Holds persistent CPU-side instance data for a single element, plus
/// the metadata needed for partial GPU uploads.
#[derive(Debug)]
pub struct PersistentBuffer {
    /// Background quad instances for this element.
    pub quads: Vec<QuadInstance>,
    /// Character/glyph instances for this element.
    pub glyphs: Vec<GlyphInstance>,
    /// Grid dimensions used to compute instance offsets.
    pub rows: u32,
    pub cols: u32,
    /// Number of quad instances per cell (typically 1 for bg).
    pub quads_per_cell: u32,
    /// Number of glyph instances per cell (typically 1 for fg character).
    pub glyphs_per_cell: u32,
    /// Generation counter, incremented on full rebuild. Used to detect
    /// when the GPU buffer needs re-uploading in its entirety.
    pub generation: u64,
    /// Whether this buffer needs a full re-upload (after resize etc).
    pub needs_full_upload: bool,
}

impl PersistentBuffer {
    /// Create a new persistent buffer for an element with given grid dimensions.
    pub fn new(rows: u32, cols: u32, quads_per_cell: u32, glyphs_per_cell: u32) -> Self {
        let total_cells = (rows * cols) as usize;
        Self {
            quads: vec![QuadInstance::zeroed(); total_cells * quads_per_cell as usize],
            glyphs: vec![GlyphInstance::zeroed(); total_cells * glyphs_per_cell as usize],
            rows,
            cols,
            quads_per_cell,
            glyphs_per_cell,
            generation: 0,
            needs_full_upload: true,
        }
    }

    /// Total number of quad instances in this buffer.
    pub fn quad_count(&self) -> usize {
        self.quads.len()
    }

    /// Total number of glyph instances in this buffer.
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Returns the byte offset and byte length for a quad slice covering
    /// the given damage region.
    pub fn quad_byte_range(&self, region: &DamageRegion) -> (u64, u64) {
        let stride = mem::size_of::<QuadInstance>() as u64;
        let cols = self.cols as u64;
        let qpc = self.quads_per_cell as u64;

        let start_cell = region.row_start as u64 * cols + region.col_start as u64;
        let start_byte = start_cell * qpc * stride;

        // For contiguous rows with full column span, compute total cells.
        // For partial columns within rows, compute row-by-row.
        let row_count = region.row_end.saturating_sub(region.row_start) as u64;
        let col_count = region.col_end.saturating_sub(region.col_start) as u64;

        if region.col_start == 0 && region.col_end == self.cols {
            // Full-width damage: contiguous in memory
            let total_cells = row_count * cols;
            let byte_len = total_cells * qpc * stride;
            (start_byte, byte_len)
        } else {
            // Partial columns: each row is separate, but we report the range
            // covering all affected rows for simplicity. The caller should
            // upload row-by-row for non-contiguous sub-row damage.
            let last_row_start = (region.row_end as u64 - 1) * cols + region.col_start as u64;
            let last_row_end = last_row_start + col_count;
            let end_byte = last_row_end * qpc * stride;
            (start_byte, end_byte - start_byte)
        }
    }

    /// Returns the byte offset and byte length for a glyph slice covering
    /// the given damage region.
    pub fn glyph_byte_range(&self, region: &DamageRegion) -> (u64, u64) {
        let stride = mem::size_of::<GlyphInstance>() as u64;
        let cols = self.cols as u64;
        let gpc = self.glyphs_per_cell as u64;

        let start_cell = region.row_start as u64 * cols + region.col_start as u64;
        let start_byte = start_cell * gpc * stride;

        let row_count = region.row_end.saturating_sub(region.row_start) as u64;
        let col_count = region.col_end.saturating_sub(region.col_start) as u64;

        if region.col_start == 0 && region.col_end == self.cols {
            let total_cells = row_count * cols;
            let byte_len = total_cells * gpc * stride;
            (start_byte, byte_len)
        } else {
            let last_row_start = (region.row_end as u64 - 1) * cols + region.col_start as u64;
            let last_row_end = last_row_start + col_count;
            let end_byte = last_row_end * gpc * stride;
            (start_byte, end_byte - start_byte)
        }
    }

    /// Update quad instances for a specific damage region.
    /// `new_quads` must contain exactly `region.cell_count() * quads_per_cell` instances,
    /// laid out row-by-row within the region.
    pub fn patch_quads(&mut self, region: &DamageRegion, new_quads: &[QuadInstance]) {
        let qpc = self.quads_per_cell as usize;
        let cols = self.cols as usize;
        let expected = region.cell_count() as usize * qpc;
        assert_eq!(
            new_quads.len(),
            expected,
            "patch_quads: expected {} instances, got {}",
            expected,
            new_quads.len()
        );

        let mut src_offset = 0;
        for row in region.row_start..region.row_end {
            for col in region.col_start..region.col_end {
                let cell_idx = row as usize * cols + col as usize;
                let dst_start = cell_idx * qpc;
                for q in 0..qpc {
                    self.quads[dst_start + q] = new_quads[src_offset + q];
                }
                src_offset += qpc;
            }
        }
    }

    /// Update glyph instances for a specific damage region.
    /// `new_glyphs` must contain exactly `region.cell_count() * glyphs_per_cell` instances.
    pub fn patch_glyphs(&mut self, region: &DamageRegion, new_glyphs: &[GlyphInstance]) {
        let gpc = self.glyphs_per_cell as usize;
        let cols = self.cols as usize;
        let expected = region.cell_count() as usize * gpc;
        assert_eq!(
            new_glyphs.len(),
            expected,
            "patch_glyphs: expected {} instances, got {}",
            expected,
            new_glyphs.len()
        );

        let mut src_offset = 0;
        for row in region.row_start..region.row_end {
            for col in region.col_start..region.col_end {
                let cell_idx = row as usize * cols + col as usize;
                let dst_start = cell_idx * gpc;
                for g in 0..gpc {
                    self.glyphs[dst_start + g] = new_glyphs[src_offset + g];
                }
                src_offset += gpc;
            }
        }
    }

    /// Resize the buffer for new grid dimensions. Existing data is discarded,
    /// and the buffer is marked for full re-upload.
    pub fn resize(&mut self, new_rows: u32, new_cols: u32) {
        self.rows = new_rows;
        self.cols = new_cols;
        let total_cells = (new_rows * new_cols) as usize;
        self.quads.clear();
        self.quads.resize(total_cells * self.quads_per_cell as usize, QuadInstance::zeroed());
        self.glyphs.clear();
        self.glyphs.resize(total_cells * self.glyphs_per_cell as usize, GlyphInstance::zeroed());
        self.generation += 1;
        self.needs_full_upload = true;
    }
}

/// Manages persistent buffers for all elements that opt in.
pub struct PersistentBufferManager {
    buffers: FxHashMap<NodeId, PersistentBuffer>,
    /// Pending partial upload commands generated during damage processing.
    pending_uploads: Vec<PartialUpload>,
}

/// A single partial upload command targeting either quads or glyphs.
#[derive(Debug)]
pub struct PartialUpload {
    pub node: NodeId,
    pub kind: UploadKind,
    pub byte_offset: u64,
    pub byte_length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadKind {
    Quads,
    Glyphs,
}

impl Default for PersistentBufferManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistentBufferManager {
    pub fn new() -> Self {
        Self { buffers: FxHashMap::default(), pending_uploads: Vec::new() }
    }

    /// Register a persistent buffer for an element.
    pub fn register(
        &mut self,
        node: NodeId,
        rows: u32,
        cols: u32,
        quads_per_cell: u32,
        glyphs_per_cell: u32,
    ) {
        self.buffers
            .insert(node, PersistentBuffer::new(rows, cols, quads_per_cell, glyphs_per_cell));
    }

    /// Remove the persistent buffer for an element.
    pub fn unregister(&mut self, node: NodeId) {
        self.buffers.remove(&node);
    }

    /// Returns true if this element has a persistent buffer.
    pub fn has_buffer(&self, node: NodeId) -> bool {
        self.buffers.contains_key(&node)
    }

    /// Get immutable access to a buffer.
    pub fn get(&self, node: NodeId) -> Option<&PersistentBuffer> {
        self.buffers.get(&node)
    }

    /// Get mutable access to a buffer.
    pub fn get_mut(&mut self, node: NodeId) -> Option<&mut PersistentBuffer> {
        self.buffers.get_mut(&node)
    }

    /// Process damage from the tracker and generate partial upload commands.
    /// This is the frame coalescing point: all damage accumulated between
    /// frames is processed in a single pass.
    pub fn process_damage(&mut self, damage_tracker: &mut DamageTracker) {
        self.pending_uploads.clear();

        let nodes: Vec<NodeId> = damage_tracker.tracked_nodes();
        for node in nodes {
            let Some(damage_state) = damage_tracker.get_mut(node) else {
                continue;
            };

            let Some(buffer) = self.buffers.get_mut(&node) else {
                continue;
            };

            // Handle structural changes (resize)
            if damage_state.structural_change {
                buffer.resize(damage_state.rows, damage_state.cols);
                damage_state.clear_structural_change();
                // Full upload will happen naturally since needs_full_upload is set
                continue;
            }

            let regions = damage_state.take_coalesced_damage();
            if regions.is_empty() {
                continue;
            }

            for region in &regions {
                let (quad_offset, quad_len) = buffer.quad_byte_range(region);
                if quad_len > 0 {
                    self.pending_uploads.push(PartialUpload {
                        node,
                        kind: UploadKind::Quads,
                        byte_offset: quad_offset,
                        byte_length: quad_len,
                    });
                }

                let (glyph_offset, glyph_len) = buffer.glyph_byte_range(region);
                if glyph_len > 0 {
                    self.pending_uploads.push(PartialUpload {
                        node,
                        kind: UploadKind::Glyphs,
                        byte_offset: glyph_offset,
                        byte_length: glyph_len,
                    });
                }
            }
        }
    }

    /// Upload pending changes to GPU buffers.
    /// Uses `queue.write_buffer_with` for partial updates when possible.
    pub fn upload_to_gpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_buffers: &mut GpuPersistentBuffers,
    ) {
        // First, handle any buffers that need full re-upload
        for (&node, buffer) in &mut self.buffers {
            if buffer.needs_full_upload {
                gpu_buffers.ensure_capacity(node, buffer, device);
                if let Some(gpu_buf) = gpu_buffers.get(node) {
                    if !buffer.quads.is_empty() {
                        queue.write_buffer(
                            &gpu_buf.quad_buffer,
                            0,
                            bytemuck::cast_slice(&buffer.quads),
                        );
                    }
                    if !buffer.glyphs.is_empty() {
                        queue.write_buffer(
                            &gpu_buf.glyph_buffer,
                            0,
                            bytemuck::cast_slice(&buffer.glyphs),
                        );
                    }
                }
                buffer.needs_full_upload = false;
            }
        }

        // Then process partial uploads
        for upload in &self.pending_uploads {
            let Some(buffer) = self.buffers.get(&upload.node) else {
                continue;
            };
            let Some(gpu_buf) = gpu_buffers.get(upload.node) else {
                continue;
            };

            match upload.kind {
                UploadKind::Quads => {
                    let start = upload.byte_offset as usize;
                    let end = start + upload.byte_length as usize;
                    let slice =
                        &bytemuck::cast_slice::<QuadInstance, u8>(&buffer.quads)[start..end];
                    // Use write_buffer_with for partial updates when possible
                    if let Some(mut view) = queue.write_buffer_with(
                        &gpu_buf.quad_buffer,
                        upload.byte_offset,
                        wgpu::BufferSize::new(upload.byte_length).unwrap(),
                    ) {
                        view.copy_from_slice(slice);
                    }
                }
                UploadKind::Glyphs => {
                    let start = upload.byte_offset as usize;
                    let end = start + upload.byte_length as usize;
                    let slice =
                        &bytemuck::cast_slice::<GlyphInstance, u8>(&buffer.glyphs)[start..end];
                    if let Some(mut view) = queue.write_buffer_with(
                        &gpu_buf.glyph_buffer,
                        upload.byte_offset,
                        wgpu::BufferSize::new(upload.byte_length).unwrap(),
                    ) {
                        view.copy_from_slice(slice);
                    }
                }
            }
        }

        self.pending_uploads.clear();
    }

    /// Take the pending upload list (for inspection/testing).
    pub fn take_pending_uploads(&mut self) -> Vec<PartialUpload> {
        mem::take(&mut self.pending_uploads)
    }

    /// Returns all registered node IDs.
    pub fn registered_nodes(&self) -> Vec<NodeId> {
        self.buffers.keys().copied().collect()
    }
}

/// GPU-side buffer pair for a single persistent buffer element.
pub struct GpuElementBuffers {
    pub quad_buffer: wgpu::Buffer,
    pub glyph_buffer: wgpu::Buffer,
    pub quad_capacity: usize,
    pub glyph_capacity: usize,
}

/// Manages GPU buffer allocations for all persistent buffer elements.
pub struct GpuPersistentBuffers {
    buffers: FxHashMap<NodeId, GpuElementBuffers>,
}

impl Default for GpuPersistentBuffers {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuPersistentBuffers {
    pub fn new() -> Self {
        Self { buffers: FxHashMap::default() }
    }

    pub fn get(&self, node: NodeId) -> Option<&GpuElementBuffers> {
        self.buffers.get(&node)
    }

    /// Ensure GPU buffers exist and are large enough for the given persistent buffer.
    pub fn ensure_capacity(
        &mut self,
        node: NodeId,
        buffer: &PersistentBuffer,
        device: &wgpu::Device,
    ) {
        let quad_needed = buffer.quad_count();
        let glyph_needed = buffer.glyph_count();

        let needs_recreate = match self.buffers.get(&node) {
            None => true,
            Some(gpu) => gpu.quad_capacity < quad_needed || gpu.glyph_capacity < glyph_needed,
        };

        if needs_recreate {
            let quad_cap = quad_needed.next_power_of_two().max(256);
            let glyph_cap = glyph_needed.next_power_of_two().max(256);

            let quad_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("persistent quad buffer"),
                size: (quad_cap * mem::size_of::<QuadInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let glyph_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("persistent glyph buffer"),
                size: (glyph_cap * mem::size_of::<GlyphInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            self.buffers.insert(
                node,
                GpuElementBuffers {
                    quad_buffer,
                    glyph_buffer,
                    quad_capacity: quad_cap,
                    glyph_capacity: glyph_cap,
                },
            );
        }
    }

    /// Remove GPU buffers for an element.
    pub fn remove(&mut self, node: NodeId) {
        self.buffers.remove(&node);
    }

    /// Returns true if this element has GPU buffers allocated.
    pub fn has_buffers(&self, node: NodeId) -> bool {
        self.buffers.contains_key(&node)
    }
}

/// Trait for zero-initializing Pod types (for buffer initialization).
trait Zeroed: Pod {
    fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

impl Zeroed for QuadInstance {}
impl Zeroed for GlyphInstance {}

#[cfg(test)]
mod tests {
    use super::*;
    use unshit_core::damage::DamageRegion;

    fn test_node(index: u32) -> NodeId {
        NodeId { index, generation: 0 }
    }

    #[test]
    fn persistent_buffer_new() {
        let buf = PersistentBuffer::new(24, 80, 1, 1);
        assert_eq!(buf.quad_count(), 24 * 80);
        assert_eq!(buf.glyph_count(), 24 * 80);
        assert_eq!(buf.rows, 24);
        assert_eq!(buf.cols, 80);
        assert!(buf.needs_full_upload);
    }

    #[test]
    fn persistent_buffer_resize() {
        let mut buf = PersistentBuffer::new(24, 80, 1, 1);
        buf.needs_full_upload = false;
        let old_gen = buf.generation;

        buf.resize(30, 100);

        assert_eq!(buf.rows, 30);
        assert_eq!(buf.cols, 100);
        assert_eq!(buf.quad_count(), 30 * 100);
        assert_eq!(buf.glyph_count(), 30 * 100);
        assert!(buf.needs_full_upload);
        assert_eq!(buf.generation, old_gen + 1);
    }

    #[test]
    fn persistent_buffer_patch_quads() {
        let mut buf = PersistentBuffer::new(4, 4, 1, 1);

        // Patch a 2x2 region at (1,1) to (3,3)
        let region = DamageRegion::new(1, 3, 1, 3);
        let mut new_quads = vec![QuadInstance::zeroed(); 4];
        new_quads[0].color = [1.0, 0.0, 0.0, 1.0]; // (1,1)
        new_quads[1].color = [0.0, 1.0, 0.0, 1.0]; // (1,2)
        new_quads[2].color = [0.0, 0.0, 1.0, 1.0]; // (2,1)
        new_quads[3].color = [1.0, 1.0, 0.0, 1.0]; // (2,2)

        buf.patch_quads(&region, &new_quads);

        // Verify cell (1,1) = index 5 in 4-col grid
        assert_eq!(buf.quads[5].color, [1.0, 0.0, 0.0, 1.0]);
        // Verify cell (1,2) = index 6
        assert_eq!(buf.quads[6].color, [0.0, 1.0, 0.0, 1.0]);
        // Verify cell (2,1) = index 9
        assert_eq!(buf.quads[9].color, [0.0, 0.0, 1.0, 1.0]);
        // Verify cell (2,2) = index 10
        assert_eq!(buf.quads[10].color, [1.0, 1.0, 0.0, 1.0]);

        // Unaffected cells should remain zero
        assert_eq!(buf.quads[0].color, [0.0; 4]);
    }

    #[test]
    fn persistent_buffer_byte_range_full_width() {
        let buf = PersistentBuffer::new(24, 80, 1, 1);
        let region = DamageRegion::new(5, 10, 0, 80);

        let (offset, len) = buf.quad_byte_range(&region);
        let stride = mem::size_of::<QuadInstance>() as u64;

        assert_eq!(offset, 5 * 80 * stride);
        assert_eq!(len, 5 * 80 * stride);
    }

    #[test]
    fn manager_register_unregister() {
        let mut mgr = PersistentBufferManager::new();
        let node = test_node(0);

        mgr.register(node, 24, 80, 1, 1);
        assert!(mgr.has_buffer(node));

        mgr.unregister(node);
        assert!(!mgr.has_buffer(node));
    }

    #[test]
    fn manager_process_damage_generates_uploads() {
        let mut mgr = PersistentBufferManager::new();
        let mut tracker = DamageTracker::new();
        let node = test_node(0);

        mgr.register(node, 24, 80, 1, 1);
        tracker.register(node, 24, 80);

        // Record some damage
        tracker.record_damage(node, DamageRegion::new(0, 3, 0, 80));

        // Mark full upload as done
        mgr.get_mut(node).unwrap().needs_full_upload = false;

        mgr.process_damage(&mut tracker);

        let uploads = mgr.take_pending_uploads();
        // Should have 2 uploads: one for quads, one for glyphs
        assert_eq!(uploads.len(), 2);
        assert!(uploads.iter().any(|u| u.kind == UploadKind::Quads));
        assert!(uploads.iter().any(|u| u.kind == UploadKind::Glyphs));
    }
}
