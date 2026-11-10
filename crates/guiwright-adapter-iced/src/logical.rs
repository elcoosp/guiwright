//! The Level-2 tree state machine: [`WidgetReport`]s in, a synthesized
//! AccessKit-shaped [`LogicalTreeDiff`](guiwright_driver::adapter::LogicalTreeDiff)
//! out.
//!
//! Identity model (§6.1 "Level-2 framework identities are normalized into
//! core's handles"): the framework's widget handle (iced `widget::Id`
//! inner, floem `ViewId` index, or an adapter-minted counter) is bound to
//! a minted AccessKit `NodeId` the first time it appears. The consumer
//! tree then keys everything on that `NodeId` — selectors, actionability,
//! `expect` — and actions arrive with exactly the minted id, which the
//! harness maps back to the widget handle (ABA-safe: a recycled handle
//! that re-enters the tree gets a *fresh* `NodeId`, and the driver's
//! generation counters catch stale handles, §8.2).
//!
//! Updates are full per pass (the egui pattern, §4.1): the consumer diffs
//! at O(changes) so full updates cost nothing when nothing changed.

use std::collections::HashMap;

use accesskit::{Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

use guiwright_driver::adapter::LogicalTreeDiff;

use crate::registry::RoleRegistry;

/// What the framework-facing side knows about one widget after a layout
/// pass — the neutral vocabulary between the harness and this engine.
#[derive(Clone, Debug)]
pub struct WidgetReport {
    /// Registry key (`"button"`, `"text_input"`, `"container"`, …, §4.6).
    pub kind: String,
    /// Framework widget handle (iced `widget::Id`, floem `ViewId`, …).
    pub handle: u64,
    /// Text content — the accessible-name fallback (§4.6 "widget `Id`s
    /// are optional → text/geometry fallbacks matter").
    pub text: Option<String>,
    /// Editable content (text inputs).
    pub value: Option<String>,
    /// Explicit test id (`getByTestId` — first-class, §8.1).
    pub test_id: Option<String>,
    /// Visible bounds, window-relative physical px (AccessKit convention,
    /// §9 "AccessKit bounds are already physical").
    pub bounds: Option<[f64; 4]>,
    pub children: Vec<WidgetReport>,
    pub enabled: bool,
    pub focused: bool,
}

impl WidgetReport {
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

    pub fn with_bounds(mut self, x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        self.bounds = Some([x0, y0, x1, y1]);
        self
    }

    pub fn with_child(mut self, child: WidgetReport) -> Self {
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

/// Text normalization lives here (the adapter owns labels/values); kept
/// separate so the engine is testable without core's selector plumbing.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The engine. Rebuilt per pass; owns the handle↔`NodeId` binding.
#[derive(Debug)]
pub struct LogicalTree {
    registry: RoleRegistry,
    /// Adapter name — the [`NodeKey::Logical`](guiwright_core::NodeKey)
    /// vocabulary and the synthesized tree's toolkit name.
    adapter: &'static str,
    slots: HashMap<u64, Slot>,
    /// Handle → generation; bumps when a handle leaves the tree so a
    /// recycled framework id never reuses a stale binding (§8.2).
    generations: HashMap<u64, u32>,
    next_node_id: u64,
}

impl LogicalTree {
    pub fn new(adapter: &'static str) -> Self {
        Self::with_registry(adapter, RoleRegistry::new())
    }

    pub fn with_registry(adapter: &'static str, registry: RoleRegistry) -> Self {
        Self {
            registry,
            adapter,
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

    /// Map a minted `NodeId` back to the widget handle (action routing).
    pub fn handle_of(&self, node: NodeId) -> Option<u64> {
        self.slots
            .values()
            .find(|slot| slot.node_id == node)
            .map(|slot| slot.handle)
    }

    /// The registry role a minted `NodeId` currently carries (action
    /// routing needs the role to pick the widget op).
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

    /// Fold one pass's widget reports into a [`LogicalTreeDiff`]: the full
    /// synthesized update (consumer-side diffing is O(changes), §15.2)
    /// plus the removed handle list for adapter bookkeeping.
    pub fn rebuild(&mut self, root: &WidgetReport) -> LogicalTreeDiff {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        let mut live: HashMap<u64, Slot> = HashMap::new();
        let mut focus: Option<NodeId> = None;
        let mut removed: Vec<u64> = Vec::new();
        let mut test_ids: Vec<(u64, String)> = Vec::new();

        let root_node_id = self.walk(root, &mut nodes, &mut live, &mut focus, &mut test_ids);

        // Handles that disappeared since the last pass — bump their
        // generation so recycled ids never alias (§8.2).
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
        report: &WidgetReport,
        nodes: &mut Vec<(NodeId, Node)>,
        live: &mut HashMap<u64, Slot>,
        focus: &mut Option<NodeId>,
        test_ids: &mut Vec<(u64, String)>,
    ) -> NodeId {
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
        // ...and remembered for action routing (`role_of`).
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
            // accesskit 0.25 carries focus on the `TreeUpdate.focus` field
            // (there is no per-node focused property) — the diff-level
            // focus is what the consumer tree exposes (§8.3 focused check).
            *focus = Some(node_id);
        }
        // Test ids ride the diff (not the tree): the session registers them
        // in the O(1) hash (§8.1) after applying the update.
        if let Some(test_id) = &report.test_id {
            test_ids.push((node_id.0, test_id.clone()));
        }

        let mut child_ids = Vec::with_capacity(report.children.len());
        for child in &report.children {
            child_ids.push(self.walk(child, nodes, live, focus, test_ids));
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
        node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::Role;

    fn sample_app() -> WidgetReport {
        WidgetReport::new("window", 100)
            .with_text("Demo")
            .with_child(
                WidgetReport::new("button", 1)
                    .with_text("Save")
                    .with_test_id("btn-save")
                    .with_bounds(16.0, 16.0, 136.0, 48.0),
            )
            .with_child(
                WidgetReport::new("text_input", 2)
                    .with_text("Email")
                    .with_value("ada@example.dev")
                    .with_bounds(16.0, 56.0, 256.0, 84.0),
            )
            .with_child(
                WidgetReport::new("button", 3)
                    .with_text("Confirm")
                    .disabled(),
            )
    }

    #[test]
    fn synthesizes_role_label_bounds_semantics() {
        let mut tree = LogicalTree::new("iced");
        let diff = tree.rebuild(&sample_app());
        assert_eq!(diff.adapter, "iced");
        assert!(diff.changed);
        assert_eq!(diff.removed.len(), 0);

        let update = &diff.update;
        assert_eq!(update.tree_id, TreeId::ROOT);
        let info = update.tree.as_ref().expect("TreeInfo");
        assert_eq!(info.root, NodeId(0), "first mint is the root");
        assert_eq!(update.nodes.len(), 4);

        // Roles come from the registry; labels from text; values from value.
        let (id, node) = update
            .nodes
            .iter()
            .find(|(id, _)| id.0 == 1)
            .expect("button node");
        let _ = id;
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.label(), Some("Save"));
        let (_, input) = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::TextInput)
            .expect("input");
        assert_eq!(input.value(), Some("ada@example.dev"));
        let (_, confirm) = update
            .nodes
            .iter()
            .find(|(id, _)| id.0 == 3)
            .expect("confirm");
        assert!(confirm.is_disabled());
    }

    #[test]
    fn handles_keep_stable_minted_ids_across_passes() {
        let mut tree = LogicalTree::new("iced");
        let first = tree.rebuild(&sample_app());
        let save_first = first
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Save"))
            .expect("save")
            .0;

        let second = tree.rebuild(&sample_app());
        let save_second = second
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Save"))
            .expect("save")
            .0;
        assert_eq!(save_first, save_second, "stable handle → stable NodeId");
        assert_eq!(tree.handle_of(save_second), Some(1), "reverse mapping");
    }

    #[test]
    fn removed_handles_report_and_never_realias() {
        let mut tree = LogicalTree::new("iced");
        tree.rebuild(&sample_app());
        let save_id = tree.handle_of(NodeId(1));

        // Pass 2: the Save button disappears.
        let mut without_save = WidgetReport::new("window", 100).with_text("Demo");
        without_save.children = vec![
            WidgetReport::new("text_input", 2)
                .with_text("Email")
                .with_value("ada@example.dev"),
            WidgetReport::new("button", 3).with_text("Confirm"),
        ];
        let diff = tree.rebuild(&without_save);
        assert_eq!(diff.removed, vec![1], "handle 1 left");

        // Pass 3: handle 1 returns (framework recycled the id) → it must
        // re-mint a FRESH NodeId and the old binding must be gone.
        let mut with_recycled = WidgetReport::new("window", 100).with_text("Demo");
        with_recycled.children = vec![
            WidgetReport::new("text", 1).with_text("Recycled id, different widget"),
            WidgetReport::new("text_input", 2).with_text("Email"),
        ];
        let diff = tree.rebuild(&with_recycled);
        let recycled_node = diff
            .update
            .nodes
            .iter()
            .find(|(id, _)| *id != diff.update.tree.as_ref().expect("info").root)
            .map(|(id, _)| *id)
            .expect("recycled node");
        assert_ne!(save_id, Some(recycled_node.0), "recycled handle re-mints");
        assert_eq!(tree.generation(1), 1, "one removal recorded");
    }

    #[test]
    fn focus_follows_the_focused_report() {
        let mut tree = LogicalTree::new("iced");
        let mut root = WidgetReport::new("window", 100);
        root.children = vec![
            WidgetReport::new("text_input", 2).with_text("Email"),
            WidgetReport::new("text_input", 5)
                .with_text("Name")
                .focused(),
        ];
        let diff = tree.rebuild(&root);
        let focus = diff.update.focus;
        assert_ne!(
            focus,
            diff.update.tree.expect("info").root,
            "not the window"
        );
        assert_eq!(tree.handle_of(focus), Some(5), "focus maps to handle 5");
    }

    #[test]
    fn custom_registry_kinds_flow_through() {
        let mut reg = RoleRegistry::new();
        reg.register("star_rating", Role::Status);
        let mut tree = LogicalTree::with_registry("iced", reg);
        let root = WidgetReport::new("window", 100)
            .with_child(WidgetReport::new("star_rating", 7).with_text("4 of 5"));
        let diff = tree.rebuild(&root);
        let star = diff
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("4 of 5"))
            .expect("star rating node");
        assert_eq!(star.1.role(), Role::Status);
    }

    #[test]
    fn test_ids_ride_the_diff_and_survive_whitespace() {
        let mut tree = LogicalTree::new("iced");
        let root = WidgetReport::new("window", 100).with_child(
            WidgetReport::new("button", 1)
                .with_text("  Save   it  ")
                .with_test_id("btn-save"),
        );
        let diff = tree.rebuild(&root);
        assert_eq!(diff.test_ids, vec![(1, "btn-save".to_string())]);
        let (_, save) = diff
            .update
            .nodes
            .iter()
            .find(|(id, _)| id.0 == 1)
            .expect("button");
        assert_eq!(save.label(), Some("Save it"), "normalized label");
    }
}
