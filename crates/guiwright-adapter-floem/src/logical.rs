//! The Level-2 tree state machine: [`ViewReport`]s in, a synthesized
//! AccessKit-shaped [`LogicalTreeDiff`](guiwright_driver::adapter::LogicalTreeDiff)
//! out (§4.7, §6.1 "Level-2 fallback: geometry + role/label polling").
//!
//! Identity model: a floem `ViewId` (slotmap index, carried as `u64`) is
//! bound to a minted AccessKit `NodeId` the first time it appears; the
//! consumer tree keys everything on that `NodeId` and actions arrive with
//! exactly the minted id (ABA-safe: a recycled slotmap key that re-enters
//! the tree gets a *fresh* `NodeId`, and the driver's generation counters
//! catch stale handles, §8.2).
//!
//! Updates are full per pass (the egui pattern, §4.1): the consumer diffs
//! at O(changes) so full updates cost nothing when nothing changed.
//! Geometry comes from `ViewId::get_visual_rect` (public, §4.7); roles
//! from the [`crate::roles`] registry; test ids from the
//! [`crate::ids`] registry.

use std::collections::HashMap;

use accesskit::{Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

use guiwright_driver::adapter::LogicalTreeDiff;

use crate::roles::RoleRegistry;

/// What the framework-facing side knows about one view after a layout
/// pass — the neutral vocabulary between the harness and this engine.
#[derive(Clone, Debug)]
pub struct ViewReport {
    /// Registry key ("button", "text_input", "vstack", …, §4.7).
    pub kind: String,
    /// Floem `ViewId` inner (slotmap index, §6.1 `NodeKey::Floem`).
    pub handle: u64,
    /// Text content — the accessible-name fallback (labels come from
    /// `Label`/`RichText` views; §4.7).
    pub text: Option<String>,
    /// Editable content (text inputs, sliders carry a formatted value).
    pub value: Option<String>,
    /// Explicit test id — pulled from the [`crate::ids`] registry by the
    /// harness; the diff registers it in the session's O(1) hash (§8.1).
    pub test_id: Option<String>,
    /// Visible bounds, window-relative physical px (`get_visual_rect`
    /// after the transform pass, AccessKit convention).
    pub bounds: Option<[f64; 4]>,
    pub children: Vec<ViewReport>,
    pub enabled: bool,
    pub focused: bool,
}

impl ViewReport {
    pub fn new(kind: &str, handle: u64) -> Self {
        Self {
            kind: kind.to_string(),
            handle,
            text: None,
            value: None,
            test_id: None,
            bounds: None,
            children: Vec::new(),
            enabled: true,
            focused: false,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_test_id(mut self, id: impl Into<String>) -> Self {
        self.test_id = Some(id.into());
        self
    }

    /// `get_visual_rect` output: floem returns origin + size.
    pub fn with_rect(mut self, origin: (f64, f64), size: (f64, f64)) -> Self {
        self.bounds = Some([origin.0, origin.1, origin.0 + size.0, origin.1 + size.1]);
        self
    }

    pub fn with_child(mut self, child: ViewReport) -> Self {
        self.children.push(child);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn focused(mut self) -> Self {
        self.focused = true;
        self
    }
}

#[derive(Debug)]
struct Slot {
    handle: u64,
    node_id: NodeId,
    role: Role,
}

/// Text normalization (whitespace collapse) — the accessible-name form.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The engine. Rebuilt per pass; owns the handle ↔ NodeId binding.
#[derive(Debug)]
pub struct FloemLogicalTree {
    registry: RoleRegistry,
    /// Adapter name — the `NodeKey::Logical` vocabulary ("floem").
    adapter: &'static str,
    slots: HashMap<u64, Slot>,
    /// Handle → generation; bumps when a handle leaves the tree so a
    /// recycled slotmap key never reuses a stale binding (§8.2).
    generations: HashMap<u64, u32>,
    next_node_id: u64,
}

impl FloemLogicalTree {
    pub fn new() -> Self {
        Self::with_registry(RoleRegistry::new())
    }

    pub fn with_registry(registry: RoleRegistry) -> Self {
        Self {
            registry,
            adapter: "floem",
            slots: HashMap::new(),
            generations: HashMap::new(),
            next_node_id: 0,
        }
    }

    pub fn registry(&self) -> &RoleRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut RoleRegistry {
        &mut self.registry
    }

    /// Map a minted `NodeId` back to the view handle (action routing).
    pub fn handle_of(&self, node: NodeId) -> Option<u64> {
        self.slots
            .values()
            .find(|slot| slot.node_id == node)
            .map(|slot| slot.handle)
    }

    /// The registry role a minted `NodeId` currently carries (action
    /// routing picks the click/fill vocabulary by role, §8.4).
    pub fn role_of(&self, node: NodeId) -> Option<Role> {
        self.slots
            .values()
            .find(|slot| slot.node_id == node)
            .map(|slot| slot.role)
    }

    /// Current ABA generation of a handle (§8.2).
    pub fn generation(&self, handle: u64) -> u32 {
        self.generations.get(&handle).copied().unwrap_or(0)
    }

    /// Fold one pass's view reports into a [`LogicalTreeDiff`].
    pub fn rebuild(&mut self, root: ViewReport) -> LogicalTreeDiff {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        let mut live: HashMap<u64, Slot> = HashMap::new();
        let mut focus: Option<NodeId> = None;
        let mut removed: Vec<u64> = Vec::new();
        let mut test_ids: Vec<(u64, String)> = Vec::new();

        let root_node_id = self.walk(&root, &mut nodes, &mut live, &mut focus, &mut test_ids);
        let root_node_id = root_node_id.unwrap_or(NodeId(0));

        // Handles that disappeared since the last pass — bump their
        // generation so recycled slotmap keys never alias (§8.2).
        for handle in self.slots.keys() {
            if !live.contains_key(handle) {
                let generation = self.generations.entry(*handle).or_insert(0);
                *generation = generation.saturating_add(1);
                removed.push(*handle);
            }
        }

        self.slots = live;

        let update = TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                root: root_node_id,
                toolkit_name: Some(format!("guiwright-{}", self.adapter)),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: focus.unwrap_or(root_node_id),
        };

        LogicalTreeDiff {
            adapter: self.adapter,
            update,
            removed,
            test_ids,
            changed: true,
        }
    }

    fn mint(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        NodeId(id)
    }

    fn walk(
        &mut self,
        report: &ViewReport,
        nodes: &mut Vec<(NodeId, Node)>,
        live: &mut HashMap<u64, Slot>,
        focus: &mut Option<NodeId>,
        test_ids: &mut Vec<(u64, String)>,
    ) -> Option<NodeId> {
        // Stable binding: a handle seen before keeps its minted NodeId —
        // unless it left the tree since (generation bump above): a handle
        // that re-enters always re-mints (ABA guard, §8.2).
        let was_live = self.slots.contains_key(&report.handle);
        let node_id = if was_live {
            self.slots[&report.handle].node_id
        } else {
            self.mint()
        };

        let role = self.registry.role_of(&report.kind);
        let mut node = Node::new(role);
        if let Some(text) = &report.text {
            node.set_label(normalize(text));
        }
        if let Some(value) = &report.value {
            node.set_value(normalize(value));
        }
        if let Some([x0, y0, x1, y1]) = report.bounds {
            node.set_bounds(Rect { x0, y0, x1, y1 });
        }
        if !report.enabled {
            node.set_disabled();
        }
        if report.focused {
            // accesskit 0.25 carries focus on the `TreeUpdate.focus` field.
            *focus = Some(node_id);
        }
        if let Some(test_id) = &report.test_id {
            test_ids.push((node_id.0, test_id.clone()));
        }

        let mut child_ids = Vec::with_capacity(report.children.len());
        for child in &report.children {
            if let Some(child_id) = self.walk(child, nodes, live, focus, test_ids) {
                child_ids.push(child_id);
            }
        }
        node.set_children(child_ids);

        nodes.push((node_id, node));
        live.insert(
            report.handle,
            Slot {
                handle: report.handle,
                node_id,
                role,
            },
        );
        Some(node_id)
    }
}

impl Default for FloemLogicalTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_app() -> ViewReport {
        ViewReport::new("window", 100)
            .with_text("Demo")
            .with_child(
                ViewReport::new("button", 1)
                    .with_text("Save")
                    .with_test_id("btn-save")
                    .with_rect((16.0, 16.0), (120.0, 32.0)),
            )
            .with_child(
                ViewReport::new("text_input", 2)
                    .with_text("Email")
                    .with_value("ada@example.dev")
                    .with_rect((16.0, 56.0), (240.0, 28.0)),
            )
            .with_child(ViewReport::new("button", 3).with_text("Confirm").disabled())
    }

    #[test]
    fn synthesizes_role_label_bounds_semantics() {
        let mut tree = FloemLogicalTree::new();
        let diff = tree.rebuild(sample_app());
        assert_eq!(diff.adapter, "floem");
        assert_eq!(diff.update.nodes.len(), 4, "window + 3 views");
        assert!(diff.changed);

        let button = diff
            .update
            .nodes
            .iter()
            .find(|(id, _)| id.0 == 1)
            .map(|(_, n)| n)
            .expect("button node");
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.label(), Some("Save"));
        let bounds = button.bounds().expect("visual rect carried");
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (16.0, 16.0, 136.0, 48.0)
        );

        let input = diff
            .update
            .nodes
            .iter()
            .find(|(id, _)| id.0 == 2)
            .map(|(_, n)| n)
            .expect("input node");
        assert_eq!(input.role(), Role::TextInput);
        assert_eq!(input.value(), Some("ada@example.dev"));

        // Test ids ride the diff for the session's O(1) hash (§8.1).
        assert_eq!(diff.test_ids, vec![(1, "btn-save".to_string())]);
    }

    #[test]
    fn stable_bindings_and_removal_generations() {
        let mut tree = FloemLogicalTree::new();
        let _first = tree.rebuild(sample_app());
        let save_id = NodeId(1);
        assert_eq!(tree.handle_of(save_id), Some(1));
        assert_eq!(tree.role_of(save_id), Some(Role::Button));

        // Same handles next pass → same minted NodeIds (stable identity).
        let second = tree.rebuild(sample_app());
        let again = second
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Button && n.label() == Some("Save"))
            .map(|(id, _)| *id)
            .expect("save button");
        assert_eq!(again, save_id, "stable across passes");

        // The Confirm button disappears → its handle's generation bumps.
        let removed = tree.rebuild(
            ViewReport::new("window", 100)
                .with_child(ViewReport::new("button", 1).with_text("Save")),
        );
        // The removal list is a set (HashMap iteration order is not a
        // contract); assert membership.
        let mut removed_sorted = removed.removed.clone();
        removed_sorted.sort_unstable();
        assert_eq!(removed_sorted, vec![2, 3], "input + confirm left");
        assert_eq!(tree.generation(3), 1, "ABA generation bumped");

        // A recycled handle 3 re-enters → fresh NodeId (ABA guard).
        let reused = tree.rebuild(
            ViewReport::new("window", 100)
                .with_child(ViewReport::new("button", 3).with_text("Recycled")),
        );
        let recycled_id = reused
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Recycled"))
            .map(|(id, _)| *id)
            .expect("recycled node");
        assert_ne!(recycled_id, NodeId(3), "fresh mint, never the stale id");
        assert_eq!(tree.handle_of(recycled_id), Some(3));
    }

    #[test]
    fn focus_rides_the_tree_update() {
        let mut tree = FloemLogicalTree::new();
        let diff = tree.rebuild(
            ViewReport::new("window", 100).with_child(
                ViewReport::new("text_input", 1)
                    .with_text("Email")
                    .focused(),
            ),
        );
        assert_eq!(diff.update.focus, NodeId(1));
    }
}
