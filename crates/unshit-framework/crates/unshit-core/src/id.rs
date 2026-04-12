use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Sentinel raw value meaning "no NodeId is stored".
const NO_NODE: u64 = u64::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub index: u32,
    pub generation: u32,
}

impl NodeId {
    pub const DANGLING: NodeId = NodeId { index: u32::MAX, generation: 0 };

    pub fn is_dangling(self) -> bool {
        self.index == u32::MAX
    }

    /// Pack this `NodeId` into a raw `u64` for atomic storage.
    /// Uses `(index as u64) << 32 | generation as u64`.
    pub fn to_raw(self) -> u64 {
        ((self.index as u64) << 32) | (self.generation as u64)
    }

    /// Unpack a raw `u64` produced by [`NodeId::to_raw`] back into a `NodeId`.
    pub fn from_raw(raw: u64) -> NodeId {
        NodeId { index: (raw >> 32) as u32, generation: (raw & 0xFFFF_FFFF) as u32 }
    }
}

/// A ref-counted, shared handle that the reconciler fills with a `NodeId`
/// when the element is mounted and clears when it is unmounted.
///
/// `NodeRef` is `Clone + Default + Send + Sync`. Clones share the same
/// underlying atomic slot, so any clone sees the same value.
#[derive(Clone)]
pub struct NodeRef(Arc<AtomicU64>);

impl Default for NodeRef {
    fn default() -> Self {
        NodeRef::new()
    }
}

impl NodeRef {
    /// Create a new, empty `NodeRef`.
    pub fn new() -> Self {
        NodeRef(Arc::new(AtomicU64::new(NO_NODE)))
    }

    /// Returns the stored `NodeId`, or `None` if the element is not currently
    /// mounted (or has been cleared).
    pub fn get(&self) -> Option<NodeId> {
        let raw = self.0.load(Ordering::Acquire);
        if raw == NO_NODE {
            None
        } else {
            Some(NodeId::from_raw(raw))
        }
    }

    /// Store a `NodeId`. Called by the reconciler after `arena.alloc`.
    pub fn set(&self, id: NodeId) {
        self.0.store(id.to_raw(), Ordering::Release);
    }

    /// Clear the stored value. Called by the reconciler before `arena.dealloc`.
    pub fn clear(&self) {
        self.0.store(NO_NODE, Ordering::Release);
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dangling() {
            write!(f, "NodeId(DANGLING)")
        } else {
            write!(f, "NodeId({}:gen{})", self.index, self.generation)
        }
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dangling() {
            write!(f, "DANGLING")
        } else {
            write!(f, "{}:g{}", self.index, self.generation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_to_raw_from_raw_roundtrip() {
        let id = NodeId { index: 42, generation: 7 };
        let raw = id.to_raw();
        let restored = NodeId::from_raw(raw);
        assert_eq!(restored.index, id.index);
        assert_eq!(restored.generation, id.generation);
    }

    #[test]
    fn node_id_to_raw_from_raw_zero() {
        let id = NodeId { index: 0, generation: 0 };
        let raw = id.to_raw();
        assert_eq!(raw, 0);
        let restored = NodeId::from_raw(raw);
        assert_eq!(restored.index, 0);
        assert_eq!(restored.generation, 0);
    }

    #[test]
    fn node_ref_new_is_empty() {
        let nr = NodeRef::new();
        assert!(nr.get().is_none());
    }

    #[test]
    fn node_ref_default_is_empty() {
        let nr = NodeRef::default();
        assert!(nr.get().is_none());
    }

    #[test]
    fn node_ref_set_and_get() {
        let nr = NodeRef::new();
        let id = NodeId { index: 10, generation: 3 };
        nr.set(id);
        let got = nr.get().expect("should have a value after set");
        assert_eq!(got.index, id.index);
        assert_eq!(got.generation, id.generation);
    }

    #[test]
    fn node_ref_clear() {
        let nr = NodeRef::new();
        let id = NodeId { index: 5, generation: 1 };
        nr.set(id);
        assert!(nr.get().is_some());
        nr.clear();
        assert!(nr.get().is_none());
    }

    #[test]
    fn node_ref_clone_shares_slot() {
        let nr1 = NodeRef::new();
        let nr2 = nr1.clone();
        let id = NodeId { index: 99, generation: 2 };
        nr1.set(id);
        // nr2 is a clone and must see the same value
        let got = nr2.get().expect("clone should share the same slot");
        assert_eq!(got.index, 99);
        assert_eq!(got.generation, 2);
        // clearing via nr2 should be visible in nr1
        nr2.clear();
        assert!(nr1.get().is_none());
    }
}
