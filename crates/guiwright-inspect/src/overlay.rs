//! The overlay plan (§13 "Overlay: adapter-level composition").
//!
//! The overlay draws hover outlines + role/TestId badges from the current
//! tree indexes (§13). This module is the **framework-free plan**: the
//! adapter composites it onto the app's scene at present time (egui:
//! painter layer after the pass; masonry/freya: overlay layer; iced:
//! `Operation`-returned rects; slint: top-level overlay window).

use std::fmt::Write;

use guiwright_core::NodeKey;

/// Window-relative physical rect (AccessKit convention, §9).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayRect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl OverlayRect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn center(&self) -> (f64, f64) {
        (
            f64::midpoint(self.x0, self.x1),
            f64::midpoint(self.y0, self.y1),
        )
    }
}

/// One badge: role/TestId caption pinned to a point (§13 "badges").
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub text: String,
    pub at: (f64, f64),
}

/// The overlay composition for one frame. Adapters render it at present
/// time — the render-once invariant holds because the plan is *data*
/// (§15.4): producing it never triggers extra passes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayPlan {
    /// The hovered node's outline (§13 "hover outlines").
    pub hover: Option<OverlayRect>,
    /// Role/TestId badges, in document order.
    pub badges: Vec<Badge>,
}

impl OverlayPlan {
    /// Build the plan from node facts (the same vocabulary codegen
    /// records): outline the hovered node, badge everything with bounds.
    /// `interactive_only` restricts badges to interactive roles (buttons,
    /// inputs, …) — full-tree badges are for the deep-inspect mode.
    pub fn from_facts(
        facts: impl IntoIterator<Item = super::codegen::NodeFacts>,
        hover_key: Option<&NodeKey>,
        interactive_only: bool,
    ) -> Self {
        const INTERACTIVE: &[&str] = &[
            "Button",
            "CheckBox",
            "RadioButton",
            "TextInput",
            "MultilineTextInput",
            "SearchInput",
            "Slider",
            "Switch",
            "ComboBox",
            "Link",
            "Tab",
            "MenuItem",
            "MenuListOption",
            "ListBoxOption",
        ];

        let mut plan = Self::default();
        for fact in facts {
            let Some([x0, y0, x1, y1]) = fact.bounds else {
                continue;
            };
            let is_hover = hover_key.is_some_and(|k| *k == fact.key);
            if is_hover {
                plan.hover = Some(OverlayRect::new(x0, y0, x1, y1));
            }
            if interactive_only
                && !fact
                    .role
                    .as_deref()
                    .is_some_and(|r| INTERACTIVE.contains(&r))
            {
                continue;
            }
            let mut text = fact.role.clone().unwrap_or_else(|| "?".into());
            if let Some(id) = &fact.test_id {
                let _ = write!(text, " #{id}");
            } else if let Some(name) = &fact.name {
                let _ = write!(text, " \u{201c}{name}\u{201d}");
            }
            let (cx, cy) = OverlayRect::new(x0, y0, x1, y1).center();
            plan.badges.push(Badge { text, at: (cx, cy) });
        }
        plan
    }

    /// `true` when nothing would be drawn (adapters skip the layer).
    pub fn is_empty(&self) -> bool {
        self.hover.is_none() && self.badges.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::NodeFacts;

    fn fact(key: NodeKey, role: &str, name: &str, bounds: [f64; 4]) -> NodeFacts {
        NodeFacts {
            key,
            role: Some(role.into()),
            name: Some(name.into()),
            bounds: Some(bounds),
            ..Default::default()
        }
    }

    fn key(id: u64) -> NodeKey {
        NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: accesskit::NodeId(id),
        }
    }

    #[test]
    fn hover_outline_and_interactive_badges() {
        let facts = vec![
            fact(key(1), "Window", "Main", [0.0, 0.0, 800.0, 600.0]),
            fact(key(2), "Button", "Save", [10.0, 20.0, 90.0, 60.0]),
            fact(key(3), "TextInput", "Email", [10.0, 80.0, 200.0, 110.0]),
        ];
        let plan = OverlayPlan::from_facts(facts, Some(&key(2)), true);
        let hover = plan.hover.expect("hovered button outlined");
        assert_eq!(
            (hover.x0, hover.y0, hover.x1, hover.y1),
            (10.0, 20.0, 90.0, 60.0)
        );
        // Interactive-only: window badge dropped, button + input kept.
        assert_eq!(plan.badges.len(), 2);
        assert!(plan.badges[0].text.contains("Button"));
        assert!(
            plan.badges[0].text.contains("Save"),
            "{:?}",
            plan.badges[0].text
        );
        assert_eq!(plan.badges[0].at, (50.0, 40.0), "badge centered");
    }

    #[test]
    fn test_id_badge_beats_name() {
        let mut fact = fact(key(2), "Button", "Save", [10.0, 20.0, 90.0, 60.0]);
        fact.test_id = Some("save-btn".into());
        let plan = OverlayPlan::from_facts([fact], None, true);
        assert!(plan.badges[0].text.contains("#save-btn"));
        assert!(!plan.badges[0].text.contains("\u{201c}Save\u{201d}"));
    }

    #[test]
    fn no_hover_key_gives_badges_only() {
        let facts = vec![fact(key(2), "Button", "Save", [10.0, 20.0, 90.0, 60.0])];
        let plan = OverlayPlan::from_facts(facts, None, true);
        assert!(plan.hover.is_none());
        assert_eq!(plan.badges.len(), 1);
    }

    #[test]
    fn empty_plan_is_reported() {
        let plan = OverlayPlan::default();
        assert!(plan.is_empty());
        let facts = vec![fact(key(2), "Window", "Main", [0.0, 0.0, 8.0, 8.0])];
        let plan = OverlayPlan::from_facts(facts, None, true);
        assert!(plan.is_empty(), "non-interactive only → nothing to draw");
    }
}
