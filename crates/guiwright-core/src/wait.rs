//! Event-driven waiters (architecture §8.3 + §15.3).
//!
//! Waiters are state machines fed [`WaiterEvent`]s by the session loop. They
//! re-evaluate **only when the world changes** (tree diff or presented
//! frame); between events they do no work at all. There is no sleep-poll
//! loop anywhere in Guiwright — the session loop pumps the adapter and
//! forwards the resulting events, and the only wall-clock involvement is the
//! external timeout deadline.

use crate::actionability::{self, Check, CheckStatus};
use crate::key::NodeKey;
use crate::selector::Selector;
use crate::tree::{Resolved, TreeDiffSummary, TreeService};

/// Events that can change the outcome of a wait (the full set — anything
/// else never wakes a waiter).
#[derive(Clone, Copy, Debug)]
pub enum WaiterEvent<'a> {
    /// A tree diff was applied (§15.2 fan-out).
    TreeChanged(&'a TreeDiffSummary),
    /// A frame was presented (§10); carries its seq and digest hash.
    FramePresented { seq: u64, hash: u64 },
    /// An explicit evaluation request, used once at waiter registration.
    Initial,
}

/// Outcome of feeding an event to a waiter.
#[derive(Debug)]
pub enum WaitOutcome {
    /// Keep waiting (nothing definitively failed).
    Pending,
    /// All checks satisfied. `None` resolution is only possible for
    /// negated-existence assertions (`to_be_hidden` on a never-attached
    /// node — Playwright parity).
    Satisfied(Option<Resolved>),
    /// Definitively failed (fail fast: strict-mode violations, disabled
    /// nodes for enabled-checks, …). The session loop aborts and reports.
    Failed(WaitFailure),
}

#[derive(Debug)]
pub struct WaitFailure {
    pub check: String,
    pub reason: String,
    /// The last successful resolution, for error reports.
    pub last_resolved: Option<Resolved>,
}

/// A waiter state machine.
pub trait Waiter {
    fn on_event(&mut self, event: WaiterEvent<'_>, tree: &TreeService) -> WaitOutcome;
    /// Human description for traces and error messages.
    fn describe(&self) -> String;
}

/// Stability tracker: bounds identical across 2 consecutive `FramePresented`
/// events (§8.3 "Stable"). Lives inside the waiter so it survives across
/// events. When the bounds change, the changed frame becomes the new
/// baseline.
#[derive(Clone, Debug, Default)]
struct StabilityTracker {
    last_rect: Option<accesskit::Rect>,
    confirmed: bool,
}

impl StabilityTracker {
    fn on_frame(&mut self, tree: &TreeService, key: &NodeKey) -> CheckStatus {
        let Some(rect) = tree.node(key).and_then(|n| n.bounding_box()) else {
            *self = StabilityTracker::default();
            return CheckStatus::Pending {
                reason: "node has no bounds yet".to_string(),
            };
        };
        match self.last_rect {
            Some(prev) if prev == rect => {
                self.confirmed = true;
                CheckStatus::Pass
            }
            _ => {
                self.last_rect = Some(rect);
                self.confirmed = false;
                CheckStatus::Pending {
                    reason: "waiting for the rect to hold on the next frame".to_string(),
                }
            }
        }
    }
}

/// The locator waiter used by actions and expect assertions: a selector plus
/// an ordered set of checks, re-resolved on every event (Playwright re-queries
/// the DOM when it mutates).
#[derive(Debug)]
pub struct ChecksWaiter {
    selector: Selector,
    checks: Vec<Check>,
    stability: StabilityTracker,
    last_resolved: Option<Resolved>,
    /// Index of the next check that has not passed yet. Checks run in
    /// order; a node identity change resets progress (generation guard, §8.2).
    current: usize,
}

impl ChecksWaiter {
    pub fn new(selector: Selector, checks: Vec<Check>) -> Self {
        Self {
            selector,
            checks,
            stability: StabilityTracker::default(),
            last_resolved: None,
            current: 0,
        }
    }

    pub fn selector(&self) -> &Selector {
        &self.selector
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// Pure-existence assertions (`to_be_hidden`) pass when the selector
    /// matches nothing (Playwright parity).
    fn allows_absence(&self) -> bool {
        self.checks.iter().all(|c| *c == Check::Hidden)
    }
}

impl Waiter for ChecksWaiter {
    fn on_event(&mut self, event: WaiterEvent<'_>, tree: &TreeService) -> WaitOutcome {
        // Re-resolve on every event: the node may have appeared, moved, or
        // been replaced since the last one (Playwright re-query semantics).
        let resolved = match tree.resolve(&self.selector) {
            Ok(r) => Some(r),
            Err(crate::CoreError::Ambiguous { selector, matches }) => {
                // Strict-mode violations fail fast (§8.2).
                return WaitOutcome::Failed(WaitFailure {
                    check: "strict mode".to_string(),
                    reason: format!("selector `{selector}` resolved to {} nodes", matches.len()),
                    last_resolved: self.last_resolved.take(),
                });
            }
            Err(_) if self.allows_absence() => None,
            Err(_) => return WaitOutcome::Pending,
        };

        match resolved {
            None => WaitOutcome::Satisfied(None),
            Some(resolved) => {
                // A different identity invalidates per-node progress (§8.2).
                if self
                    .last_resolved
                    .as_ref()
                    .is_some_and(|prev| prev.key != resolved.key)
                {
                    self.current = 0;
                    self.stability = StabilityTracker::default();
                }
                self.last_resolved = Some(resolved.clone());

                // Advance the check sequence. `Stable` is frame-driven.
                while self.current < self.checks.len() {
                    let check = self.checks[self.current].clone();
                    if check == Check::Stable {
                        if !matches!(event, WaiterEvent::FramePresented { .. }) {
                            break;
                        }
                        match self.stability.on_frame(tree, &resolved.key) {
                            CheckStatus::Pass => self.current += 1,
                            CheckStatus::Pending { .. } => return WaitOutcome::Pending,
                            CheckStatus::Failed { reason } => {
                                return WaitOutcome::Failed(WaitFailure {
                                    check: check.describe(),
                                    reason,
                                    last_resolved: Some(resolved),
                                });
                            }
                        }
                    } else {
                        match actionability::evaluate(tree, &resolved.key, &check) {
                            CheckStatus::Pass => self.current += 1,
                            CheckStatus::Pending { .. } => return WaitOutcome::Pending,
                            CheckStatus::Failed { reason } => {
                                return WaitOutcome::Failed(WaitFailure {
                                    check: check.describe(),
                                    reason,
                                    last_resolved: Some(resolved),
                                });
                            }
                        }
                    }
                }

                if self.current >= self.checks.len() {
                    WaitOutcome::Satisfied(Some(resolved))
                } else {
                    WaitOutcome::Pending
                }
            }
        }
    }

    fn describe(&self) -> String {
        let checks: Vec<String> = self.checks.iter().map(Check::describe).collect();
        format!("{}: {}", self.selector.render(), checks.join(", "))
    }
}

#[cfg(test)]
impl WaitOutcome {
    fn expect_pending(&self) {
        match self {
            WaitOutcome::Pending => {}
            other => panic!("expected pending, got {other:?}"),
        }
    }

    fn expect_satisfied(self) {
        match self {
            WaitOutcome::Satisfied(_) => {}
            other => panic!("expected satisfied, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use accesskit::{NodeId, Role, TreeId};

    use crate::selector::TextMatch;
    use crate::test_util::Fixture;

    use super::*;

    fn setup() -> (TreeService, Fixture) {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        (tree, fixture)
    }

    fn save_key(f: &Fixture) -> NodeKey {
        NodeKey::AccessKit {
            tree: TreeId::ROOT,
            node: f.save_button,
        }
    }

    #[test]
    fn pending_on_initial_when_stability_required() {
        let (tree, _) = setup();
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            actionability::action_checks(false),
        );
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
    }

    #[test]
    fn stable_passes_after_two_equal_frames() {
        let (tree, f) = setup();
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            actionability::action_checks(false),
        );
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
        waiter
            .on_event(WaiterEvent::FramePresented { seq: 1, hash: 0 }, &tree)
            .expect_pending();
        match waiter.on_event(WaiterEvent::FramePresented { seq: 2, hash: 0 }, &tree) {
            WaitOutcome::Satisfied(Some(r)) => assert_eq!(r.key, save_key(&f)),
            other => panic!("expected satisfied after frame 2, got {other:?}"),
        }
    }

    #[test]
    fn moved_button_resets_stability() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Stable],
        );
        // Frame 1: baseline (old rect).
        waiter
            .on_event(WaiterEvent::FramePresented { seq: 1, hash: 1 }, &tree)
            .expect_pending();

        // Button jumps before stability confirmed: the changed frame
        // becomes the new baseline and the tracker resets.
        tree.apply_update(fixture.move_save_update(300.0, 10.0));
        waiter
            .on_event(WaiterEvent::FramePresented { seq: 2, hash: 2 }, &tree)
            .expect_pending();
        // Frame 3: equal to frame 2 → stable.
        waiter
            .on_event(WaiterEvent::FramePresented { seq: 3, hash: 2 }, &tree)
            .expect_satisfied();
    }

    #[test]
    fn satisfied_waiter_stays_satisfied() {
        let (tree, _) = setup();
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Visible],
        );
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_satisfied();
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_satisfied();
    }

    #[test]
    fn strict_violation_fails_fast() {
        let (mut tree, f) = setup();
        // Make a second "Save" button appear → ambiguous.
        let mut b2 = accesskit::Node::new(Role::Button);
        b2.set_label("Save");
        b2.set_bounds(accesskit::Rect {
            x0: 200.0,
            y0: 16.0,
            x1: 320.0,
            y1: 48.0,
        });
        let mut group = accesskit::Node::new(Role::Group);
        group.set_children([
            f.save_button,
            f.cancel_button,
            f.welcome_label,
            f.email_input,
            f.hidden_button,
            f.disabled_button,
            NodeId(9),
        ]);
        tree.apply_update(accesskit::TreeUpdate {
            nodes: vec![(NodeId(9), b2), (NodeId(1), group)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        });
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Attached],
        );
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Failed(failure) => assert_eq!(failure.check, "strict mode"),
            other => panic!("strict violation must fail fast, got {other:?}"),
        }
    }

    #[test]
    fn disable_keeps_enabled_check_pending() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.disable_save_update());
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Enabled],
        );
        // Playwright parity: wait for enabled until timeout, never fail fast.
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Pending => {}
            other => panic!("disabled must keep waiting, got {other:?}"),
        }
    }

    #[test]
    fn has_text_assertion_passes_on_value() {
        let (tree, _) = setup();
        let mut waiter = ChecksWaiter::new(
            Selector::role(Role::TextInput),
            vec![Check::HasText(TextMatch::new("example.dev"))],
        );
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_satisfied();
    }

    #[test]
    fn hidden_assertion_passes_when_node_missing() {
        let tree = TreeService::new();
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Ghost"),
            vec![Check::Hidden],
        );
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(None) => {}
            other => panic!("absence must satisfy hidden, got {other:?}"),
        }
    }

    #[test]
    fn identity_change_resets_progress() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        let mut waiter = ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Visible],
        );
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(Some(r)) => {
                let NodeKey::AccessKit { node, .. } = r.key else {
                    panic!("fixture keys are accesskit keys");
                };
                assert_eq!(node, fixture.save_button);
                assert_eq!(r.generation, 0);
            }
            other => panic!("expected satisfied on original save, got {other:?}"),
        }

        // Old Save leaves, a new "Save" appears under a different identity:
        // progress must reset and the new node must resolve.
        tree.apply_update(fixture.replace_save_update());
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(Some(r)) => {
                let NodeKey::AccessKit { node, .. } = r.key else {
                    panic!("fixture keys are accesskit keys");
                };
                assert_ne!(node, fixture.save_button);
                assert_eq!(r.generation, 0);
            }
            other => panic!("expected satisfied on replacement save, got {other:?}"),
        }
    }

    #[test]
    fn stale_generation_is_detected() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        let resolved = tree
            .resolve(&Selector::role_named(Role::Button, "Save"))
            .expect("save resolves");
        // Remove + re-add the same identity → generation bumped.
        tree.apply_update(fixture.remove_save_update());
        assert!(tree.check_generation(&resolved).is_err());
    }
}
