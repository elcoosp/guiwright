//! The a11y-hybrid activation model (§4.5 + §18).
//!
//! GPUI's AccessKit support is unconditional, but the tree only flows
//! while the window's accessibility is **active** — and activation
//! semantics differ per platform (§18 "AccessKit laziness/activation
//! semantics differ"). The §18 mitigation is the contract here: "adapters
//! force-enable at driver attach (`update_if_active`-compatible;
//! `ActivationHandler` with immediate activation for tests)".
//!
//! Once active, per-frame `TreeUpdate`s flow through
//! `PlatformWindow::a11y_tree_update` with node bounds recorded per
//! `NodeId` (`A11y.node_bounds`, `window/a11y.rs:150`; built in
//! `element.rs:428-462`) — this module models exactly that bookkeeping:
//! the activation gate + the node-bounds map the geometry checks need.

use std::collections::HashMap;

use accesskit::{NodeId, TreeUpdate};

/// When the a11y tree activates (§18 mitigation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationPolicy {
    /// Test mode: the `ActivationHandler` returns `Activate` immediately
    /// at attach — the driver never waits for a screen reader.
    Immediate,
    /// Production default: activation on the OS's signal.
    OnDemand,
}

/// Statistics of one applied `TreeUpdate` (what the session logs per
/// frame, §15.2 "one diff, many consumers").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub nodes_added: usize,
    pub nodes_updated: usize,
    pub nodes_removed: usize,
    pub focus_moved: bool,
}

/// The activation gate + node-bounds map (§4.5 `A11y.node_bounds` analog).
#[derive(Clone, Debug)]
pub struct A11yHybrid {
    policy: ActivationPolicy,
    activated: bool,
    bounds: HashMap<u64, [f64; 4]>,
    focus: Option<u64>,
}

impl A11yHybrid {
    /// Attach-time state: [`ActivationPolicy::Immediate`] activates at
    /// attach (§18 "force-enable at driver attach").
    pub fn new(policy: ActivationPolicy) -> Self {
        Self {
            policy,
            activated: matches!(policy, ActivationPolicy::Immediate),
            bounds: HashMap::new(),
            focus: None,
        }
    }

    /// Activate the tree (the OS signal, or the test handler).
    pub fn activate(&mut self) {
        self.activated = true;
    }

    /// Deactivate (window hidden, OS signal) — updates stop flowing.
    pub fn deactivate(&mut self) {
        self.activated = false;
    }

    pub fn is_active(&self) -> bool {
        self.activated
    }

    pub fn policy(&self) -> ActivationPolicy {
        self.policy
    }

    /// Apply one per-frame update while active. While **inactive** the
    /// update is dropped (GPUI gates the tree on the active flag) — the
    /// driver-side consequence is "no tree yet", which the attach-time
    /// immediate activation prevents.
    pub fn apply(&mut self, update: &TreeUpdate) -> Option<DiffStats> {
        if !self.activated {
            return None;
        }
        let mut stats = DiffStats {
            focus_moved: false,
            ..DiffStats::default()
        };
        for (id, node) in &update.nodes {
            if self.bounds.contains_key(&id.0) {
                stats.nodes_updated += 1;
            } else {
                stats.nodes_added += 1;
            }
            if let Some(rect) = node.bounds() {
                self.bounds
                    .insert(id.0, [rect.x0, rect.y0, rect.x1, rect.y1]);
            }
        }
        // Removed ids: a TreeUpdate carries no tombstone list; the
        // consumer tree tracks removals incrementally. The hybrid's map
        // prunes ids absent from the update's node set only when the
        // update is a full-tree reset (`tree` present).
        if update.tree.is_some() {
            let live: std::collections::HashSet<u64> =
                update.nodes.iter().map(|(id, _)| id.0).collect();
            let removed = self.bounds.keys().filter(|k| !live.contains(k)).count();
            stats.nodes_removed = removed;
            self.bounds.retain(|k, _| live.contains(k));
        }
        let new_focus: Option<u64> = Some(update.focus.0);
        if new_focus != self.focus {
            self.focus = new_focus;
            stats.focus_moved = true;
        }
        Some(stats)
    }

    /// The node bounds the geometry checks need (§8.3 stable/visible).
    pub fn bounds_of(&self, node: NodeId) -> Option<[f64; 4]> {
        self.bounds.get(&node.0).copied()
    }

    /// The focused node (§8.3 focused check).
    pub fn focus(&self) -> Option<u64> {
        self.focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Node, Rect, TreeId, TreeInfo};

    fn update(nodes: &[(u64, [f64; 4])], focus: Option<u64>, full: bool) -> TreeUpdate {
        TreeUpdate {
            nodes: nodes
                .iter()
                .map(|(id, b)| {
                    let mut n = Node::new(accesskit::Role::Button);
                    n.set_bounds(Rect {
                        x0: b[0],
                        y0: b[1],
                        x1: b[2],
                        y1: b[3],
                    });
                    (NodeId(*id), n)
                })
                .collect(),
            tree: full.then(|| TreeInfo {
                root: NodeId(nodes.first().map_or(0, |(id, _)| *id)),
                toolkit_name: Some("gpui-test".into()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: NodeId(focus.unwrap_or(0)),
        }
    }

    #[test]
    fn immediate_policy_activates_at_attach() {
        let hybrid = A11yHybrid::new(ActivationPolicy::Immediate);
        assert!(hybrid.is_active(), "§18: force-enable at driver attach");
        assert_eq!(hybrid.policy(), ActivationPolicy::Immediate);
        let on_demand = A11yHybrid::new(ActivationPolicy::OnDemand);
        assert!(!on_demand.is_active());
    }

    #[test]
    fn inactive_updates_are_dropped() {
        let mut hybrid = A11yHybrid::new(ActivationPolicy::OnDemand);
        assert!(
            hybrid
                .apply(&update(&[(1, [0.0; 4])], None, true))
                .is_none()
        );
        hybrid.activate();
        assert!(
            hybrid
                .apply(&update(&[(1, [0.0; 4])], None, true))
                .is_some()
        );
        hybrid.deactivate();
        assert!(
            hybrid
                .apply(&update(&[(2, [0.0; 4])], None, true))
                .is_none()
        );
    }

    #[test]
    fn node_bounds_are_recorded_per_node() {
        let mut hybrid = A11yHybrid::new(ActivationPolicy::Immediate);
        let stats = hybrid
            .apply(&update(
                &[(1, [0.0, 0.0, 100.0, 40.0]), (2, [0.0, 48.0, 80.0, 80.0])],
                Some(2),
                true,
            ))
            .expect("active");
        assert_eq!(stats.nodes_added, 2);
        assert_eq!(stats.nodes_updated, 0);
        assert!(stats.focus_moved);
        assert_eq!(hybrid.bounds_of(NodeId(1)), Some([0.0, 0.0, 100.0, 40.0]));
        assert_eq!(hybrid.focus(), Some(2));
    }

    #[test]
    fn full_resets_prune_stale_bounds() {
        let mut hybrid = A11yHybrid::new(ActivationPolicy::Immediate);
        hybrid
            .apply(&update(&[(1, [0.0; 4]), (2, [0.0; 4])], None, true))
            .expect("active");
        // Incremental update for node 1 only: node 2 survives.
        hybrid
            .apply(&update(&[(1, [8.0; 4])], None, false))
            .expect("active");
        assert!(hybrid.bounds_of(NodeId(2)).is_some());
        // Full reset without node 2: pruned.
        let stats = hybrid
            .apply(&update(&[(1, [8.0; 4])], None, true))
            .expect("active");
        assert_eq!(stats.nodes_removed, 1);
        assert!(hybrid.bounds_of(NodeId(2)).is_none());
    }
}
