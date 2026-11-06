//! Node identity keys and generation counters.
//!
//! Level-1 adapters address nodes through the canonical AccessKit identity;
//! Level-2 adapters normalize their framework identities into the
//! [`NodeKey::Logical`] handle (architecture §6.1 "Level-2 framework
//! identities (normalized into core's handles)").

use accesskit::{NodeId, TreeId};

/// Identity of a node across the driver boundary.
///
/// `Copy` on purpose: keys are handed around at frame boundaries and stored
/// in small bounded queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKey {
    /// Level-1 canonical identity (AccessKit).
    AccessKit { tree: TreeId, node: NodeId },
    /// Level-2 identity normalized by the owning adapter. `adapter` is the
    /// adapter's static name (e.g. `"iced"`), `id` is adapter-minted and
    /// unique within a session.
    Logical { adapter: &'static str, id: u64 },
}

impl NodeKey {
    /// Human-friendly identity used in error reports.
    pub fn describe(&self) -> String {
        match self {
            NodeKey::AccessKit { node, .. } => format!("accesskit#{}", node.0),
            NodeKey::Logical { adapter, id } => format!("{adapter}#{id}"),
        }
    }
}

/// Generation of a node identity within a session (§8.2).
///
/// Ids are recycled by some frameworks (floem slotmap keys, iced `Unique`
/// ids). A generation increments every time an identity is removed from the
/// tree, so a stale [`crate::tree::Resolved`] can be detected before an
/// action is applied (ABA guard).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u32);

impl Generation {
    pub fn next(self) -> Self {
        Generation(self.0.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_monotonic() {
        let g = Generation::default();
        assert_eq!(g.next().next(), Generation(2));
    }

    #[test]
    fn describe_is_stable() {
        let key = NodeKey::AccessKit {
            tree: TreeId::ROOT,
            node: NodeId(7),
        };
        assert_eq!(key.describe(), "accesskit#7");
        let key = NodeKey::Logical {
            adapter: "iced",
            id: 42,
        };
        assert_eq!(key.describe(), "iced#42");
    }
}
