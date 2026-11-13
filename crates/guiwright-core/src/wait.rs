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
    /// All checks satisfied. `None` resolution covers the assertions whose
    /// outcome is not a node: negated-existence (`to_be_hidden` on a
    /// never-attached node — Playwright parity) and the selector-level
    /// count waiter (M22 — the count is the answer, no node to hand back).
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
    /// The last mismatch the waiter can name, for timeout reports — the
    /// M17 gate's `stuck on {check} ({reason})` convention generalized
    /// (M22): a waiter that knows what it is waiting on embeds it in the
    /// timeout error instead of leaving the operation header alone.
    /// Default: none — the header carries the whole context.
    fn stuck_on(&self) -> Option<String> {
        None
    }
}

/// Stability tracker: bounds identical across 2 consecutive `FramePresented`
/// events (§8.3 "Stable"). Lives inside the waiter so it survives across
/// events. When the bounds change, the changed frame becomes the new
/// baseline.
///
/// Public since M17: the wire `Action` path's pre-resolved actionability
/// gate re-uses the exact `Stable` semantics without going through a
/// selector waiter — feed it once per frame boundary.
#[derive(Clone, Debug, Default)]
pub struct StabilityTracker {
    last_rect: Option<accesskit::Rect>,
    confirmed: bool,
}

impl StabilityTracker {
    pub fn on_frame(&mut self, tree: &TreeService, key: &NodeKey) -> CheckStatus {
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

/// The comparison a count assertion demands (§16 assertions row, M25):
/// the M22 exact match widened with the two one-sided comparators — the
/// m22 record's reserved vocabulary widening. A comparator holds while
/// the match-set size stands on the demanded side of the bound; the
/// mismatch stays waitable, never fail-fast (nodes stream in and out,
/// the same contract as the exact count).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountMatch {
    /// Exactly `n` matches (M22's `to_have_count`).
    Exact(usize),
    /// More than `n` matches (M25's `to_have_count_greater_than`).
    GreaterThan(usize),
    /// Fewer than `n` matches (M25's `to_have_count_less_than`).
    LessThan(usize),
}

impl CountMatch {
    /// Whether `found` satisfies the comparison.
    fn holds(self, found: usize) -> bool {
        match self {
            CountMatch::Exact(n) => found == n,
            CountMatch::GreaterThan(n) => found > n,
            CountMatch::LessThan(n) => found < n,
        }
    }

    /// The rendering used by descriptions and timeout headers
    /// (`count 4`, `count > 3`, `count < 5`) — the exact form is the
    /// M22 header, byte-identical through the shared path.
    pub fn describe(self) -> String {
        match self {
            CountMatch::Exact(n) => format!("count {n}"),
            CountMatch::GreaterThan(n) => format!("count > {n}"),
            CountMatch::LessThan(n) => format!("count < {n}"),
        }
    }

    /// The bound the comparison stands against (the count the assertion
    /// names).
    pub fn bound(self) -> usize {
        match self {
            CountMatch::Exact(n) | CountMatch::GreaterThan(n) | CountMatch::LessThan(n) => n,
        }
    }
}

/// The selector-level waiter (§16 assertions row, M22): counts
/// [`TreeService::resolve_all`] matches instead of strict-resolving one
/// node — the seam that lets the assertion engine talk about the *set* of
/// matches (`expect(locator).to_have_count(n)`, Playwright parity) without
/// tripping strict mode. A mismatched count is waitable, never fail-fast:
/// nodes stream in and out, exactly what the assertion is for.
///
/// M25 generalizes the demanded count into a [`CountMatch`]: the exact
/// form and the one-sided comparators are one waiter — only the
/// comparison differs.
#[derive(Debug)]
pub struct CountWaiter {
    selector: Selector,
    /// The comparison the assertion demands.
    comparison: CountMatch,
    /// The match count the last evaluation saw (timeout reports name it).
    found: Option<usize>,
}

impl CountWaiter {
    pub fn new(selector: Selector, expected: usize) -> Self {
        Self::with_match(selector, CountMatch::Exact(expected))
    }

    /// The comparator-shaped constructor (M25): the exact count, the
    /// greater-than and the less-than assertions ride the same waiter.
    pub fn with_match(selector: Selector, comparison: CountMatch) -> Self {
        Self {
            selector,
            comparison,
            found: None,
        }
    }

    /// The bound the assertion demands (the comparator's count).
    pub fn expected(&self) -> usize {
        self.comparison.bound()
    }

    /// The comparison the assertion demands.
    pub fn comparison(&self) -> CountMatch {
        self.comparison
    }

    /// The count the last evaluation found (`None` before the first one).
    pub fn found(&self) -> Option<usize> {
        self.found
    }
}

impl Waiter for CountWaiter {
    fn on_event(&mut self, _event: WaiterEvent<'_>, tree: &TreeService) -> WaitOutcome {
        // Re-resolve on every event (Playwright re-query semantics). A
        // resolution error keeps waiting — transient by contract, and
        // there is no strict mode to fail fast into. The event kind itself
        // is irrelevant: any world change re-reads the set.
        let Ok(matches) = tree.resolve_all(&self.selector) else {
            return WaitOutcome::Pending;
        };
        self.found = Some(matches.len());
        if self.comparison.holds(matches.len()) {
            // The comparison is the answer; no node rides back (the same
            // `Satisfied(None)` shape the negated-existence assertions use).
            WaitOutcome::Satisfied(None)
        } else {
            WaitOutcome::Pending
        }
    }

    fn describe(&self) -> String {
        format!(
            "{} — {}",
            self.comparison.describe(),
            self.selector.render()
        )
    }

    fn stuck_on(&self) -> Option<String> {
        match self.found {
            Some(found) if !self.comparison.holds(found) => Some(format!("found {found}")),
            _ => None,
        }
    }
}

/// The negation decorator (§16 assertions row, M24): wraps any waiter and
/// inverts its *waitable* outcome — Playwright's `expect(locator).not.…`
/// modifier as a waiter over any waiter (the M22 `Box<dyn Waiter>` seam
/// made `Expectation` a dispatcher over any waiter; this is the first
/// composition that seam enables).
///
/// The mapping is the Playwright semantics:
///
/// * inner **satisfied** → outer [`WaitOutcome::Pending`] — the condition
///   the assertion rejects still holds, so the negated form keeps
///   waiting;
/// * inner **pending** → outer `Satisfied(None)` — the world does not
///   satisfy the positive condition right now, so the negation holds,
///   with no node to hand back (the node that satisfied the positive
///   form is precisely the one the negation rejects);
/// * inner **failed** → outer **failed** — fail-fast stays fail-fast. A
///   strict-mode violation is a resolution failure, not a state
///   mismatch, so `not` must not launder it into a pass (Playwright
///   parity), and a role that can never expose the checked state cannot
///   pass under negation either (the M20 rule).
pub struct NotWaiter {
    inner: Box<dyn Waiter>,
}

impl NotWaiter {
    pub fn new(inner: Box<dyn Waiter>) -> Self {
        Self { inner }
    }
}

impl std::fmt::Debug for NotWaiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The inner waiter is a state machine over borrowed trees, not
        // `Debug` itself; its own description is the honest rendering.
        f.debug_tuple("NotWaiter")
            .field(&self.inner.describe())
            .finish()
    }
}

impl Waiter for NotWaiter {
    fn on_event(&mut self, event: WaiterEvent<'_>, tree: &TreeService) -> WaitOutcome {
        match self.inner.on_event(event, tree) {
            WaitOutcome::Satisfied(_) => WaitOutcome::Pending,
            WaitOutcome::Pending => WaitOutcome::Satisfied(None),
            WaitOutcome::Failed(failure) => WaitOutcome::Failed(failure),
        }
    }

    fn describe(&self) -> String {
        format!("not ({})", self.inner.describe())
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

    // --- The count waiter (§16 assertions row, M22) -----------------------

    #[test]
    fn count_waiter_satisfied_on_the_exact_match_count() {
        let (tree, _) = setup();
        let mut waiter = CountWaiter::new(Selector::role(Role::Button), 4);
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(_) => {}
            other => panic!("expected satisfied on 4 buttons, got {other:?}"),
        }
    }

    #[test]
    fn count_waiter_pends_until_the_count_moves() {
        let (tree, f) = setup();
        let mut waiter = CountWaiter::new(Selector::role(Role::Button), 5);
        // 4 buttons: not the expected 5 yet — and never fail-fast; the
        // count is waitable (nodes stream in and out, Playwright parity).
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
        let mut tree = tree;
        tree.apply_update(f.add_button_update("Export"));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(_) => {}
            other => panic!("expected satisfied once the fifth lands, got {other:?}"),
        }
    }

    #[test]
    fn count_waiter_satisfied_on_zero_matches() {
        let tree = TreeService::new();
        let mut waiter = CountWaiter::new(Selector::role_named(Role::Button, "Ghost"), 0);
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(None) => {}
            other => panic!("zero matches is a satisfied count, got {other:?}"),
        }
    }

    #[test]
    fn count_waiter_names_the_found_count_when_stuck() {
        let (tree, _) = setup();
        let mut waiter = CountWaiter::new(Selector::role(Role::Button), 5);
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
        assert_eq!(waiter.found(), Some(4));
        assert_eq!(waiter.stuck_on().as_deref(), Some("found 4"));
    }

    // --- The count comparators (§16 assertions row, M25) ------------------

    #[test]
    fn greater_than_satisfied_when_the_count_clears_the_bound() {
        let (tree, _) = setup();
        let mut waiter =
            CountWaiter::with_match(Selector::role(Role::Button), CountMatch::GreaterThan(3));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(_) => {}
            other => panic!("4 buttons clear the > 3 bound, got {other:?}"),
        }
    }

    #[test]
    fn greater_than_pends_below_the_bound_and_names_the_found_count() {
        let (tree, _) = setup();
        let mut waiter = CountWaiter::with_match(
            Selector::role_named(Role::Button, "Save"),
            CountMatch::GreaterThan(1),
        );
        // Exactly 1 "Save": not more than 1 — waitable, never fail-fast.
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
        assert_eq!(waiter.stuck_on().as_deref(), Some("found 1"));
        assert_eq!(waiter.expected(), 1);
    }

    #[test]
    fn less_than_satisfied_below_the_bound_and_pends_above() {
        let (tree, _) = setup();
        let mut waiter =
            CountWaiter::with_match(Selector::role(Role::Button), CountMatch::LessThan(5));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(_) => {}
            other => panic!("4 buttons sit below the < 5 bound, got {other:?}"),
        }
        // Zero matches sit below any positive bound too.
        let tree = TreeService::new();
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(None) => {}
            other => panic!("0 buttons sit below the < 5 bound, got {other:?}"),
        }
        let (tree, _) = setup();
        let mut waiter =
            CountWaiter::with_match(Selector::role(Role::Button), CountMatch::LessThan(4));
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
    }

    #[test]
    fn comparators_describe_the_comparison() {
        let waiter =
            CountWaiter::with_match(Selector::role(Role::Button), CountMatch::GreaterThan(3));
        assert_eq!(waiter.describe(), "count > 3 — getByRole(Button)");
        let waiter = CountWaiter::with_match(Selector::role(Role::Button), CountMatch::LessThan(5));
        assert_eq!(waiter.describe(), "count < 5 — getByRole(Button)");
        // The exact form keeps the M22 description byte-identical.
        let waiter = CountWaiter::new(Selector::role(Role::Button), 4);
        assert_eq!(waiter.describe(), "count 4 — getByRole(Button)");
    }

    #[test]
    fn the_bound_is_the_demanded_count_for_every_comparator() {
        assert_eq!(CountMatch::Exact(4).bound(), 4);
        assert_eq!(CountMatch::GreaterThan(3).bound(), 3);
        assert_eq!(CountMatch::LessThan(5).bound(), 5);
    }

    // --- The negation decorator (§16 assertions row, M24) -----------------

    #[test]
    fn not_waiter_pends_while_the_positive_condition_holds() {
        let (tree, _) = setup();
        // Save is visible → the positive waiter is satisfied → the negated
        // assertion is exactly not satisfied yet.
        let mut waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Visible],
        )));
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
    }

    #[test]
    fn not_waiter_satisfies_when_the_positive_condition_does_not_hold() {
        let (tree, _) = setup();
        // "Ghost" matches nothing → to_be_visible pends → not(visible)
        // holds right now, with no node to hand back.
        let mut waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Ghost"),
            vec![Check::Visible],
        )));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(None) => {}
            other => panic!("absence satisfies the negated visibility, got {other:?}"),
        }
    }

    #[test]
    fn not_waiter_flips_when_the_world_flips() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        let mut waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Visible],
        )));
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
        // The app removes Save: the next event satisfies the negation.
        tree.apply_update(fixture.remove_save_update());
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(None) => {}
            other => panic!("the removal satisfies not(visible), got {other:?}"),
        }
    }

    #[test]
    fn not_hidden_on_an_absent_node_keeps_waiting() {
        // Absent IS hidden, so not(hidden) cannot hold yet: the wrapper
        // inverts the inner `Satisfied(None)` into a pending wait.
        let tree = TreeService::new();
        let mut waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Ghost"),
            vec![Check::Hidden],
        )));
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
    }

    #[test]
    fn not_waiter_keeps_strict_mode_failing() {
        let (mut tree, f) = setup();
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
        // Strict mode is a resolution failure, not a state mismatch: the
        // negation must not launder it into a pass (Playwright parity).
        let mut waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Visible],
        )));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Failed(failure) => assert_eq!(failure.check, "strict mode"),
            other => panic!("strict violation must fail under not(), got {other:?}"),
        }
    }

    #[test]
    fn not_waiter_keeps_the_fail_fast_checked_rule() {
        let (tree, _) = setup();
        // A plain button can never expose a checked state: not(checked)
        // cannot pass either — the world cannot change it (the M20 rule).
        let mut waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Checked(accesskit::Toggled::True)],
        )));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Failed(failure) => {
                assert!(failure.reason.contains("no checked state"), "{failure:?}");
            }
            other => panic!("the non-checkable role must fail fast, got {other:?}"),
        }
    }

    #[test]
    fn not_count_satisfies_while_the_count_differs() {
        let (mut tree, f) = setup();
        let mut waiter =
            NotWaiter::new(Box::new(CountWaiter::new(Selector::role(Role::Button), 5)));
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Satisfied(None) => {}
            other => panic!("4 buttons is not 5, the negation holds, got {other:?}"),
        }
        // The fifth button lands: the count matches and the negation flips
        // to pending.
        tree.apply_update(f.add_button_update("Export"));
        waiter
            .on_event(WaiterEvent::Initial, &tree)
            .expect_pending();
    }

    #[test]
    fn not_waiter_describes_the_inversion() {
        let waiter = NotWaiter::new(Box::new(ChecksWaiter::new(
            Selector::role_named(Role::Button, "Save"),
            vec![Check::Visible],
        )));
        assert_eq!(
            waiter.describe(),
            "not (getByRole(Button, \"Save\" (fuzzy)): visible)"
        );
    }

    #[test]
    fn not_waiter_is_debuggable_through_the_inner_description() {
        let waiter = NotWaiter::new(Box::new(CountWaiter::new(Selector::role(Role::Button), 5)));
        let rendered = format!("{waiter:?}");
        assert!(rendered.contains("count 5"), "{rendered}");
    }
}
