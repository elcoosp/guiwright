//! The replayable §13 overlay (m8's named viewer gaps, paid): the plan
//! is rebuilt from the *archived* tree facts instead of the live
//! session's — m8's remaining-gaps list records exactly this
//! destination ("Full-tree badges (the §13 'deep-inspect mode') land
//! with the viewer" and "replayable overlays belong to the viewer").
//!
//! The live session composites **interactive-only** badges at present
//! time (`OverlayPlan::from_facts(..., interactive_only = true)`, the
//! §13 default); the replay is the deep-inspect half: every
//! bounds-carrying node is badged — non-interactive roles included —
//! and the recorded focus is outlined (the archived stand-in for the
//! live hover outline: an archive has no hover, but it records what had
//! focus). Removed rows (`TreeViewNode::removed`, the timeline's greyed
//! rows) earn no badge — they are not on screen at that frame — and
//! bounds-less nodes stay unpinned (the §13 badge needs a point).
//!
//! The facts ride [`NodeFacts`] and the plan comes from the same
//! [`OverlayPlan::from_facts`] cost model the live session speaks, so
//! replayed and live badges render identically. The replay mints
//! replay-scoped [`NodeKey`]s (`Logical` under the `"trace"` adapter
//! name) purely for the outline's hover match — they never leave the
//! plan build and carry no identity beyond the archived row ids.

use guiwright_core::NodeKey;
use guiwright_inspect::OverlayPlan;
use guiwright_inspect::codegen::NodeFacts;

use crate::timeline::{Timeline, TreeView};

/// The replay-scoped logical-key adapter name: archived rows carry only
/// `u64` ids, so the outline's hover match keys under this namespace.
const REPLAY_ADAPTER: &str = "trace";

/// Build the deep-inspect overlay plan from a resolved tree view
/// (`Timeline::tree_at`'s snapshot+diffs resolution): every live node
/// with bounds is badged in document order, the focused node outlined.
pub fn replay_plan(tree: &TreeView) -> OverlayPlan {
    let facts = tree.nodes.iter().filter(|n| !n.removed).map(|n| NodeFacts {
        key: NodeKey::Logical {
            adapter: REPLAY_ADAPTER,
            id: n.id,
        },
        role: Some(n.role.clone()),
        name: n.label.clone(),
        label: None,
        value: n.value.clone(),
        test_id: None,
        bounds: n.bounds,
    });
    let hover_key = tree.focus.map(|id| NodeKey::Logical {
        adapter: REPLAY_ADAPTER,
        id,
    });
    OverlayPlan::from_facts(facts, hover_key.as_ref(), false)
}

/// The scrubber-facing composition: resolve the tree at `frame` and
/// replay its plan. `None` when the frame predates every snapshot — an
/// honest absence, not an empty layer.
pub fn replay_plan_at(timeline: &Timeline, frame: u64) -> Option<OverlayPlan> {
    timeline.tree_at(frame).as_ref().map(replay_plan)
}

/// One plan per snapshot frame (the HTML embed's unit — the scrubber
/// shows the nearest snapshot ≤ frame, so the overlay rides the same
/// snapshot the tree inspector does).
pub fn snapshot_replay_plans(timeline: &Timeline) -> Vec<(u64, OverlayPlan)> {
    timeline
        .snapshot_payloads()
        .keys()
        .filter_map(|frame| {
            let plan = replay_plan_at(timeline, *frame)?;
            Some((*frame, plan))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::TreeViewNode;
    use guiwright_inspect::overlay::OverlayRect;

    fn view(nodes: Vec<TreeViewNode>, focus: Option<u64>) -> TreeView {
        TreeView {
            frame_seq: 1,
            snapshot_frame: Some(1),
            nodes,
            pending_changes: Vec::new(),
            focus,
        }
    }

    fn node(id: u64, role: &str, bounds: Option<[f64; 4]>) -> TreeViewNode {
        TreeViewNode {
            id,
            parent: Some(1),
            role: role.into(),
            label: Some(format!("n{id}")),
            value: None,
            bounds,
            removed: false,
        }
    }

    #[test]
    fn full_tree_badges_every_node_with_bounds() {
        // The deep-inspect rule (the plan line this module pays): the
        // non-interactive role is badged too — only the live session
        // composites interactive-only.
        let tree = view(
            vec![
                node(1, "Window", Some([0.0, 0.0, 800.0, 600.0])),
                node(2, "Button", Some([16.0, 16.0, 136.0, 48.0])),
                node(3, "Text", Some([16.0, 56.0, 136.0, 88.0])),
            ],
            Some(2),
        );
        let plan = replay_plan(&tree);
        let texts: Vec<&str> = plan.badges.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Window \u{201c}n1\u{201d}",
                "Button \u{201c}n2\u{201d}",
                "Text \u{201c}n3\u{201d}",
            ],
            "document order, full tree"
        );
        assert_eq!(
            plan.hover.expect("focused node outlined"),
            OverlayRect::new(16.0, 16.0, 136.0, 48.0)
        );
        assert!(!plan.is_empty());
    }

    #[test]
    fn removed_rows_and_boundsless_nodes_earn_no_badge() {
        let mut gone = node(2, "Button", Some([16.0, 16.0, 136.0, 48.0]));
        gone.removed = true;
        let tree = view(
            vec![
                node(1, "Window", Some([0.0, 0.0, 800.0, 600.0])),
                gone,
                node(3, "Text", None),
            ],
            None,
        );
        let plan = replay_plan(&tree);
        let texts: Vec<&str> = plan.badges.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Window \u{201c}n1\u{201d}"],
            "removed + bounds-less skipped"
        );
        assert!(plan.hover.is_none(), "no focus recorded, no outline");
    }

    #[test]
    fn an_empty_view_plans_nothing() {
        let plan = replay_plan(&view(Vec::new(), None));
        assert!(plan.is_empty(), "the honest empty plan");
    }
}
