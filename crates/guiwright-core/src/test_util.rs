//! Shared synthetic AccessKit tree fixtures.
//!
//! These model the `TreeUpdate`s a Level-1 adapter emits (window → panel →
//! buttons/labels/inputs). They are used by core tests, the driver's mock
//! adapter, adapter tests and the benchmark suites so every layer exercises
//! the same reality.

use accesskit::{Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};

use crate::key::NodeKey;

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
    /// The checked-state trio (checked / unchecked / mixed, §8 check
    /// vocabulary) — `Some` only for [`Fixture::with_checkboxes`]; the
    /// `buttons()` base stays byte-stable for the tests that pin its
    /// 8-node summary.
    pub checked_checkbox: Option<NodeId>,
    pub unchecked_checkbox: Option<NodeId>,
    pub mixed_checkbox: Option<NodeId>,
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
            checked_checkbox: None,
            unchecked_checkbox: None,
            mixed_checkbox: None,
            screen_size: (480.0, 240.0),
        }
    }

    /// The checked-state fixture (§8 check vocabulary): the `buttons()`
    /// base plus a checkbox trio under the group —
    ///
    /// ```text
    /// group (1)
    /// ├── … the six buttons()/label/input nodes …
    /// ├── checkbox "Notifications"       (9,  toggled True)
    /// ├── checkbox "Dark mode"           (10, toggled False)
    /// └── checkbox "Partially installed" (11, toggled Mixed)
    /// ```
    ///
    /// `NodeId(8)` stays reserved for [`Fixture::add_button_update`]; the
    /// trio lands above it so both updates compose on one fixture.
    pub fn with_checkboxes() -> Self {
        Self {
            expected_nodes: 11,
            checked_checkbox: Some(NodeId(9)),
            unchecked_checkbox: Some(NodeId(10)),
            mixed_checkbox: Some(NodeId(11)),
            ..Self::buttons()
        }
    }

    fn node(role: Role, name: Option<&str>) -> Node {
        let mut n = Node::new(role);
        if let Some(name) = name {
            n.set_label(name);
        }
        n
    }

    /// One checkbox row of the trio: bounded, labeled, tri-state stamped.
    fn checkbox(name: &str, toggled: Toggled) -> Node {
        let mut n = Self::node(Role::CheckBox, Some(name));
        n.set_toggled(toggled);
        n
    }

    /// Full initial update carrying the root `TreeInfo` (what egui emits on
    /// its first pass and what masonry emits per redraw).
    pub fn initial_update(&self) -> TreeUpdate {
        let mut window = Self::node(Role::Window, Some("Test App"));
        window.set_children([NodeId(1)]);
        let mut group = Self::node(Role::Group, None);
        let mut children = vec![
            self.save_button,
            self.cancel_button,
            self.welcome_label,
            self.email_input,
            self.hidden_button,
            self.disabled_button,
        ];
        // The checked-state trio rides the same group when present
        // (`with_checkboxes`); `buttons()` keeps its 8-node shape.
        let mut checkboxes: Vec<(NodeId, Node)> = Vec::new();
        for (slot, name, toggled) in [
            (self.checked_checkbox, "Notifications", Toggled::True),
            (self.unchecked_checkbox, "Dark mode", Toggled::False),
            (self.mixed_checkbox, "Partially installed", Toggled::Mixed),
        ] {
            if let Some(id) = slot {
                let mut node = Self::checkbox(name, toggled);
                let y0 = 204.0 + 40.0 * checkboxes.len() as f64;
                node.set_bounds(Rect {
                    x0: 16.0,
                    y0,
                    x1: 136.0,
                    y1: y0 + 32.0,
                });
                children.push(id);
                checkboxes.push((id, node));
            }
        }
        group.set_children(children);

        let mut save = Self::node(Role::Button, Some("Save"));
        save.set_bounds(Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });

        let mut cancel = Self::node(Role::Button, Some("Cancel"));
        cancel.set_bounds(Rect {
            x0: 16.0,
            y0: 56.0,
            x1: 136.0,
            y1: 88.0,
        });

        let mut label = Self::node(Role::Label, Some("Welcome, Ada"));
        label.set_bounds(Rect {
            x0: 16.0,
            y0: 96.0,
            x1: 216.0,
            y1: 120.0,
        });

        let mut input = Self::node(Role::TextInput, Some("Email"));
        input.set_value("ada@example.dev");
        input.set_bounds(Rect {
            x0: 16.0,
            y0: 128.0,
            x1: 256.0,
            y1: 156.0,
        });

        let mut hidden = Self::node(Role::Button, Some("Invisible"));
        hidden.set_hidden();

        let mut disabled = Self::node(Role::Button, Some("Confirm"));
        disabled.set_disabled();
        disabled.set_bounds(Rect {
            x0: 16.0,
            y0: 164.0,
            x1: 136.0,
            y1: 196.0,
        });

        let mut nodes = vec![
            (NodeId(0), window),
            (NodeId(1), group),
            (self.save_button, save),
            (self.cancel_button, cancel),
            (self.welcome_label, label),
            (self.email_input, input),
            (self.hidden_button, hidden),
            (self.disabled_button, disabled),
        ];
        nodes.extend(checkboxes);
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                root: NodeId(0),
                toolkit_name: Some("guiwright-fixture".to_string()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// Incremental update: re-state one checkbox of the trio with a new
    /// toggled state (the checked-check flip mid-wait). Fixture-scoped:
    /// `None` for ids outside the trio, so callers decide how to fail.
    pub fn checkbox_toggle_update(&self, node: NodeId, toggled: Toggled) -> Option<TreeUpdate> {
        let (name, y0) = [
            (self.checked_checkbox, "Notifications", 204.0),
            (self.unchecked_checkbox, "Dark mode", 244.0),
            (self.mixed_checkbox, "Partially installed", 284.0),
        ]
        .into_iter()
        .find(|(slot, _, _)| *slot == Some(node))
        .map(|(_, name, y0)| (name, y0))?;
        let mut box_node = Self::checkbox(name, toggled);
        box_node.set_bounds(Rect {
            x0: 16.0,
            y0,
            x1: 136.0,
            y1: y0 + 32.0,
        });
        Some(TreeUpdate {
            nodes: vec![(node, box_node)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        })
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

    /// Incremental update: rename Save → "Save All" (the `node_updated` path).
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

/// The M6 multi-window fixture (§16 v0.4): a two-window app modeled the way
/// AccessKit models multi-window — **grafted subtrees**. The root tree holds
/// one window node per window, each carrying a `tree_id` property naming the
/// subtree that holds the window's content; the consumer grafts the subtrees
/// under their window nodes.
///
/// ```text
/// ROOT tree                       W1 subtree ("Editor")        W2 subtree ("Preview")
/// app (0)                         content (1)                  content (1)
/// ├── window "Editor" (1) ──►W1   └── toolbar (4)              ├── button "Save" (2)
/// │                               └── button "Save" (2)        └── label "Preview text" (3)
/// └── window "Preview" (2) ──►W2
/// ```
///
/// Both windows deliberately reuse the same **local** `NodeId`s (the subtree
/// namespaces are independent): only the `TreeId` — the window identity —
/// tells the two "Save" buttons apart, which is exactly the discriminator a
/// window-scoped resolver must honor.
///
/// Incremental updates restate the interior toolbar node, never the subtree
/// root: the consumer clears a subtree root's graft parent whenever an
/// incremental update restates it (only full updates re-link the graft), so
/// window content hangs one level below the root — the shape real adapters
/// emit (window → panel → content).
#[derive(Clone, Debug)]
pub struct MultiWindowFixture {
    /// Subtree (window) ids.
    pub w1: TreeId,
    pub w2: TreeId,
    /// Graft node ids in the ROOT tree (`window "Editor"` / `window "Preview"`).
    pub w1_graft: u64,
    pub w2_graft: u64,
}

impl Default for MultiWindowFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiWindowFixture {
    pub fn new() -> Self {
        Self {
            w1: TreeId(accesskit::Uuid::from_u128(
                0xA600_0000_0000_0000_0000_0000_0000_0001,
            )),
            w2: TreeId(accesskit::Uuid::from_u128(
                0xA600_0000_0000_0000_0000_0000_0000_0002,
            )),
            w1_graft: 1,
            w2_graft: 2,
        }
    }

    fn graft(title: &str, subtree: TreeId) -> Node {
        let mut window = Node::new(Role::Window);
        window.set_label(title);
        window.set_tree_id(subtree);
        window
    }

    fn subtree_root(name: &str, children: &[NodeId]) -> Node {
        let mut content = Node::new(Role::Group);
        content.set_label(name);
        content.set_children(children.to_vec());
        content
    }

    fn save_button(x0: f64, y0: f64) -> Node {
        let mut save = Node::new(Role::Button);
        save.set_label("Save");
        save.set_bounds(Rect {
            x0,
            y0,
            x1: x0 + 120.0,
            y1: y0 + 32.0,
        });
        save
    }

    /// The editor window's toolbar — the interior node incremental updates
    /// restate (children changes ride the changed parent; the subtree root
    /// is never restated incrementally, see the type-level docs).
    fn toolbar(children: &[NodeId]) -> Node {
        let mut toolbar = Node::new(Role::Toolbar);
        toolbar.set_children(children.to_vec());
        toolbar
    }

    /// The three full updates a multi-window adapter emits on attach, in the
    /// order the consumer requires: the root tree (with the graft nodes)
    /// first, then one full update per grafted subtree.
    pub fn initial_updates(&self) -> Vec<TreeUpdate> {
        let mut app = Node::new(Role::Application);
        app.set_label("guiwright app");
        app.set_children([NodeId(self.w1_graft), NodeId(self.w2_graft)]);

        let w1_content = Self::subtree_root("editor content", &[NodeId(4)]);
        let w1_toolbar = Self::toolbar(&[NodeId(2)]);
        let w1_save = Self::save_button(16.0, 16.0);
        let w2_content = Self::subtree_root("preview content", &[NodeId(2), NodeId(3)]);
        let w2_save = Self::save_button(12.0, 416.0);
        let mut preview = Node::new(Role::Label);
        preview.set_label("Preview text");
        preview.set_bounds(Rect {
            x0: 12.0,
            y0: 456.0,
            x1: 212.0,
            y1: 480.0,
        });

        vec![
            TreeUpdate {
                nodes: vec![
                    (NodeId(0), app),
                    (NodeId(self.w1_graft), Self::graft("Editor", self.w1)),
                    (NodeId(self.w2_graft), Self::graft("Preview", self.w2)),
                ],
                tree: Some(TreeInfo {
                    root: NodeId(0),
                    toolkit_name: Some("guiwright-fixture".to_string()),
                    toolkit_version: None,
                }),
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            TreeUpdate {
                nodes: vec![
                    (NodeId(1), w1_content),
                    (NodeId(4), w1_toolbar),
                    (NodeId(2), w1_save),
                ],
                tree: Some(TreeInfo {
                    root: NodeId(1),
                    toolkit_name: Some("guiwright-fixture".to_string()),
                    toolkit_version: None,
                }),
                tree_id: self.w1,
                focus: NodeId(1),
            },
            TreeUpdate {
                nodes: vec![
                    (NodeId(1), w2_content),
                    (NodeId(2), w2_save),
                    (NodeId(3), preview),
                ],
                tree: Some(TreeInfo {
                    root: NodeId(1),
                    toolkit_name: Some("guiwright-fixture".to_string()),
                    toolkit_version: None,
                }),
                tree_id: self.w2,
                focus: NodeId(1),
            },
        ]
    }

    /// Incremental update: add a button to the Editor window's toolbar —
    /// the subtree root is untouched (graft link preserved).
    pub fn w1_add_button_update(&self, name: &str) -> TreeUpdate {
        let mut btn = Node::new(Role::Button);
        btn.set_label(name);
        btn.set_bounds(Rect {
            x0: 16.0,
            y0: 56.0,
            x1: 136.0,
            y1: 88.0,
        });
        let toolbar = Self::toolbar(&[NodeId(2), NodeId(5)]);
        TreeUpdate {
            nodes: vec![(NodeId(5), btn), (NodeId(4), toolbar)],
            tree: None,
            tree_id: self.w1,
            focus: NodeId(1),
        }
    }

    /// Incremental update: the app closes the Preview window — the graft
    /// node leaves the root tree (the consumer tears the subtree down with
    /// it). The Editor window is restated unchanged.
    pub fn close_w2_update(&self) -> TreeUpdate {
        let mut app = Node::new(Role::Application);
        app.set_label("guiwright app");
        app.set_children([NodeId(self.w1_graft)]);
        TreeUpdate {
            nodes: vec![(NodeId(0), app)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    /// The W1 "Save" button's identity (its window's `TreeId` is what makes it
    /// distinct from W2's).
    pub fn w1_save_key(&self) -> NodeKey {
        NodeKey::AccessKit {
            tree: self.w1,
            node: NodeId(2),
        }
    }

    /// The W2 "Save" button's identity.
    pub fn w2_save_key(&self) -> NodeKey {
        NodeKey::AccessKit {
            tree: self.w2,
            node: NodeId(2),
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
