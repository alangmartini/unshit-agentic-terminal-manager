use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DirtyFlags: u32 {
        const STYLE    = 0b0000_0001;
        const LAYOUT   = 0b0000_0010;
        const PAINT    = 0b0000_0100;
        const CHILDREN = 0b0000_1000;
        /// Content changed but no structural change. Persistent buffer
        /// elements use this to trigger a partial buffer update instead
        /// of a full batch rebuild.
        const CONTENT  = 0b0001_0000;
        /// Set on ancestor nodes when any descendant in the subtree has
        /// STYLE dirty. Allows the cascade to skip clean subtrees entirely.
        const SUBTREE_STYLE = 0b0100_0000;
        /// Set on ancestor nodes when any descendant in the subtree has
        /// LAYOUT dirty. Reserved for future layout short-circuit.
        const SUBTREE_LAYOUT = 0b1000_0000;
        /// Set on ancestor nodes when at least one descendant in the subtree
        /// has PAINT dirty. Allows the batch builder to skip clean subtrees
        /// entirely, so idle UI regions cost zero draw work.
        const SUBTREE_PAINT = 0b0010_0000_0000;
    }
}

impl DirtyFlags {
    pub fn needs_work(self) -> bool {
        !self.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_and_subtree_paint_exist_and_compose() {
        let flags = DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT;
        assert!(flags.contains(DirtyFlags::PAINT));
        assert!(flags.contains(DirtyFlags::SUBTREE_PAINT));
        // They should not overlap with any other flags
        let other = DirtyFlags::STYLE
            | DirtyFlags::LAYOUT
            | DirtyFlags::CHILDREN
            | DirtyFlags::CONTENT
            | DirtyFlags::SUBTREE_STYLE
            | DirtyFlags::SUBTREE_LAYOUT;
        assert!((flags & other).is_empty());
    }

    #[test]
    fn paint_flag_only_covers_own_node() {
        let paint_only = DirtyFlags::PAINT;
        assert!(!paint_only.contains(DirtyFlags::SUBTREE_PAINT));
    }

    #[test]
    fn subtree_paint_flag_only_covers_descendants() {
        let subtree_only = DirtyFlags::SUBTREE_PAINT;
        assert!(!subtree_only.contains(DirtyFlags::PAINT));
    }
}
