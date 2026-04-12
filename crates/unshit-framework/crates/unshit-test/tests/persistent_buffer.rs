//! Tests for the persistent buffer rendering optimization (issue #107).
//!
//! These tests verify that persistent GPU buffers survive across frames,
//! damage regions correctly track content changes, partial buffer updates
//! write the correct slice, and that frame coalescing works properly.

use std::mem;
use unshit_core::damage::{DamageRegion, DamageTracker, DirtyRows, ElementDamageState};
use unshit_core::dirty::DirtyFlags;
use unshit_core::element::{Element, ElementDef, Tag};
use unshit_core::id::NodeId;
use unshit_core::tree::NodeArena;
use unshit_renderer::persistent_buffer::{PersistentBuffer, PersistentBufferManager, UploadKind};
use unshit_renderer::pipeline::quad::QuadInstance;
use unshit_renderer::pipeline::text::GlyphInstance;

fn test_node(index: u32) -> NodeId {
    NodeId { index, generation: 0 }
}

// ---------------------------------------------------------------------------
// 1. Persistent buffer survives across frames without re-allocation
// ---------------------------------------------------------------------------

#[test]
fn persistent_buffer_survives_across_frames() {
    // A persistent buffer, once created, should keep its data intact across
    // simulated frames. The generation counter should not change unless a
    // resize occurs.
    let mut mgr = PersistentBufferManager::new();
    let node = test_node(0);

    mgr.register(node, 24, 80, 1, 1);

    // Simulate patching some content into the buffer
    let buf = mgr.get_mut(node).unwrap();
    buf.needs_full_upload = false;
    let initial_gen = buf.generation;

    // Write recognizable data into cell (0,0)
    buf.quads[0].color = [1.0, 0.5, 0.25, 1.0];
    buf.glyphs[0].color = [0.0, 1.0, 0.0, 1.0];

    // Simulate several frames passing with no damage
    let mut tracker = DamageTracker::new();
    tracker.register(node, 24, 80);

    for _ in 0..10 {
        mgr.process_damage(&mut tracker);
        // Buffer should still exist with same data
        let buf = mgr.get(node).unwrap();
        assert_eq!(buf.generation, initial_gen);
        assert_eq!(buf.quads[0].color, [1.0, 0.5, 0.25, 1.0]);
        assert_eq!(buf.glyphs[0].color, [0.0, 1.0, 0.0, 1.0]);
        assert!(!buf.needs_full_upload);
    }
}

// ---------------------------------------------------------------------------
// 2. Damage region correctly identifies changed cells after partial update
// ---------------------------------------------------------------------------

#[test]
fn damage_region_identifies_changed_cells() {
    // After marking specific rows as dirty, the damage regions should
    // correctly reflect those rows and only those rows.
    let mut state = ElementDamageState::new(24, 80);

    // Simulate a terminal: lines 5-7 changed (e.g. a command output)
    state.record_damage(DamageRegion::new(5, 8, 0, 80));

    assert!(state.dirty_rows.is_row_dirty(5));
    assert!(state.dirty_rows.is_row_dirty(6));
    assert!(state.dirty_rows.is_row_dirty(7));
    assert!(!state.dirty_rows.is_row_dirty(4));
    assert!(!state.dirty_rows.is_row_dirty(8));

    let coalesced = state.take_coalesced_damage();
    assert_eq!(coalesced.len(), 1);
    assert_eq!(coalesced[0], DamageRegion::new(5, 8, 0, 80));

    // After taking damage, rows should be clean
    assert!(!state.dirty_rows.any_dirty());
}

// ---------------------------------------------------------------------------
// 3. Partial buffer update writes correct slice (byte offset + length)
// ---------------------------------------------------------------------------

#[test]
fn partial_buffer_update_correct_byte_range() {
    // For a 24x80 grid with 1 quad per cell, damaging rows 10-15
    // should produce a byte offset starting at row 10 and length
    // covering exactly 5 rows worth of QuadInstances.
    let buf = PersistentBuffer::new(24, 80, 1, 1);
    let region = DamageRegion::new(10, 15, 0, 80);

    let (quad_offset, quad_len) = buf.quad_byte_range(&region);
    let quad_stride = mem::size_of::<QuadInstance>() as u64;

    // Row 10 starts at cell index 10*80 = 800
    let expected_offset = 800 * quad_stride;
    // 5 full rows = 5*80 = 400 cells
    let expected_len = 400 * quad_stride;

    assert_eq!(quad_offset, expected_offset);
    assert_eq!(quad_len, expected_len);

    // Same for glyphs
    let (glyph_offset, glyph_len) = buf.glyph_byte_range(&region);
    let glyph_stride = mem::size_of::<GlyphInstance>() as u64;

    assert_eq!(glyph_offset, 800 * glyph_stride);
    assert_eq!(glyph_len, 400 * glyph_stride);
}

// ---------------------------------------------------------------------------
// 4. Full-content update works correctly when all cells change
// ---------------------------------------------------------------------------

#[test]
fn full_content_update_all_cells() {
    let mut buf = PersistentBuffer::new(4, 4, 1, 1);

    // Full grid damage
    let region = DamageRegion::full(4, 4);

    // Create new quads for all 16 cells
    let new_quads: Vec<QuadInstance> = (0..16)
        .map(|i| {
            let mut q = bytemuck::Zeroable::zeroed();
            let q: &mut QuadInstance = &mut q;
            q.color = [i as f32, 0.0, 0.0, 1.0];
            *q
        })
        .collect();

    buf.patch_quads(&region, &new_quads);

    // Verify every cell got the correct data
    for i in 0..16 {
        assert_eq!(
            buf.quads[i].color[0], i as f32,
            "cell {} should have color[0] = {}, got {}",
            i, i as f32, buf.quads[i].color[0]
        );
    }

    // Full damage byte range should cover the entire buffer
    let (offset, len) = buf.quad_byte_range(&region);
    assert_eq!(offset, 0);
    assert_eq!(len, 16 * mem::size_of::<QuadInstance>() as u64);
}

// ---------------------------------------------------------------------------
// 5. Frame coalescing merges multiple updates within single frame interval
// ---------------------------------------------------------------------------

#[test]
fn frame_coalescing_merges_updates() {
    // Multiple damage events arriving between frames should be coalesced
    // into a minimal set of upload operations.
    let mut mgr = PersistentBufferManager::new();
    let mut tracker = DamageTracker::new();
    let node = test_node(0);

    mgr.register(node, 24, 80, 1, 1);
    tracker.register(node, 24, 80);

    // Mark the initial full upload as done
    mgr.get_mut(node).unwrap().needs_full_upload = false;

    // Simulate rapid updates between frames: rows 0-2, then rows 1-4
    tracker.record_damage(node, DamageRegion::new(0, 3, 0, 80));
    tracker.record_damage(node, DamageRegion::new(1, 5, 0, 80));

    // Process damage (frame coalescing point)
    mgr.process_damage(&mut tracker);

    let uploads = mgr.take_pending_uploads();
    // The two overlapping regions should coalesce, so we get at most
    // 2 uploads (1 quad + 1 glyph) for the merged region
    assert_eq!(uploads.len(), 2, "expected 2 uploads (quad + glyph), got {}", uploads.len());

    // Verify the merged region covers rows 0-5
    let quad_upload = uploads.iter().find(|u| u.kind == UploadKind::Quads).unwrap();
    let quad_stride = mem::size_of::<QuadInstance>() as u64;
    // Row 0, col 0 = offset 0
    assert_eq!(quad_upload.byte_offset, 0);
    // 5 rows * 80 cols = 400 cells
    assert_eq!(quad_upload.byte_length, 400 * quad_stride);
}

// ---------------------------------------------------------------------------
// 6. Buffer resize handles dimension changes without corruption
// ---------------------------------------------------------------------------

#[test]
fn buffer_resize_handles_dimension_change() {
    let mut buf = PersistentBuffer::new(24, 80, 1, 1);

    // Write recognizable data
    buf.quads[0].color = [1.0, 0.0, 0.0, 1.0];
    buf.needs_full_upload = false;
    let old_gen = buf.generation;

    // Resize to larger dimensions
    buf.resize(30, 120);

    assert_eq!(buf.rows, 30);
    assert_eq!(buf.cols, 120);
    assert_eq!(buf.quad_count(), 30 * 120);
    assert_eq!(buf.glyph_count(), 30 * 120);
    // Generation should increment
    assert_eq!(buf.generation, old_gen + 1);
    // Buffer should need full re-upload
    assert!(buf.needs_full_upload);

    // Old data is gone (buffer was re-allocated)
    assert_eq!(buf.quads[0].color, [0.0; 4]);

    // Resize to smaller
    buf.resize(10, 40);
    assert_eq!(buf.quad_count(), 10 * 40);
    assert_eq!(buf.generation, old_gen + 2);
}

// ---------------------------------------------------------------------------
// 7. No visual artifacts at damage region boundaries
// ---------------------------------------------------------------------------

#[test]
fn no_artifacts_at_damage_boundaries() {
    // When patching adjacent but separate damage regions, the boundary
    // cells should have correct values with no overlap or gap.
    let mut buf = PersistentBuffer::new(10, 10, 1, 1);

    // Patch rows 0-5 with red
    let region_a = DamageRegion::new(0, 5, 0, 10);
    let red_quads: Vec<QuadInstance> = (0..50)
        .map(|_| {
            let mut q: QuadInstance = bytemuck::Zeroable::zeroed();
            q.color = [1.0, 0.0, 0.0, 1.0];
            q
        })
        .collect();
    buf.patch_quads(&region_a, &red_quads);

    // Patch rows 5-10 with blue
    let region_b = DamageRegion::new(5, 10, 0, 10);
    let blue_quads: Vec<QuadInstance> = (0..50)
        .map(|_| {
            let mut q: QuadInstance = bytemuck::Zeroable::zeroed();
            q.color = [0.0, 0.0, 1.0, 1.0];
            q
        })
        .collect();
    buf.patch_quads(&region_b, &blue_quads);

    // Check the boundary: row 4 should be red, row 5 should be blue
    let row4_cell0 = 4 * 10; // cell index for (4, 0)
    let row5_cell0 = 5 * 10; // cell index for (5, 0)

    assert_eq!(buf.quads[row4_cell0].color, [1.0, 0.0, 0.0, 1.0], "row 4 should be red");
    assert_eq!(buf.quads[row5_cell0].color, [0.0, 0.0, 1.0, 1.0], "row 5 should be blue");

    // Every cell should be filled (no unpatched gaps)
    for i in 0..100 {
        let c = buf.quads[i].color;
        assert!(
            c == [1.0, 0.0, 0.0, 1.0] || c == [0.0, 0.0, 1.0, 1.0],
            "cell {} has unexpected color {:?}",
            i,
            c
        );
    }
}

// ---------------------------------------------------------------------------
// 8. Buffer freed when owning element removed from tree
// ---------------------------------------------------------------------------

#[test]
fn buffer_freed_when_element_removed() {
    let mut mgr = PersistentBufferManager::new();
    let mut tracker = DamageTracker::new();

    let node = test_node(42);
    mgr.register(node, 24, 80, 1, 1);
    tracker.register(node, 24, 80);
    assert!(mgr.has_buffer(node));
    assert!(tracker.is_tracked(node));

    // Simulate element removal: unregister from both systems
    mgr.unregister(node);
    tracker.unregister(node);

    assert!(!mgr.has_buffer(node));
    assert!(!tracker.is_tracked(node));

    // Re-registering should create fresh state
    mgr.register(node, 10, 40, 1, 1);
    let buf = mgr.get(node).unwrap();
    assert_eq!(buf.rows, 10);
    assert_eq!(buf.cols, 40);
    assert!(buf.needs_full_upload);
}

// ---------------------------------------------------------------------------
// 9. Mixing persistent-buffer elements and normal elements
// ---------------------------------------------------------------------------

#[test]
fn mixing_persistent_and_normal_elements() {
    // Build a tree with some persistent-buffer elements and some normal ones.
    // Verify the persistent buffer flag is correctly set via ElementDef and
    // that the manager tracks only opt-in elements.
    let mut arena = NodeArena::new();

    // Create normal element
    let normal = arena.alloc(Element::new(Tag::Div));
    // Create persistent-buffer element
    let mut pb_elem = Element::new(Tag::Div);
    pb_elem.persistent_buffer = true;
    let persistent = arena.alloc(pb_elem);

    // Verify flags
    assert!(!arena.get(normal).unwrap().persistent_buffer);
    assert!(arena.get(persistent).unwrap().persistent_buffer);

    // Manager should only track the opt-in element
    let mut mgr = PersistentBufferManager::new();
    let mut tracker = DamageTracker::new();

    // Only register the persistent one
    mgr.register(persistent, 24, 80, 1, 1);
    tracker.register(persistent, 24, 80);

    assert!(!mgr.has_buffer(normal));
    assert!(mgr.has_buffer(persistent));

    // Damage to the persistent element generates uploads
    tracker.record_damage(persistent, DamageRegion::new(0, 1, 0, 80));
    mgr.get_mut(persistent).unwrap().needs_full_upload = false;
    mgr.process_damage(&mut tracker);

    let uploads = mgr.take_pending_uploads();
    assert_eq!(uploads.len(), 2); // quad + glyph for the persistent element
    assert!(uploads.iter().all(|u| u.node == persistent));
}

// ---------------------------------------------------------------------------
// Additional tests for edge cases
// ---------------------------------------------------------------------------

#[test]
fn dirty_rows_bitvec_large_grid() {
    // Verify bitvec works correctly with grids larger than 64 rows
    let mut dr = DirtyRows::new(200);
    dr.mark_row(0);
    dr.mark_row(63);
    dr.mark_row(64); // boundary between first and second u64
    dr.mark_row(127);
    dr.mark_row(128);
    dr.mark_row(199);

    assert!(dr.is_row_dirty(0));
    assert!(dr.is_row_dirty(63));
    assert!(dr.is_row_dirty(64));
    assert!(dr.is_row_dirty(127));
    assert!(dr.is_row_dirty(128));
    assert!(dr.is_row_dirty(199));

    assert!(!dr.is_row_dirty(1));
    assert!(!dr.is_row_dirty(65));
    assert!(!dr.is_row_dirty(129));
}

#[test]
fn damage_tracker_multiple_elements() {
    // Multiple elements tracked simultaneously should be independent
    let mut tracker = DamageTracker::new();
    let a = test_node(0);
    let b = test_node(1);

    tracker.register(a, 24, 80);
    tracker.register(b, 10, 40);

    tracker.record_damage(a, DamageRegion::new(0, 5, 0, 80));
    tracker.record_damage(b, DamageRegion::new(2, 4, 0, 40));

    let a_damage = tracker.take_coalesced_damage(a);
    assert_eq!(a_damage.len(), 1);
    assert_eq!(a_damage[0], DamageRegion::new(0, 5, 0, 80));

    let b_damage = tracker.take_coalesced_damage(b);
    assert_eq!(b_damage.len(), 1);
    assert_eq!(b_damage[0], DamageRegion::new(2, 4, 0, 40));
}

#[test]
fn content_dirty_flag_exists() {
    // Verify the CONTENT dirty flag is available and distinct
    let flags = DirtyFlags::CONTENT;
    assert!(!flags.is_empty());
    assert!(!flags.contains(DirtyFlags::PAINT));
    assert!(!flags.contains(DirtyFlags::STYLE));
    assert!(!flags.contains(DirtyFlags::LAYOUT));
    assert!(!flags.contains(DirtyFlags::CHILDREN));
}

#[test]
fn element_def_persistent_buffer_builder() {
    // Verify the builder method works
    let def = ElementDef::new(Tag::Div).with_persistent_buffer(true);
    assert!(def.persistent_buffer);

    let def2 = ElementDef::new(Tag::Div).with_persistent_buffer(false);
    assert!(!def2.persistent_buffer);
}

#[test]
fn update_from_def_transfers_persistent_buffer_flag() {
    let mut elem = Element::new(Tag::Div);
    assert!(!elem.persistent_buffer);

    let def = ElementDef::new(Tag::Div).with_persistent_buffer(true);
    elem.update_from_def(&def);
    assert!(elem.persistent_buffer);
}

#[test]
fn structural_change_triggers_full_rebuild() {
    // When an element's grid dimensions change, the damage state should
    // flag a structural change and the buffer manager should handle resize.
    let mut mgr = PersistentBufferManager::new();
    let mut tracker = DamageTracker::new();
    let node = test_node(0);

    mgr.register(node, 24, 80, 1, 1);
    tracker.register(node, 24, 80);

    mgr.get_mut(node).unwrap().needs_full_upload = false;

    // Trigger a resize through the damage tracker
    tracker.get_mut(node).unwrap().resize(30, 100);

    mgr.process_damage(&mut tracker);

    let buf = mgr.get(node).unwrap();
    assert_eq!(buf.rows, 30);
    assert_eq!(buf.cols, 100);
    assert!(buf.needs_full_upload);
}

#[test]
fn empty_damage_produces_no_uploads() {
    let mut mgr = PersistentBufferManager::new();
    let mut tracker = DamageTracker::new();
    let node = test_node(0);

    mgr.register(node, 24, 80, 1, 1);
    tracker.register(node, 24, 80);
    mgr.get_mut(node).unwrap().needs_full_upload = false;

    // No damage recorded
    mgr.process_damage(&mut tracker);

    let uploads = mgr.take_pending_uploads();
    assert!(uploads.is_empty());
}

#[test]
fn patch_glyphs_writes_correct_cells() {
    let mut buf = PersistentBuffer::new(4, 4, 1, 1);

    let region = DamageRegion::new(1, 3, 1, 3);
    let new_glyphs: Vec<GlyphInstance> = (0..4)
        .map(|i| {
            let mut g: GlyphInstance = bytemuck::Zeroable::zeroed();
            g.color = [i as f32, 0.0, 0.0, 1.0];
            g
        })
        .collect();

    buf.patch_glyphs(&region, &new_glyphs);

    // Cell (1,1) is at index 5 in a 4-col grid
    assert_eq!(buf.glyphs[5].color, [0.0, 0.0, 0.0, 1.0]);
    // Cell (1,2) is at index 6
    assert_eq!(buf.glyphs[6].color, [1.0, 0.0, 0.0, 1.0]);
    // Cell (2,1) is at index 9
    assert_eq!(buf.glyphs[9].color, [2.0, 0.0, 0.0, 1.0]);
    // Cell (2,2) is at index 10
    assert_eq!(buf.glyphs[10].color, [3.0, 0.0, 0.0, 1.0]);

    // Unaffected cells are zero
    assert_eq!(buf.glyphs[0].color, [0.0; 4]);
}
