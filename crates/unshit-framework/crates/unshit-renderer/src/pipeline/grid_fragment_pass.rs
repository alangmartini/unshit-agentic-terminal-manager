//! Consumer of [`GridDrawRecord`]s produced during the batch walk when the
//! experimental fragment shader grid path is active.
//!
//! Step 2 scope (see issue #96): this is the wiring stub. The pass owns a
//! lazily constructed [`GridFragmentPipeline`] and tracks how many records
//! it has seen per frame and overall. It does not yet issue any GPU draw
//! calls; that lands in Step 4 once the glyph meta bridge is in place.
//!
//! The stats are exposed so unit tests can prove the wiring is active
//! without needing an adapter.

use crate::batch::GridDrawRecord;
use crate::pipeline::grid_fragment::GridFragmentPipeline;

/// Per frame and cumulative counters for the fragment grid path. Used by
/// tests and by future benches as a lightweight activity signal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridFragmentPassStats {
    pub last_frame_records: usize,
    pub total_records_seen: u64,
    pub frames_with_records: u64,
}

/// Owner of the experimental fragment grid pipeline and per frame stats.
///
/// The pipeline is `Option<_>` because construction requires a device, an
/// atlas view, and an atlas sampler that are only available inside
/// `gpu.rs`. The pass lazily initializes the pipeline the first time it
/// actually has a record to process; feature builds that never flip the
/// runtime flag pay no GPU resource cost.
#[derive(Default)]
pub struct GridFragmentPass {
    pipeline: Option<GridFragmentPipeline>,
    stats: GridFragmentPassStats,
}

impl GridFragmentPass {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> GridFragmentPassStats {
        self.stats
    }

    pub fn has_pipeline(&self) -> bool {
        self.pipeline.is_some()
    }

    /// Record seen-count stats for this frame. Does not touch the GPU.
    ///
    /// Called from `gpu.rs` once per layer, after the main content pass,
    /// to accumulate the records the batch walk produced. Step 4 will add
    /// a real render method that takes `&mut RenderPass` and issues draw
    /// calls; the split keeps the stats path testable without a device.
    pub fn process(&mut self, records: &[GridDrawRecord]) {
        self.stats.last_frame_records = records.len();
        self.stats.total_records_seen =
            self.stats.total_records_seen.saturating_add(records.len() as u64);
        if !records.is_empty() {
            self.stats.frames_with_records = self.stats.frames_with_records.saturating_add(1);
        }
    }

    /// Reset the per frame counter. Called at the start of each render so
    /// `last_frame_records` reflects just the most recent frame.
    pub fn begin_frame(&mut self) {
        self.stats.last_frame_records = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unshit_core::id::NodeId;

    fn rec(node_id: NodeId) -> GridDrawRecord {
        GridDrawRecord {
            node_id,
            origin_x: 0.0,
            origin_y: 0.0,
            cell_w: 8.0,
            cell_h: 16.0,
            cols: 80,
            rows: 24,
            font_size: 14.0,
            opacity: 1.0,
            clip_rect: [0.0, 0.0, 9999.0, 9999.0],
        }
    }

    #[test]
    fn new_pass_has_zero_stats_and_no_pipeline() {
        let pass = GridFragmentPass::new();
        assert_eq!(pass.stats(), GridFragmentPassStats::default());
        assert!(!pass.has_pipeline());
    }

    #[test]
    fn process_empty_slice_does_not_bump_frames_with_records() {
        let mut pass = GridFragmentPass::new();
        pass.process(&[]);
        let s = pass.stats();
        assert_eq!(s.last_frame_records, 0);
        assert_eq!(s.total_records_seen, 0);
        assert_eq!(s.frames_with_records, 0);
    }

    #[test]
    fn process_accumulates_total_across_frames() {
        let mut pass = GridFragmentPass::new();
        pass.process(&[rec(NodeId::DANGLING), rec(NodeId::DANGLING)]);
        pass.begin_frame();
        pass.process(&[rec(NodeId::DANGLING)]);
        let s = pass.stats();
        assert_eq!(s.last_frame_records, 1);
        assert_eq!(s.total_records_seen, 3);
        assert_eq!(s.frames_with_records, 2);
    }

    #[test]
    fn begin_frame_clears_only_last_frame_count() {
        let mut pass = GridFragmentPass::new();
        pass.process(&[rec(NodeId::DANGLING)]);
        pass.begin_frame();
        let s = pass.stats();
        assert_eq!(s.last_frame_records, 0);
        assert_eq!(s.total_records_seen, 1);
    }
}
