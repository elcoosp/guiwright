//! Shared synthetic AccessKit tree fixtures.
//!
//! These model the `TreeUpdate`s a Level-1 adapter emits (window → panel →
//! buttons/labels/inputs). They are used by core tests, the driver's mock
//! adapter, adapter tests and the benchmark suites so every layer exercises
//! the same reality.

use accesskit::{Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

/// A fixture definition: builds both the initial full update and scripted
/// incremental updates against it.
#[derive(Clone, Debug)]
pub struct Fixture {
    /// Node ids are stable: 0 = window root.
    pub expected_nodes: usize,
    pub save_button: NodeId,
    pub cancel_button: NodeId,
    pub welcome_label: NodeId,
    pub email_input: NodeId,
    pub hidden_button: NodeId,
    pub disabled_button: NodeId,
    pub screen_size: (f64, f64),
}

impl Default for Fixture {
    fn default() -> Self {
        Self::buttons()
    }
}

impl Fixture {
    /// The canonical M0 fixture:
    ///
    /// ```text
    /// window (0)
    /// └── group (1)
    ///     ├── button "Save"      (2)   @ 16,16 120×32
    ///     ├── button "Cancel"    (3)   @ 16,56 120×32
    ///     ├── label  "Welcome, Ada" (4) @ 16,96 200×24
    ///     ├── text_input value="ada@example.dev" (5) @ 16,128 240×28
    ///     ├── button "Invisible" (6, hidden)
    ///     └── button "Confirm"   (7, disabled)
    /// ```
    pub fn buttons() -> Self {
        Self {
            expected_nodes: 8,
            save_button: NodeId(2),
            cancel_button: NodeId(3),
            welcome_label: NodeId(4),
            email_input: NodeId(5),
            hidden_button: NodeId(6),
            disabled_button: NodeId(7),
            screen_size: (480.0, 240.0),
        }
    }

    fn node(id: NodeId, role: Role, name: Option<&str>) -> Node {
        let mut n = Node::new(role);
        if let Some(name) = name {
            n.set_label(name);
        }
        let _ = id;
        n
    }

    /// Full initial update carrying the root `TreeInfo` (what egui emits on
    /// its first pass and what masonry emits per redraw).
    pub fn initial_update(&self) -> TreeUpdate {
        let mut window = Self::node(NodeId(0), Role::Window, Some("Test App"));
        window.set_children([NodeId(1)]);
        let mut group = Self::node(NodeId(1), Role::Group, None);
        group.set_children([
            self.save_button,
            self.cancel_button,
            self.welcome_label,
            self.email_input,
            self.hidden_button,
            self.disabled_button,
        ]);

        let mut save = Self::node(self.save_button, Role::Button, Some("Save"));
        save.set_bounds(Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });

        let mut cancel = Self::node(self.cancel_button, Role::Button, Some("Cancel"));
        cancel.set_bounds(Rect {
            x0: 16.0,
            y0: 56.0,
            x1: 136.0,
            y1: 88.0,
        });

        let mut label = Self::node(self.welcome_label, Role::Label, Some("Welcome, Ada"));
        label.set_bounds(Rect {
            x0: 16.0,
            y0: 96.0,
            x1: 216.0,
            y1: 120.0,
        });

        let mut input = Self::node(self.email_input, Role::TextInput, Some("Email"));
        input.set_value("ada@example.dev");
        input.set_bounds(Rect {
            x0: 16.0,
            y0: 128.0,
            x1: 256.0,
            y1: 156.0,
        });

        let mut hidden = Self::node(self.hidden_button, Role::Button, Some("Invisible"));
        hidden.set_hidden();

        let mut disabled = Self::node(self.disabled_button, Role::Button, Some("Confirm"));
        disabled.set_disabled();
        disabled.set_bounds(Rect {
            x0: 16.0,
            y0: 164.0,
            x1: 136.0,
            y1: 196.0,
        });

        TreeUpdate {
            nodes: vec![
                (NodeId(0), window),
                (NodeId(1), group),
                (self.save_button, save),
                (self.cancel_button, cancel),
                (self.welcome_label, label),
                (self.email_input, input),
                (self.hidden_button, hidden),
                (self.disabled_button, disabled),
            ],
            tree: Some(TreeInfo {
                root: NodeId(0),
                toolkit_name: Some("guiwright-fixture".to_string()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: add `button "Export"` under the group (§15.2
    /// "typ. < 2 % nodes changed").
    pub fn add_button_update(&self, name: &str) -> TreeUpdate {
        let new_id = NodeId(8);
        let mut btn = Node::new(Role::Button);
        btn.set_label(name);
        btn.set_bounds(Rect {
            x0: 16.0,
            y0: 200.0,
            x1: 136.0,
            y1: 232.0,
        });
        let mut group = Node::new(Role::Group);
        group.set_children([
            self.save_button,
            self.cancel_button,
            self.welcome_label,
            self.email_input,
            self.hidden_button,
            self.disabled_button,
            new_id,
        ]);
        TreeUpdate {
            nodes: vec![(new_id, btn), (NodeId(1), group)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: remove the Save button (parent restated without
    /// it, matching how real adapters express removals).
    pub fn remove_save_update(&self) -> TreeUpdate {
        let mut group = Node::new(Role::Group);
        group.set_children([
            self.cancel_button,
            self.welcome_label,
            self.email_input,
            self.hidden_button,
            self.disabled_button,
        ]);
        TreeUpdate {
            nodes: vec![(NodeId(1), group)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: rename Save → "Save All" (node_updated path).
    pub fn rename_save_update(&self) -> TreeUpdate {
        let mut save = Node::new(Role::Button);
        save.set_label("Save All");
        save.set_bounds(Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });
        TreeUpdate {
            nodes: vec![(self.save_button, save)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: disable the Save button (actionability diff).
    pub fn disable_save_update(&self) -> TreeUpdate {
        let mut save = Node::new(Role::Button);
        save.set_label("Save");
        save.set_disabled();
        save.set_bounds(Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });
        TreeUpdate {
            nodes: vec![(self.save_button, save)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: move the Save button to the far corner (stability
    /// checks see a changed rect, then an unchanged one).
    pub fn move_save_update(&self, x: f64, y: f64) -> TreeUpdate {
        let mut save = Node::new(Role::Button);
        save.set_label("Save");
        save.set_bounds(Rect {
            x0: x,
            y0: y,
            x1: x + 120.0,
            y1: y + 32.0,
        });
        TreeUpdate {
            nodes: vec![(self.save_button, save)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Update that hides the Save button (visible-check flip).
    pub fn hide_save_update(&self) -> TreeUpdate {
        let mut save = Node::new(Role::Button);
        save.set_label("Save");
        save.set_hidden();
        TreeUpdate {
            nodes: vec![(self.save_button, save)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: the old Save (id 2) leaves the tree and a *new*
    /// button named "Save" appears under a different id (9). Exercises the
    /// identity-change / ABA path in waiters (§8.2).
    pub fn replace_save_update(&self) -> TreeUpdate {
        let mut replacement = Node::new(Role::Button);
        replacement.set_label("Save");
        replacement.set_bounds(Rect {
            x0: 150.0,
            y0: 16.0,
            x1: 270.0,
            y1: 48.0,
        });
        let mut group = Node::new(Role::Group);
        group.set_children([
            NodeId(9),
            self.cancel_button,
            self.welcome_label,
            self.email_input,
            self.hidden_button,
            self.disabled_button,
        ]);
        TreeUpdate {
            nodes: vec![(NodeId(9), replacement), (NodeId(1), group)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_ids_are_distinct() {
        let f = Fixture::buttons();
        let ids = [
            f.save_button,
            f.cancel_button,
            f.welcome_label,
            f.email_input,
            f.hidden_button,
            f.disabled_button,
        ];
        let mut sorted = ids;
        sorted.sort_by_key(|id| id.0);
        for w in sorted.windows(2) {
            assert_ne!(w[0], w[1]);
        }
    }
}
