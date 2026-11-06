//! The `AccessibilityId` ↔ accesskit `NodeId` bridge (§4.3).
//!
//! Freya's accessibility ids are app-assigned (`AccessibilityId(n)`) and
//! its `AccessibilityTree` maps them to internal `NodeId`s; Guiwright
//! addresses nodes through accesskit `NodeId`s. This map is the single
//! owner of that correspondence: binds once per freya id (stable across
//! passes), tracks generations for ids that leave the tree (§8.2 ABA
//! guard — auto-assigned freya ids are a process counter and recycle
//! across re-mounts).

use std::collections::HashMap;

use accesskit::NodeId;

/// Bidirectional, allocation-light id bridge.
#[derive(Clone, Debug, Default)]
pub struct AccessibilityMap {
    freya_to_node: HashMap<u64, NodeId>,
    node_to_freya: HashMap<NodeId, u64>,
    generations: HashMap<u64, u32>,
}

impl AccessibilityMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind (or fetch) the accesskit `NodeId` for a freya accessibility
    /// id. Bindings are stable for the lifetime of the id in the tree.
    pub fn bind(&mut self, freya_id: u64) -> NodeId {
        if let Some(node) = self.freya_to_node.get(&freya_id) {
            return *node;
        }
        let node = NodeId(freya_id);
        self.freya_to_node.insert(freya_id, node);
        self.node_to_freya.insert(node, freya_id);
        node
    }

    pub fn node_of(&self, freya_id: u64) -> Option<NodeId> {
        self.freya_to_node.get(&freya_id).copied()
    }

    pub fn freya_of(&self, node: NodeId) -> Option<u64> {
        self.node_to_freya.get(&node).copied()
    }

    /// Record ids that left the tree this pass (§4.3 dirty-node diffing):
    /// bumps the generation so a recycled auto id re-binds fresh (§8.2).
    pub fn unbind_all(&mut self, removed: impl IntoIterator<Item = u64>) {
        for freya_id in removed {
            if let Some(node) = self.freya_to_node.remove(&freya_id) {
                self.node_to_freya.remove(&node);
                let generation = self.generations.entry(freya_id).or_insert(0);
                *generation = generation.saturating_add(1);
            }
        }
    }

    /// Current ABA generation of a freya id (§8.2).
    pub fn generation(&self, freya_id: u64) -> u32 {
        self.generations.get(&freya_id).copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.freya_to_node.len()
    }

    pub fn is_empty(&self) -> bool {
        self.freya_to_node.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_are_stable_and_bidirectional() {
        let mut map = AccessibilityMap::new();
        let a = map.bind(7);
        let b = map.bind(7);
        assert_eq!(a, b, "same freya id → same NodeId");
        assert_eq!(map.node_of(7), Some(a));
        assert_eq!(map.freya_of(a), Some(7));
        assert_eq!(map.node_of(9), None, "unbound ids stay unbound");
    }

    #[test]
    fn distinct_ids_never_collide() {
        let mut map = AccessibilityMap::new();
        let a = map.bind(1);
        let b = map.bind(2);
        assert_ne!(a, b);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn unbound_ids_bump_generations_and_rebind_cleanly() {
        let mut map = AccessibilityMap::new();
        let first = map.bind(5);
        assert_eq!(map.generation(5), 0);
        map.unbind_all([5]);
        assert_eq!(map.generation(5), 1);
        assert_eq!(map.node_of(5), None);
        let second = map.bind(5);
        assert_eq!(
            first, second,
            "freya ids are stable numbers — rebind is identity"
        );
        assert_eq!(map.generation(5), 1, "generation survives rebind");
    }

    #[test]
    fn unbind_unknown_id_is_a_noop() {
        let mut map = AccessibilityMap::new();
        map.unbind_all([42]);
        assert!(map.is_empty());
        assert_eq!(map.generation(42), 0);
    }
}
