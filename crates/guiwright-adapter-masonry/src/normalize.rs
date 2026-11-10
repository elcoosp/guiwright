//! accesskit version normalization (masonry 0.21 → core 0.25).
//!
//! The architecture (§5.2 dependency rule) makes `accesskit` the one
//! universal substrate — but framework releases pin different `accesskit`
//! majors (`egui` 0.37 → accesskit 0.25; masonry 0.4.0 → accesskit 0.21).
//! Adapters normalize **their** framework's tree updates into the core
//! dialect, exactly like Level-2 adapters normalize their identities into
//! [`NodeKey`](guiwright_core::NodeKey).
//!
//! Scope (M0): the property subset the locator engine and actionability
//! checks consume — role, label, `labelled_by`, value, description, disabled,
//! hidden, bounds, transform, children, actions. New accesskit properties
//! flow through as soon as masonry bumps its accesskit dependency (the
//! shim is deleted then); unknown roles degrade to `Unknown` so newer
//! toolkits never break older drivers.

use accesskit as ak25;
use masonry_core::accesskit as ak21;

/// Accessible-name-from-content fixup for masonry 0.4.
///
/// masonry 0.4's accessibility pass emits interactive widgets (buttons,
/// text areas…) with an **empty** label and their text in a descendant
/// `Label`/`TextRun` node — the recon's "kittest integration is an upstream
/// TODO" gap (§4.2). Playwright computes a control's accessible name from
/// its content, so this pass propagates the first non-empty descendant
/// text into the node's label. Deleted when masonry emits names natively.
pub fn fixup_accessible_names(nodes: &mut [(ak21::NodeId, ak21::Node)]) {
    // Recurse from `id` down the a11y tree (mirrors the widget tree) for the
    // first non-empty descendant text (§4.2 name-from-content).
    fn descendant_text(
        id: ak21::NodeId,
        children_of: &std::collections::HashMap<ak21::NodeId, Vec<ak21::NodeId>>,
        text_of: &std::collections::HashMap<ak21::NodeId, String>,
    ) -> Option<String> {
        for child in children_of.get(&id)? {
            if let Some(text) = text_of.get(child) {
                if !text.trim().is_empty() {
                    return Some(text.clone());
                }
            }
            if let Some(found) = descendant_text(*child, children_of, text_of) {
                return Some(found);
            }
        }
        None
    }

    // children map for descendant walks (a11y tree mirrors the widget tree).
    let children_of: std::collections::HashMap<ak21::NodeId, Vec<ak21::NodeId>> = nodes
        .iter()
        .map(|(id, node)| (*id, node.children().to_vec()))
        .collect();
    let text_of: std::collections::HashMap<ak21::NodeId, String> = nodes
        .iter()
        .map(|(id, node)| (*id, node.value().map(String::from).unwrap_or_default()))
        .collect();

    let ids: Vec<ak21::NodeId> = nodes.iter().map(|(id, _)| *id).collect();
    let mut labels: std::collections::HashMap<ak21::NodeId, Option<String>> = nodes
        .iter()
        .map(|(id, node)| (*id, node.label().map(String::from)))
        .collect();

    for id in ids {
        let is_empty = labels
            .get(&id)
            .and_then(|l| l.as_deref())
            .is_none_or(|l| l.trim().is_empty());
        if !is_empty {
            continue;
        }
        if let Some(text) = descendant_text(id, &children_of, &text_of) {
            labels.insert(id, Some(text));
        }
    }
    for (id, node) in nodes.iter_mut() {
        if let Some(label) = labels.get(id) {
            match label {
                Some(text) if !text.trim().is_empty() => node.set_label(text.clone()),
                Some(_) | None => node.clear_label(),
            }
        }
    }
}

/// Convert a full `TreeUpdate` (0.21) into the core dialect (0.25).
/// accesskit 0.21 has no `tree_id` field (it is ROOT-only); 0.25 carries it
/// explicitly and the core consumer requires `TreeId::ROOT` at init.
pub fn normalize_update(update: ak21::TreeUpdate) -> ak25::TreeUpdate {
    ak25::TreeUpdate {
        nodes: update
            .nodes
            .into_iter()
            .map(|(id, node)| (ak25::NodeId(id.0), normalize_node(&node)))
            .collect(),
        tree: update.tree.map(|tree| ak25::TreeInfo {
            root: ak25::NodeId(tree.root.0),
            toolkit_name: tree.toolkit_name,
            toolkit_version: tree.toolkit_version,
        }),
        tree_id: ak25::TreeId::ROOT,
        focus: ak25::NodeId(update.focus.0),
    }
}

fn normalize_node(node: &ak21::Node) -> ak25::Node {
    // Actions (the semantic surface of §8.4). The closed set is identical
    // between 0.21 and 0.25.
    const ACTIONS: [ak21::Action; 22] = [
        ak21::Action::Click,
        ak21::Action::Focus,
        ak21::Action::Blur,
        ak21::Action::Collapse,
        ak21::Action::Expand,
        ak21::Action::CustomAction,
        ak21::Action::Decrement,
        ak21::Action::Increment,
        ak21::Action::HideTooltip,
        ak21::Action::ShowTooltip,
        ak21::Action::ReplaceSelectedText,
        ak21::Action::ScrollDown,
        ak21::Action::ScrollLeft,
        ak21::Action::ScrollRight,
        ak21::Action::ScrollUp,
        ak21::Action::ScrollIntoView,
        ak21::Action::ScrollToPoint,
        ak21::Action::SetScrollOffset,
        ak21::Action::SetSequentialFocusNavigationStartingPoint,
        ak21::Action::SetTextSelection,
        ak21::Action::SetValue,
        ak21::Action::ShowContextMenu,
    ];
    let mut out = ak25::Node::new(ak25::Role::Unknown);

    // Role (matched by stable name; unknown roles degrade gracefully).
    let role_name = format!("{:?}", node.role());
    out.set_role(role_from_name(&role_name));

    // Scalar text properties (0.21 getters return Option<&str>).
    if let Some(label) = node.label() {
        out.set_label(label);
    }
    if let Some(value) = node.value() {
        out.set_value(value);
    }
    if let Some(description) = node.description() {
        out.set_description(description);
    }

    // Flags.
    if node.is_disabled() {
        out.set_disabled();
    }
    if node.is_hidden() {
        out.set_hidden();
    }

    // Structure.
    let children: Vec<ak25::NodeId> = node
        .children()
        .iter()
        .map(|id| ak25::NodeId(id.0))
        .collect();
    out.set_children(children);
    let labelled_by: Vec<ak25::NodeId> = node
        .labelled_by()
        .iter()
        .map(|id| ak25::NodeId(id.0))
        .collect();
    if !labelled_by.is_empty() {
        out.set_labelled_by(labelled_by);
    }

    // Geometry (own geometry types in 0.21; kurbo in 0.25 — same shapes).
    if let Some(bounds) = node.bounds() {
        out.set_bounds(ak25::Rect {
            x0: bounds.x0,
            y0: bounds.y0,
            x1: bounds.x1,
            y1: bounds.y1,
        });
    }
    if let Some(transform) = node.transform() {
        let coeffs = transform.as_coeffs();
        out.set_transform(ak25::Affine::new(coeffs));
    }

    // Actions (the semantic surface of §8.4). The closed set is identical
    // between 0.21 and 0.25.
    for action in ACTIONS {
        if node.supports_action(action) {
            out.add_action(translate_action(action));
        }
    }

    out
}

fn translate_action(action: ak21::Action) -> ak25::Action {
    // The Action set is identical between 0.21 and 0.25 (verified 2026-11).
    match action {
        ak21::Action::Click => ak25::Action::Click,
        ak21::Action::Focus => ak25::Action::Focus,
        ak21::Action::Blur => ak25::Action::Blur,
        ak21::Action::Collapse => ak25::Action::Collapse,
        ak21::Action::Expand => ak25::Action::Expand,
        ak21::Action::CustomAction => ak25::Action::CustomAction,
        ak21::Action::Decrement => ak25::Action::Decrement,
        ak21::Action::Increment => ak25::Action::Increment,
        ak21::Action::HideTooltip => ak25::Action::HideTooltip,
        ak21::Action::ShowTooltip => ak25::Action::ShowTooltip,
        ak21::Action::ReplaceSelectedText => ak25::Action::ReplaceSelectedText,
        ak21::Action::ScrollDown => ak25::Action::ScrollDown,
        ak21::Action::ScrollLeft => ak25::Action::ScrollLeft,
        ak21::Action::ScrollRight => ak25::Action::ScrollRight,
        ak21::Action::ScrollUp => ak25::Action::ScrollUp,
        ak21::Action::ScrollIntoView => ak25::Action::ScrollIntoView,
        ak21::Action::ScrollToPoint => ak25::Action::ScrollToPoint,
        ak21::Action::SetScrollOffset => ak25::Action::SetScrollOffset,
        ak21::Action::SetSequentialFocusNavigationStartingPoint => {
            ak25::Action::SetSequentialFocusNavigationStartingPoint
        }
        ak21::Action::SetTextSelection => ak25::Action::SetTextSelection,
        ak21::Action::SetValue => ak25::Action::SetValue,
        ak21::Action::ShowContextMenu => ak25::Action::ShowContextMenu,
    }
}

/// Role names are the Debug names of the closed set (§ convert.rs parity).
fn role_from_name(name: &str) -> ak25::Role {
    match name {
        "Window" => ak25::Role::Window,
        "Group" => ak25::Role::Group,
        "GenericContainer" => ak25::Role::GenericContainer,
        "Button" => ak25::Role::Button,
        "DefaultButton" => ak25::Role::DefaultButton,
        "Label" => ak25::Role::Label,
        "TextRun" => ak25::Role::TextRun,
        "TextInput" => ak25::Role::TextInput,
        "MultilineTextInput" => ak25::Role::MultilineTextInput,
        "SearchInput" => ak25::Role::SearchInput,
        "EmailInput" => ak25::Role::EmailInput,
        "NumberInput" => ak25::Role::NumberInput,
        "CheckBox" => ak25::Role::CheckBox,
        "Switch" => ak25::Role::Switch,
        "RadioButton" => ak25::Role::RadioButton,
        "List" => ak25::Role::List,
        "ListItem" => ak25::Role::ListItem,
        "ListBoxOption" => ak25::Role::ListBoxOption,
        "Menu" => ak25::Role::Menu,
        "MenuItem" => ak25::Role::MenuItem,
        "MenuListOption" => ak25::Role::MenuListOption,
        "Dialog" => ak25::Role::Dialog,
        "Pane" => ak25::Role::Pane,
        "Image" => ak25::Role::Image,
        "Link" => ak25::Role::Link,
        "Table" => ak25::Role::Table,
        "Row" => ak25::Role::Row,
        "Cell" => ak25::Role::Cell,
        "TreeItem" => ak25::Role::TreeItem,
        "Paragraph" => ak25::Role::Paragraph,
        "Tree" => ak25::Role::Tree,
        "Tab" => ak25::Role::Tab,
        "Slider" => ak25::Role::Slider,
        "ProgressIndicator" => ak25::Role::ProgressIndicator,
        _ => ak25::Role::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry_core::accesskit as ak21;

    #[test]
    fn node_properties_survive_normalization() {
        let mut src = ak21::Node::new(ak21::Role::Button);
        src.set_label("Save");
        src.set_bounds(ak21::Rect {
            x0: 10.0,
            y0: 20.0,
            x1: 110.0,
            y1: 52.0,
        });
        src.add_action(ak21::Action::Click);

        let out = normalize_node(&src);
        assert_eq!(out.role(), ak25::Role::Button);
        assert_eq!(out.label(), Some("Save"));
        let bounds = out.bounds().expect("bounds");
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (10.0, 20.0, 110.0, 52.0)
        );
        assert!(out.supports_action(ak25::Action::Click));
    }

    #[test]
    fn unknown_roles_degrade_gracefully() {
        // A role that exists in 0.21 but has no explicit mapping here.
        let src = ak21::Node::new(ak21::Role::Abbr);
        let out = normalize_node(&src);
        assert_eq!(out.role(), ak25::Role::Unknown);
    }

    #[test]
    fn update_structure_is_preserved() {
        let mut root = ak21::Node::new(ak21::Role::Window);
        root.set_children([ak21::NodeId(1)]);
        let mut child = ak21::Node::new(ak21::Role::Button);
        child.set_label("OK");
        let update = ak21::TreeUpdate {
            nodes: vec![(ak21::NodeId(0), root), (ak21::NodeId(1), child)],
            tree: Some(ak21::Tree::new(ak21::NodeId(0))),
            focus: ak21::NodeId(1),
        };
        let out = normalize_update(update);
        assert_eq!(out.focus, ak25::NodeId(1));
        assert_eq!(out.nodes.len(), 2);
        let tree = out.tree.expect("tree info");
        assert_eq!(tree.root, ak25::NodeId(0));
    }
}
