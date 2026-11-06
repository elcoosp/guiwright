//! Actionability checks — the anti-flakiness core (architecture §8.3).
//!
//! Every check is a pure function of the current consumer-tree state, so
//! they can be evaluated exactly when the world changes (event-driven
//! waiting, §15.3) instead of on a poll loop.

use accesskit_consumer::{FilterResult, common_filter};

use crate::key::NodeKey;
use crate::selector::TextMatch;
use crate::tree::TreeService;

/// One actionability check (§8.3 table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// Node is present in the (consumer) tree.
    Attached,
    /// `common_filter` includes the node *and* its bounds have area > 0.
    Visible,
    /// Negated visibility (expect().to_be_hidden()).
    Hidden,
    /// `!node.is_disabled()`.
    Enabled,
    /// `node.is_disabled()`.
    Disabled,
    /// Bounds identical across 2 consecutive `FramePresented` events.
    /// Evaluated only on frame events; the tracker lives in the waiter.
    Stable,
    /// `hit_test` at the node's center resolves to the node or a
    /// descendant (no occluding overlay).
    ReceivesEvents,
    /// Label/value text matches (assertions + `fill` pre-checks).
    HasText(TextMatch),
    /// Focused node identity matches.
    Focused,
}

impl Check {
    /// Stable rendering for error messages and traces.
    pub fn describe(&self) -> String {
        match self {
            Check::Attached => "attached".to_string(),
            Check::Visible => "visible".to_string(),
            Check::Hidden => "hidden".to_string(),
            Check::Enabled => "enabled".to_string(),
            Check::Disabled => "disabled".to_string(),
            Check::Stable => "stable".to_string(),
            Check::ReceivesEvents => "receives events".to_string(),
            Check::HasText(m) => format!("has text {}", render(m)),
            Check::Focused => "focused".to_string(),
        }
    }
}

fn render(m: &TextMatch) -> String {
    let pat = m.pattern.as_str();
    if m.exact && !m.ignore_case {
        format!("{pat:?}")
    } else {
        format!("{pat:?} (fuzzy)")
    }
}

/// A check is either satisfiable right now, not yet (keep waiting), or
/// definitively failed (fail fast, e.g. strict-mode ambiguity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Pending { reason: String },
    Failed { reason: String },
}

/// Evaluate `check` against the current tree state for the node `key`.
///
/// `Pending` means "the world may still change so the check can pass";
/// `Failed` means "this cannot pass anymore, abort early".
pub fn evaluate(tree: &TreeService, key: &NodeKey, check: &Check) -> CheckStatus {
    let Some(node) = tree.node(key) else {
        return match check {
            // Missing node: keep waiting for attach, except when the check
            // is about being gone.
            Check::Hidden => CheckStatus::Pass,
            _ => CheckStatus::Pending {
                reason: "node not attached".to_string(),
            },
        };
    };

    match check {
        Check::Attached => CheckStatus::Pass,
        Check::Visible => {
            let filtered = common_filter(&node);
            if filtered != FilterResult::Include {
                // Playwright waits for visibility; hidden nodes may become
                // visible, so this stays Pending (never fail-fast).
                return CheckStatus::Pending {
                    reason: "node is hidden".to_string(),
                };
            }
            match node.bounding_box() {
                Some(rect) if rect.area() > 0.0 => CheckStatus::Pass,
                Some(_) => CheckStatus::Pending {
                    reason: "bounds are empty (zero area)".to_string(),
                },
                None => CheckStatus::Pending {
                    reason: "node has no bounds yet".to_string(),
                },
            }
        }
        Check::Hidden => {
            let filtered = common_filter(&node);
            if filtered != FilterResult::Include {
                return CheckStatus::Pass;
            }
            match node.bounding_box() {
                Some(rect) if rect.area() > 0.0 => CheckStatus::Pending {
                    reason: "node is visible".to_string(),
                },
                _ => CheckStatus::Pass,
            }
        }
        Check::Enabled => {
            if node.is_disabled() {
                // Playwright waits for the node to become enabled.
                CheckStatus::Pending {
                    reason: "node is disabled".to_string(),
                }
            } else {
                CheckStatus::Pass
            }
        }
        Check::Disabled => {
            if node.is_disabled() {
                CheckStatus::Pass
            } else {
                CheckStatus::Pending {
                    reason: "node is enabled".to_string(),
                }
            }
        }
        Check::Stable => {
            // Evaluated by the waiter's stability tracker on FramePresented.
            CheckStatus::Pending {
                reason: "waiting for two consecutive stable frames".to_string(),
            }
        }
        Check::ReceivesEvents => match node.bounding_box() {
            Some(rect) => {
                let point = accesskit::Point {
                    x: (rect.x0 + rect.x1) / 2.0,
                    y: (rect.y0 + rect.y1) / 2.0,
                };
                match tree.hit_test(point) {
                    Some(hit) if hit == *key => CheckStatus::Pass,
                    Some(hit) => {
                        // A descendant receiving the event is fine
                        // (buttons contain label children).
                        let mut ok = false;
                        tree.descendants(key, &mut |k, _| {
                            if k == hit {
                                ok = true;
                            }
                        });
                        if ok {
                            CheckStatus::Pass
                        } else {
                            CheckStatus::Pending {
                                reason: format!("point is intercepted by {}", hit.describe()),
                            }
                        }
                    }
                    None => CheckStatus::Pending {
                        reason: "hit test found nothing at the node center".to_string(),
                    },
                }
            }
            None => CheckStatus::Pending {
                reason: "node has no bounds yet".to_string(),
            },
        },
        Check::HasText(m) => {
            let label = node.label();
            let value = node.value();
            if label.as_deref().is_some_and(|l| m.matches(l))
                || value.as_deref().is_some_and(|v| m.matches(v))
            {
                CheckStatus::Pass
            } else {
                CheckStatus::Pending {
                    reason: format!(
                        "text is {:?}, expected {}",
                        label.or(value).unwrap_or_default(),
                        render(m)
                    ),
                }
            }
        }
        Check::Focused => {
            if tree.focus() == Some(*key) {
                CheckStatus::Pass
            } else {
                CheckStatus::Pending {
                    reason: "node is not focused".to_string(),
                }
            }
        }
    }
}

/// The ordered checks Playwright runs before an action (§8.3), minus
/// `Stable` when `force` is set (§8.3 "Force mode").
pub fn action_checks(force: bool) -> Vec<Check> {
    if force {
        vec![Check::Attached, Check::Visible, Check::Enabled]
    } else {
        vec![
            Check::Attached,
            Check::Visible,
            Check::Stable,
            Check::Enabled,
            Check::ReceivesEvents,
        ]
    }
}

#[cfg(test)]
mod tests {
    use accesskit::Role;
    use accesskit_consumer::common_filter;

    use crate::selector::TextMatch;
    use crate::test_util::Fixture;
    use crate::tree::TreeService;

    use super::*;

    fn service_with_save() -> (TreeService, NodeKey) {
        let fixture = Fixture::buttons();
        let mut svc = TreeService::new();
        svc.apply_update(fixture.initial_update());
        let key = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: fixture.save_button,
        };
        (svc, key)
    }

    #[test]
    fn visible_passes_for_bounded_button() {
        let (svc, save) = service_with_save();
        assert_eq!(evaluate(&svc, &save, &Check::Visible), CheckStatus::Pass);
        assert_eq!(evaluate(&svc, &save, &Check::Enabled), CheckStatus::Pass);
        assert_eq!(evaluate(&svc, &save, &Check::Attached), CheckStatus::Pass);
    }

    #[test]
    fn hidden_button_pends_on_visible_and_passes_hidden() {
        let fixture = Fixture::buttons();
        let mut svc = TreeService::new();
        svc.apply_update(fixture.initial_update());
        let key = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: fixture.hidden_button,
        };
        let status = evaluate(&svc, &key, &Check::Visible);
        // Playwright parity: wait for visibility, never fail fast.
        assert!(matches!(status, CheckStatus::Pending { .. }), "{status:?}");
        assert_eq!(evaluate(&svc, &key, &Check::Hidden), CheckStatus::Pass);
    }

    #[test]
    fn disabled_check_reflects_state() {
        let fixture = Fixture::buttons();
        let mut svc = TreeService::new();
        svc.apply_update(fixture.initial_update());
        let key = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: fixture.disabled_button,
        };
        assert_eq!(evaluate(&svc, &key, &Check::Disabled), CheckStatus::Pass);
        let status = evaluate(&svc, &key, &Check::Enabled);
        // Playwright parity: wait for enabled, never fail fast.
        assert!(matches!(status, CheckStatus::Pending { .. }), "{status:?}");
    }

    #[test]
    fn receives_events_passes_for_save() {
        let (svc, save) = service_with_save();
        // Save button's center is not occluded.
        assert_eq!(
            evaluate(&svc, &save, &Check::ReceivesEvents),
            CheckStatus::Pass
        );
    }

    #[test]
    fn receives_events_fails_when_occluded_by_overlay() {
        let (mut svc, save) = service_with_save();
        // Add a modal dialog covering the save button's center.
        let mut dialog = accesskit::Node::new(Role::Dialog);
        dialog.set_label("Modal");
        dialog.set_bounds(accesskit::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 480.0,
            y1: 240.0,
        });
        let mut window = accesskit::Node::new(Role::Window);
        window.set_children([accesskit::NodeId(1), accesskit::NodeId(9)]);
        let mut group = accesskit::Node::new(Role::Group);
        group.set_children([
            accesskit::NodeId(2),
            accesskit::NodeId(3),
            accesskit::NodeId(4),
            accesskit::NodeId(5),
            accesskit::NodeId(6),
            accesskit::NodeId(7),
        ]);
        svc.apply_update(accesskit::TreeUpdate {
            nodes: vec![
                (accesskit::NodeId(0), window),
                (accesskit::NodeId(1), group),
                (accesskit::NodeId(9), dialog),
            ],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(0),
        });
        let status = evaluate(&svc, &save, &Check::ReceivesEvents);
        assert!(matches!(status, CheckStatus::Pending { .. }), "{status:?}");
    }

    #[test]
    fn missing_node_keeps_waiting_except_hidden() {
        let svc = TreeService::new();
        let ghost = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: accesskit::NodeId(999),
        };
        let status = evaluate(&svc, &ghost, &Check::Attached);
        assert!(matches!(status, CheckStatus::Pending { .. }), "{status:?}");
        assert_eq!(evaluate(&svc, &ghost, &Check::Hidden), CheckStatus::Pass);
    }

    #[test]
    fn has_text_evaluates_label_and_value() {
        let fixture = Fixture::buttons();
        let mut svc = TreeService::new();
        svc.apply_update(fixture.initial_update());
        let email = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: fixture.email_input,
        };
        assert_eq!(
            evaluate(&svc, &email, &Check::HasText(TextMatch::new("example"))),
            CheckStatus::Pass
        );
        let status = evaluate(&svc, &email, &Check::HasText(TextMatch::exact("ada")));
        assert!(matches!(status, CheckStatus::Pending { .. }), "{status:?}");
    }

    #[test]
    fn common_filter_excludes_hidden() {
        let fixture = Fixture::buttons();
        let mut svc = TreeService::new();
        svc.apply_update(fixture.initial_update());
        let hidden = svc
            .node(&NodeKey::AccessKit {
                tree: accesskit::TreeId::ROOT,
                node: fixture.hidden_button,
            })
            .expect("hidden button is in the tree");
        assert_ne!(common_filter(&hidden), FilterResult::Include);
    }

    #[test]
    fn force_skips_stability_and_hit_test() {
        let checks = action_checks(true);
        assert!(!checks.contains(&Check::Stable));
        assert!(!checks.contains(&Check::ReceivesEvents));
        let checks = action_checks(false);
        assert!(checks.contains(&Check::Stable));
        assert!(checks.contains(&Check::ReceivesEvents));
    }
}
