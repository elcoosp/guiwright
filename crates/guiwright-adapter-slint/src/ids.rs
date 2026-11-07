//! The `(component, item)` ↔ accesskit `NodeId` bridge (§4.4, §6.1
//! `NodeKey::Slint { component, item }`).
//!
//! Slint items are addressed by their position in the item tree
//! (component instance index + item index within it). Testing-backend
//! builds (with `SLINT_EMIT_DEBUG_INFO=1`, §11) also carry element names,
//! which become test ids. The bridge flattens the pair into one u64
//! handle for the tree engine and back.

/// A slint item identity: component instance + item index (§6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlintId {
    pub component: u64,
    pub item: u32,
}

impl SlintId {
    pub const fn new(component: u64, item: u32) -> Self {
        Self { component, item }
    }

    /// The engine handle: `(component << 32) | item` — lossless for
    /// component indexes that fit in u32 (slint item counts are small).
    pub const fn to_handle(self) -> u64 {
        ((self.component & 0xFFFF_FFFF) << 32) | self.item as u64
    }

    /// Recover the pair from a handle.
    pub const fn from_handle(handle: u64) -> Self {
        Self {
            component: handle >> 32,
            item: handle as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_roundtrip_is_lossless() {
        // Lossless for component indexes within u32 (realistic instances).
        for (component, item) in [(0u64, 0u32), (1, 7), (0x1234_5678, 0xFFFF_FFFF)] {
            let id = SlintId::new(component, item);
            let back = SlintId::from_handle(id.to_handle());
            assert_eq!(back, id, "({component}, {item})");
        }
    }

    #[test]
    fn oversized_components_survive_truncated() {
        // Component indexes beyond u32 lose their high bits (documented):
        // item ordering still holds within one component.
        let id = SlintId::new(u64::MAX, 3);
        let handle = id.to_handle();
        assert_eq!(handle as u32, 3);
        assert_eq!(SlintId::from_handle(handle).item, 3);
    }
}
