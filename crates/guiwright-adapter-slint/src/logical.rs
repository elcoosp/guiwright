//! The slint kind → accesskit role registry (§4.4) plus the Level-2
//! logical-tree engine over testing-backend introspection.
//!
//! Slint's testing backend exposes the item tree (element kind, geometry,
//! text, enabled state; element *names* when built with
//! `SLINT_EMIT_DEBUG_INFO=1`, §11). This module synthesizes the
//! AccessKit-shaped update from it — the same Level-2 pattern as the iced
//! adapter (§4.6): minted `NodeId`s, stable per `(component, item)`, ABA
//! generation guards, test ids riding the diff (§8.1).
//!
//! Role mapping note (§4.4): the testing backend has **no AccessKit
//! tree**, so roles come from the element-kind registry — the same
//! "contributed role registry" the iced adapter documents.

use std::collections::HashMap;

use accesskit::{Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

use guiwright_driver::adapter::LogicalTreeDiff;

use crate::ids::SlintId;

/// What the glue side knows about one slint item after an introspection
/// pass (the testing backend's `search_api` vocabulary, §4.4).
#[derive(Clone, Debug)]
pub struct IntrospectionItem {
    pub kind: String,
    pub id: SlintId,
    /// Display text / accessible name (Text, Button caption, …).
    pub text: Option<String>,
    /// Editable content (`TextInput`, `LineEdit`).
    pub value: Option<String>,
    /// Element name from `SLINT_EMIT_DEBUG_INFO=1` builds — the natural
    /// test id (§4.4 `debug_info` build flag + §8.1).
    pub test_id: Option<String>,
    /// Visible bounds, window-relative physical px (§9).
    pub bounds: Option<[f64; 4]>,
    pub children: Vec<IntrospectionItem>,
    pub enabled: bool,
    pub focused: bool,
}

impl IntrospectionItem {
    pub fn new(kind: &str, id: SlintId) -> Self {
        Self {
            kind: kind.to_string(),
            id,
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

    pub fn with_child(mut self, child: IntrospectionItem) -> Self {
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

/// Slint element kind → accesskit role (§4.4; unmapped kinds become
/// `GenericContainer` so geometry-only nodes still resolve).
pub fn role_of_kind(kind: &str) -> Role {
    match kind {
        "Window" | "WindowItem" => Role::Window,
        "Button" => Role::Button,
        "CheckBox" => Role::CheckBox,
        "RadioButton" | "Radio" => Role::RadioButton,
        "Switch" => Role::Switch,
        "LineEdit" | "TextInput" | "TextEdit" => Role::TextInput,
        "SpinBox" => Role::NumberInput,
        "Slider" => Role::Slider,
        "ComboBox" => Role::ComboBox,
        "ListView" | "StandardListView" => Role::List,
        "StandardTableView" => Role::Table,
        "TabWidget" => Role::TabList,
        "Text" | "PrimaryText" | "SubtitleText" | "SmallText" | "BoldText" | "ErrorText"
        | "SuccessText" | "WarningText" => Role::TextRun,
        "Image" => Role::Image,
        "ProgressBar" => Role::ProgressIndicator,
        "HorizontalLayout" | "VerticalLayout" | "GridLayout" | "Rectangle" | "BorderRectangle"
        | "TouchArea" | "FocusScope" | "Flickable" | "ScrollView" | "RectangleNode" => {
            Role::GenericContainer
        }
        _ => Role::GenericContainer,
    }
}

#[derive(Debug)]
struct Slot {
    handle: u64,
    node_id: NodeId,
    role: Role,
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The Level-2 engine (§4.4): introspection pass → synthesized
/// [`LogicalTreeDiff`]. Full updates per pass — consumer-side diffing is
/// O(changes), so static UIs cost nothing extra (§15.2).
#[derive(Debug)]
pub struct SlintTreeBuilder {
    /// Adapter name — [`guiwright_core::NodeKey::Logical`] vocabulary and
    /// the synthesized tree's toolkit name.
    adapter: &'static str,
    slots: HashMap<u64, Slot>,
    generations: HashMap<u64, u32>,
    next_node_id: u64,
}

impl SlintTreeBuilder {
    pub fn new(adapter: &'static str) -> Self {
        Self {
            adapter,
            slots: HashMap::new(),
            generations: HashMap::new(),
            next_node_id: 0,
        }
    }

    /// Map a minted `NodeId` back to the slint item (action routing).
    pub fn item_of(&self, node: NodeId) -> Option<SlintId> {
        self.slots
            .values()
            .find(|slot| slot.node_id == node)
            .map(|slot| SlintId::from_handle(slot.handle))
    }

    /// The role a minted `NodeId` currently carries.
    pub fn role_of(&self, node: NodeId) -> Option<Role> {
        self.slots
            .values()
            .find(|slot| slot.node_id == node)
            .map(|slot| slot.role)
    }

    /// Current ABA generation of an item (§8.2).
    pub fn generation(&self, id: SlintId) -> u32 {
        self.generations.get(&id.to_handle()).copied().unwrap_or(0)
    }

    /// Fold one introspection pass into a [`LogicalTreeDiff`].
    pub fn rebuild(&mut self, root: &IntrospectionItem) -> LogicalTreeDiff {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        let mut live: HashMap<u64, Slot> = HashMap::new();
        let mut focus: Option<NodeId> = None;
        let mut removed: Vec<u64> = Vec::new();
        let mut test_ids: Vec<(u64, String)> = Vec::new();

        let root_node_id = self.walk(root, &mut nodes, &mut live, &mut focus, &mut test_ids);

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
        item: &IntrospectionItem,
        nodes: &mut Vec<(NodeId, Node)>,
        live: &mut HashMap<u64, Slot>,
        focus: &mut Option<NodeId>,
        test_ids: &mut Vec<(u64, String)>,
    ) -> NodeId {
        let handle = item.id.to_handle();
        let was_live = self.slots.contains_key(&handle);
        let node_id = if was_live {
            self.slots[&handle].node_id
        } else {
            self.mint()
        };

        let role = role_of_kind(&item.kind);
        let mut node = Node::new(role);
        if let Some(text) = &item.text {
            node.set_label(normalize(text));
        }
        if let Some(value) = &item.value {
            node.set_value(normalize(value));
        }
        if let Some([x0, y0, x1, y1]) = item.bounds {
            node.set_bounds(Rect { x0, y0, x1, y1 });
        }
        if !item.enabled {
            node.set_disabled();
        }
        if item.focused {
            *focus = Some(node_id);
        }
        if let Some(test_id) = &item.test_id {
            test_ids.push((node_id.0, test_id.clone()));
        }

        let mut child_ids = Vec::with_capacity(item.children.len());
        for child in &item.children {
            child_ids.push(self.walk(child, nodes, live, focus, test_ids));
        }
        node.set_children(child_ids);

        nodes.push((node_id, node));
        live.insert(
            handle,
            Slot {
                handle,
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

    fn sample_window() -> IntrospectionItem {
        IntrospectionItem::new("Window", SlintId::new(0, 0))
            .with_text("Demo")
            .with_test_id("win-main")
            .with_child(
                IntrospectionItem::new("Button", SlintId::new(0, 1))
                    .with_text("Save")
                    .with_test_id("btn-save")
                    .with_bounds(16.0, 16.0, 136.0, 48.0),
            )
            .with_child(
                IntrospectionItem::new("LineEdit", SlintId::new(0, 2))
                    .with_text("Email")
                    .with_value("ada@example.dev")
                    .with_bounds(16.0, 56.0, 256.0, 84.0),
            )
            .with_child(
                IntrospectionItem::new("Rectangle", SlintId::new(0, 3)).with_child(
                    IntrospectionItem::new("Text", SlintId::new(0, 4)).with_text("Welcome, Ada"),
                ),
            )
    }

    #[test]
    fn synthesizes_roles_labels_and_test_ids() {
        let mut builder = SlintTreeBuilder::new("slint");
        let diff = builder.rebuild(&sample_window());

        // Roles: window → button → text_input → container → text.
        let roles: Vec<Role> = diff.update.nodes.iter().map(|(_, n)| n.role()).collect();
        assert!(roles.contains(&Role::Window));
        assert!(roles.contains(&Role::Button));
        assert!(roles.contains(&Role::TextInput));
        assert!(roles.contains(&Role::GenericContainer));

        // Test ids ride the diff (session registers them in the O(1) hash).
        let ids: Vec<&String> = diff.test_ids.iter().map(|(_, id)| id).collect();
        assert!(ids.contains(&&"win-main".to_string()));
        assert!(ids.contains(&&"btn-save".to_string()));

        // Labels/values.
        let (save_id, save) = diff
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Button && n.label().is_some())
            .expect("save button");
        assert_eq!(save.label(), Some("Save"));
        assert!(save.bounds().is_some());
        let _ = save_id;
    }

    #[test]
    fn identity_is_stable_and_actions_route_back() {
        let mut builder = SlintTreeBuilder::new("slint");
        let first = builder.rebuild(&sample_window());
        let btn_handle = SlintId::new(0, 1).to_handle();
        let minted = first
            .update
            .nodes
            .iter()
            .find(|(id, _)| builder.item_of(*id) == Some(SlintId::new(0, 1)))
            .map(|(id, _)| *id)
            .expect("button minted");

        // A second pass keeps the binding (no re-mint).
        let second = builder.rebuild(&sample_window());
        let minted_again = second
            .update
            .nodes
            .iter()
            .find(|(id, _)| builder.item_of(*id) == Some(SlintId::new(0, 1)))
            .map(|(id, _)| *id)
            .expect("button minted again");
        assert_eq!(minted, minted_again, "stable identity across passes");
        assert_eq!(builder.generation(SlintId::new(0, 1)), 0);

        let _ = btn_handle;
    }

    #[test]
    fn removal_bumps_generation_aba_guard() {
        let mut builder = SlintTreeBuilder::new("slint");
        builder.rebuild(&sample_window());
        assert_eq!(builder.generation(SlintId::new(0, 1)), 0);

        // The button leaves the tree.
        let shrunk = IntrospectionItem::new("Window", SlintId::new(0, 0));
        let diff = builder.rebuild(&shrunk);
        assert!(diff.removed.contains(&SlintId::new(0, 1).to_handle()));
        assert_eq!(
            builder.generation(SlintId::new(0, 1)),
            1,
            "recycled ids re-mint fresh (§8.2)"
        );

        // It returns: fresh NodeId (no stale aliasing).
        let back = builder.rebuild(&sample_window());
        assert_eq!(builder.generation(SlintId::new(0, 1)), 1);
        let _ = back;
    }

    #[test]
    fn focus_rides_the_update() {
        let mut builder = SlintTreeBuilder::new("slint");
        let item = IntrospectionItem::new("Window", SlintId::new(0, 0))
            .with_child(IntrospectionItem::new("LineEdit", SlintId::new(0, 1)).focused());
        let diff = builder.rebuild(&item);
        // Focus lands on the LineEdit's minted id, not the window root.
        let minted_edit = diff
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::TextInput)
            .map(|(id, _)| *id)
            .expect("text input minted");
        assert_eq!(diff.update.focus, minted_edit);
    }

    #[test]
    fn role_registry_covers_the_inventory_kinds() {
        for (kind, expected) in [
            ("Button", Role::Button),
            ("CheckBox", Role::CheckBox),
            ("LineEdit", Role::TextInput),
            ("Slider", Role::Slider),
            ("ComboBox", Role::ComboBox),
            ("Window", Role::Window),
            ("TouchArea", Role::GenericContainer),
            ("WhateverNewElement", Role::GenericContainer),
        ] {
            assert_eq!(role_of_kind(kind), expected, "{kind}");
        }
    }
}
