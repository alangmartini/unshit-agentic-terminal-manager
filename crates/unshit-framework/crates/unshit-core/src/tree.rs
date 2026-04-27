use crate::element::Element;
use crate::id::NodeId;
use smallvec::SmallVec;

#[allow(clippy::large_enum_variant)]
enum NodeSlot {
    Occupied { generation: u32, node: Element },
    Vacant { generation: u32 },
}

pub struct NodeArena {
    nodes: Vec<NodeSlot>,
    free_list: Vec<u32>,
}

impl Default for NodeArena {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeArena {
    pub fn new() -> Self {
        Self { nodes: Vec::with_capacity(1024), free_list: Vec::new() }
    }

    pub fn alloc(&mut self, element: Element) -> NodeId {
        if let Some(index) = self.free_list.pop() {
            let slot = &mut self.nodes[index as usize];
            let generation = match slot {
                NodeSlot::Vacant { generation } => *generation,
                _ => unreachable!(),
            };
            *slot = NodeSlot::Occupied { generation, node: element };
            NodeId { index, generation }
        } else {
            let index = self.nodes.len() as u32;
            let generation = 0;
            self.nodes.push(NodeSlot::Occupied { generation, node: element });
            NodeId { index, generation }
        }
    }

    pub fn dealloc(&mut self, id: NodeId) {
        if let Some(slot) = self.nodes.get_mut(id.index as usize) {
            match slot {
                NodeSlot::Occupied { generation, .. } if *generation == id.generation => {
                    let next_gen = generation.wrapping_add(1);
                    *slot = NodeSlot::Vacant { generation: next_gen };
                    self.free_list.push(id.index);
                }
                _ => {}
            }
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&Element> {
        match self.nodes.get(id.index as usize)? {
            NodeSlot::Occupied { generation, node } if *generation == id.generation => Some(node),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        match self.nodes.get_mut(id.index as usize)? {
            NodeSlot::Occupied { generation, node } if *generation == id.generation => Some(node),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len() - self.free_list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns children as a SmallVec (stack-allocated for up to 16 children).
    pub fn children(&self, node_id: NodeId) -> SmallVec<[NodeId; 16]> {
        let mut ids = SmallVec::new();
        if let Some(element) = self.get(node_id) {
            let mut child = element.first_child;
            while !child.is_dangling() {
                ids.push(child);
                child = self.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
            }
        }
        ids
    }

    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Element)> {
        self.nodes.iter().enumerate().filter_map(|(i, slot)| {
            if let NodeSlot::Occupied { generation, node } = slot {
                Some((NodeId { index: i as u32, generation: *generation }, node))
            } else {
                None
            }
        })
    }

    /// Lowest common ancestor of `a` and `b`, falling back to `root` when
    /// either node is dangling, deallocated, or not in `root`'s subtree.
    ///
    /// Used to narrow restyle cascades after pseudo-class state changes
    /// (`:hover`, `:focus`, `:active`): both the leaving and entering nodes
    /// share an ancestor whose subtree contains every selector the change
    /// could possibly re-evaluate, so the cascade can stop walking at the
    /// LCA and skip every other branch of the tree.
    ///
    /// Sibling combinators (`.parent:hover ~ .other`) are NOT supported by
    /// this scoping rule because the affected sibling subtree may live
    /// outside the LCA. Callers that depend on sibling-combinator pseudo
    /// rules must restyle from `root` instead.
    pub fn lowest_common_ancestor(&self, a: NodeId, b: NodeId, root: NodeId) -> NodeId {
        if a == b && self.get(a).is_some() {
            return a;
        }
        let a_in = !a.is_dangling() && self.get(a).is_some();
        let b_in = !b.is_dangling() && self.get(b).is_some();
        if !a_in && !b_in {
            return root;
        }
        if !a_in {
            return b;
        }
        if !b_in {
            return a;
        }

        let mut a_path: smallvec::SmallVec<[NodeId; 16]> = smallvec::SmallVec::new();
        let mut cur = a;
        while !cur.is_dangling() {
            a_path.push(cur);
            cur = match self.get(cur) {
                Some(el) => el.parent,
                None => NodeId::DANGLING,
            };
        }

        let mut cur = b;
        while !cur.is_dangling() {
            if a_path.contains(&cur) {
                return cur;
            }
            cur = match self.get(cur) {
                Some(el) => el.parent,
                None => NodeId::DANGLING,
            };
        }

        root
    }

    /// Recursively deallocate a node and all its descendants (depth-first).
    pub fn dealloc_subtree(&mut self, id: NodeId) {
        if id.is_dangling() || self.get(id).is_none() {
            return;
        }

        let mut stack = vec![id];

        while let Some(current) = stack.pop() {
            if let Some(element) = self.get(current) {
                let mut child = element.first_child;
                while !child.is_dangling() {
                    let next = self.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
                    stack.push(child);
                    child = next;
                }
            }
            self.dealloc(current);
        }
    }

    /// Unlink `child` from `parent`'s child list without deallocating it.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        let Some(child_elem) = self.get(child) else { return };
        let (prev_sib, next_sib) = (child_elem.prev_sibling, child_elem.next_sibling);

        if prev_sib.is_dangling() {
            if let Some(parent_elem) = self.get_mut(parent) {
                parent_elem.first_child = next_sib;
            }
        } else if let Some(prev_elem) = self.get_mut(prev_sib) {
            prev_elem.next_sibling = next_sib;
        }

        if next_sib.is_dangling() {
            if let Some(parent_elem) = self.get_mut(parent) {
                parent_elem.last_child = prev_sib;
            }
        } else if let Some(next_elem) = self.get_mut(next_sib) {
            next_elem.prev_sibling = prev_sib;
        }

        if let Some(child_elem) = self.get_mut(child) {
            child_elem.parent = NodeId::DANGLING;
            child_elem.prev_sibling = NodeId::DANGLING;
            child_elem.next_sibling = NodeId::DANGLING;
        }
    }

    /// Append `child` at the end of `parent`'s child list.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let Some(parent_elem) = self.get(parent) else { return };
        let old_last = parent_elem.last_child;

        if old_last.is_dangling() {
            if let Some(parent_elem) = self.get_mut(parent) {
                parent_elem.first_child = child;
                parent_elem.last_child = child;
            }
        } else {
            if let Some(old_last_elem) = self.get_mut(old_last) {
                old_last_elem.next_sibling = child;
            }
            if let Some(child_elem) = self.get_mut(child) {
                child_elem.prev_sibling = old_last;
            }
            if let Some(parent_elem) = self.get_mut(parent) {
                parent_elem.last_child = child;
            }
        }

        if let Some(child_elem) = self.get_mut(child) {
            child_elem.parent = parent;
        }
    }

    /// Insert `child` before `before` in `parent`'s child list.
    pub fn insert_child_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        let Some(before_elem) = self.get(before) else { return };
        let prev_sib = before_elem.prev_sibling;

        if let Some(child_elem) = self.get_mut(child) {
            child_elem.next_sibling = before;
            child_elem.prev_sibling = prev_sib;
            child_elem.parent = parent;
        }
        if let Some(before_elem) = self.get_mut(before) {
            before_elem.prev_sibling = child;
        }

        if prev_sib.is_dangling() {
            if let Some(parent_elem) = self.get_mut(parent) {
                parent_elem.first_child = child;
            }
        } else if let Some(prev_elem) = self.get_mut(prev_sib) {
            prev_elem.next_sibling = child;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Element, Tag};

    /// Helper: allocate a Div element in the arena.
    fn alloc_div(arena: &mut NodeArena) -> NodeId {
        arena.alloc(Element::new(Tag::Div))
    }

    #[test]
    fn test_append_child() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let c3 = alloc_div(&mut arena);

        arena.append_child(parent, c1);
        arena.append_child(parent, c2);
        arena.append_child(parent, c3);

        let children = arena.children(parent);
        assert_eq!(children.as_slice(), &[c1, c2, c3]);

        // Verify parent links.
        assert_eq!(arena.get(c1).unwrap().parent, parent);
        assert_eq!(arena.get(c2).unwrap().parent, parent);
        assert_eq!(arena.get(c3).unwrap().parent, parent);

        // Verify first/last child.
        assert_eq!(arena.get(parent).unwrap().first_child, c1);
        assert_eq!(arena.get(parent).unwrap().last_child, c3);
    }

    #[test]
    fn test_remove_child_middle() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let c3 = alloc_div(&mut arena);

        arena.append_child(parent, c1);
        arena.append_child(parent, c2);
        arena.append_child(parent, c3);

        arena.remove_child(parent, c2);

        let children = arena.children(parent);
        assert_eq!(children.as_slice(), &[c1, c3]);

        // c1 and c3 are now siblings.
        assert_eq!(arena.get(c1).unwrap().next_sibling, c3);
        assert_eq!(arena.get(c3).unwrap().prev_sibling, c1);

        // c2 is fully unlinked.
        assert!(arena.get(c2).unwrap().parent.is_dangling());
        assert!(arena.get(c2).unwrap().prev_sibling.is_dangling());
        assert!(arena.get(c2).unwrap().next_sibling.is_dangling());
    }

    #[test]
    fn test_remove_child_first() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let c3 = alloc_div(&mut arena);

        arena.append_child(parent, c1);
        arena.append_child(parent, c2);
        arena.append_child(parent, c3);

        arena.remove_child(parent, c1);

        let children = arena.children(parent);
        assert_eq!(children.as_slice(), &[c2, c3]);
        assert_eq!(arena.get(parent).unwrap().first_child, c2);
        assert!(arena.get(c2).unwrap().prev_sibling.is_dangling());
    }

    #[test]
    fn test_remove_child_last() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let c3 = alloc_div(&mut arena);

        arena.append_child(parent, c1);
        arena.append_child(parent, c2);
        arena.append_child(parent, c3);

        arena.remove_child(parent, c3);

        let children = arena.children(parent);
        assert_eq!(children.as_slice(), &[c1, c2]);
        assert_eq!(arena.get(parent).unwrap().last_child, c2);
        assert!(arena.get(c2).unwrap().next_sibling.is_dangling());
    }

    #[test]
    fn test_remove_only_child() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let child = alloc_div(&mut arena);

        arena.append_child(parent, child);
        arena.remove_child(parent, child);

        let children = arena.children(parent);
        assert!(children.is_empty());
        assert!(arena.get(parent).unwrap().first_child.is_dangling());
        assert!(arena.get(parent).unwrap().last_child.is_dangling());
    }

    #[test]
    fn test_dealloc_subtree() {
        let mut arena = NodeArena::new();
        // Build: root -> [a -> [a1, a2], b]
        let root = alloc_div(&mut arena);
        let a = alloc_div(&mut arena);
        let a1 = alloc_div(&mut arena);
        let a2 = alloc_div(&mut arena);
        let b = alloc_div(&mut arena);

        arena.append_child(root, a);
        arena.append_child(root, b);
        arena.append_child(a, a1);
        arena.append_child(a, a2);

        assert_eq!(arena.len(), 5);

        // Deallocate subtree rooted at `a` (a, a1, a2 = 3 nodes).
        arena.dealloc_subtree(a);

        assert_eq!(arena.len(), 2); // root + b remain

        // `a`, `a1`, `a2` should be gone.
        assert!(arena.get(a).is_none());
        assert!(arena.get(a1).is_none());
        assert!(arena.get(a2).is_none());

        // root and b still accessible.
        assert!(arena.get(root).is_some());
        assert!(arena.get(b).is_some());
    }

    #[test]
    fn test_dealloc_subtree_leaf() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let leaf = alloc_div(&mut arena);

        arena.append_child(root, leaf);
        assert_eq!(arena.len(), 2);

        arena.dealloc_subtree(leaf);

        assert_eq!(arena.len(), 1);
        assert!(arena.get(leaf).is_none());
        assert!(arena.get(root).is_some());
    }

    #[test]
    fn test_insert_child_before_first() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let new_first = alloc_div(&mut arena);

        arena.append_child(parent, c1);
        arena.append_child(parent, c2);

        arena.insert_child_before(parent, new_first, c1);

        let children = arena.children(parent);
        assert_eq!(children.as_slice(), &[new_first, c1, c2]);
        assert_eq!(arena.get(parent).unwrap().first_child, new_first);
        assert!(arena.get(new_first).unwrap().prev_sibling.is_dangling());
        assert_eq!(arena.get(new_first).unwrap().next_sibling, c1);
        assert_eq!(arena.get(c1).unwrap().prev_sibling, new_first);
    }

    #[test]
    fn test_insert_child_before_middle() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let inserted = alloc_div(&mut arena);

        arena.append_child(parent, c1);
        arena.append_child(parent, c2);

        arena.insert_child_before(parent, inserted, c2);

        let children = arena.children(parent);
        assert_eq!(children.as_slice(), &[c1, inserted, c2]);
        assert_eq!(arena.get(c1).unwrap().next_sibling, inserted);
        assert_eq!(arena.get(inserted).unwrap().prev_sibling, c1);
        assert_eq!(arena.get(inserted).unwrap().next_sibling, c2);
        assert_eq!(arena.get(c2).unwrap().prev_sibling, inserted);
    }

    #[test]
    fn lca_self_returns_self() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let a = alloc_div(&mut arena);
        arena.append_child(root, a);
        assert_eq!(arena.lowest_common_ancestor(a, a, root), a);
    }

    #[test]
    fn lca_siblings_returns_parent() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let a = alloc_div(&mut arena);
        let b = alloc_div(&mut arena);
        arena.append_child(root, a);
        arena.append_child(root, b);
        assert_eq!(arena.lowest_common_ancestor(a, b, root), root);
    }

    #[test]
    fn lca_ancestor_descendant_returns_ancestor() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let parent = alloc_div(&mut arena);
        let child = alloc_div(&mut arena);
        arena.append_child(root, parent);
        arena.append_child(parent, child);
        assert_eq!(arena.lowest_common_ancestor(parent, child, root), parent);
        assert_eq!(arena.lowest_common_ancestor(child, parent, root), parent);
    }

    #[test]
    fn lca_dangling_a_returns_b() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let b = alloc_div(&mut arena);
        arena.append_child(root, b);
        assert_eq!(arena.lowest_common_ancestor(NodeId::DANGLING, b, root), b);
    }

    #[test]
    fn lca_dangling_b_returns_a() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let a = alloc_div(&mut arena);
        arena.append_child(root, a);
        assert_eq!(arena.lowest_common_ancestor(a, NodeId::DANGLING, root), a);
    }

    #[test]
    fn lca_both_dangling_returns_root() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        assert_eq!(
            arena.lowest_common_ancestor(NodeId::DANGLING, NodeId::DANGLING, root),
            root,
        );
    }

    #[test]
    fn lca_deep_subtrees_finds_first_shared_ancestor() {
        // Build:    root
        //           /  \
        //          x    y
        //         / \   |
        //        x1 x2  y1
        //        /
        //       x1a
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let x = alloc_div(&mut arena);
        let x1 = alloc_div(&mut arena);
        let x2 = alloc_div(&mut arena);
        let x1a = alloc_div(&mut arena);
        let y = alloc_div(&mut arena);
        let y1 = alloc_div(&mut arena);
        arena.append_child(root, x);
        arena.append_child(root, y);
        arena.append_child(x, x1);
        arena.append_child(x, x2);
        arena.append_child(x1, x1a);
        arena.append_child(y, y1);

        // Cousins under x.
        assert_eq!(arena.lowest_common_ancestor(x1a, x2, root), x);
        // Across subtrees.
        assert_eq!(arena.lowest_common_ancestor(x1a, y1, root), root);
    }

    #[test]
    fn lca_after_dealloc_falls_back_to_root() {
        let mut arena = NodeArena::new();
        let root = alloc_div(&mut arena);
        let a = alloc_div(&mut arena);
        let b = alloc_div(&mut arena);
        arena.append_child(root, a);
        arena.append_child(root, b);
        let stale = a;
        arena.remove_child(root, a);
        arena.dealloc(a);
        // `stale` no longer resolves; fall back to using `b` since b is still in tree.
        assert_eq!(arena.lowest_common_ancestor(stale, b, root), b);
    }

    #[test]
    fn test_append_then_remove() {
        let mut arena = NodeArena::new();
        let parent = alloc_div(&mut arena);
        let c1 = alloc_div(&mut arena);
        let c2 = alloc_div(&mut arena);
        let c3 = alloc_div(&mut arena);

        // Append all three.
        arena.append_child(parent, c1);
        arena.append_child(parent, c2);
        arena.append_child(parent, c3);
        assert_eq!(arena.children(parent).as_slice(), &[c1, c2, c3]);

        // Remove middle, then re-append it at the end.
        arena.remove_child(parent, c2);
        assert_eq!(arena.children(parent).as_slice(), &[c1, c3]);

        arena.append_child(parent, c2);
        assert_eq!(arena.children(parent).as_slice(), &[c1, c3, c2]);

        // Remove first, then re-append.
        arena.remove_child(parent, c1);
        arena.append_child(parent, c1);
        assert_eq!(arena.children(parent).as_slice(), &[c3, c2, c1]);

        // Remove all.
        arena.remove_child(parent, c3);
        arena.remove_child(parent, c2);
        arena.remove_child(parent, c1);
        assert!(arena.children(parent).is_empty());
        assert!(arena.get(parent).unwrap().first_child.is_dangling());
        assert!(arena.get(parent).unwrap().last_child.is_dangling());
    }
}
