//! The client-side wait tracker (m42): the pure evaluation core behind
//! [`guiwright_proto::client::ClientConn::wait_for`]. The tracker is
//! transport-blind — it only sees [`DescribedV1`] row sets (the M39
//! describe-all rows) and knows whether the wake that delivered them was
//! a tree change or a presented frame — so its whole state machine is
//! pinned here without a wire, the same way the core crate pins the §8.3
//! check table. The wire composition (subscribe → evaluate → wake →
//! re-read) is `driver/tests/wire_wait.rs`'s leg.

use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{DescribedV1, NodeKeyV1};
use guiwright_proto::wait::{Eval, WaitState, WaitTracker};

fn root() -> [u8; 16] {
    [0u8; 16]
}

fn row(node: u64, name: &str, bounds: Option<[f64; 4]>) -> DescribedV1 {
    DescribedV1 {
        node: NodeKeyV1::AccessKit { tree: root(), node },
        generation: 0,
        role: "Button".into(),
        name: Some(name.into()),
        value: None,
        bounds,
    }
}

/// The fixture's Save button rect (16,16 → 136,48, physical px).
const SAVE_BOUNDS: [f64; 4] = [16.0, 16.0, 136.0, 48.0];

#[test]
fn attached_is_satisfied_by_any_row_even_a_boundsless_one() {
    // The hidden "Invisible" button (node 6, no bounds) is still attached:
    // Playwright's attached-vs-visible distinction over the wire rows.
    let mut tracker = WaitTracker::new(WaitState::Attached);
    let rows = vec![row(6, "Invisible", None)];
    match tracker.evaluate(&rows) {
        Eval::Satisfied(Some(satisfied)) => assert_eq!(satisfied.node, rows[0].node),
        other => panic!("expected a satisfied attach, got {other:?}"),
    }
}

#[test]
fn attached_stays_pending_on_an_empty_set() {
    let mut tracker = WaitTracker::new(WaitState::Attached);
    match tracker.evaluate(&[]) {
        Eval::Pending { reason } => assert!(reason.contains("no match"), "{reason:?}"),
        other => panic!("expected pending, got {other:?}"),
    }
}

#[test]
fn visible_requires_non_empty_bounds() {
    let mut tracker = WaitTracker::new(WaitState::Visible);
    let hidden_only = vec![row(6, "Invisible", None)];
    match tracker.evaluate(&hidden_only) {
        Eval::Pending { reason } => assert!(reason.contains("no bounds"), "{reason:?}"),
        other => panic!("expected pending, got {other:?}"),
    }

    let visible = vec![row(2, "Save", Some(SAVE_BOUNDS))];
    match tracker.evaluate(&visible) {
        Eval::Satisfied(Some(satisfied)) => assert_eq!(satisfied.node, visible[0].node),
        other => panic!("expected the visible row, got {other:?}"),
    }
}

#[test]
fn visible_rejects_a_zero_area_rect() {
    // A degenerate rect ([16,16 → 16,48]: zero width) is not laid out —
    // the §8.3 area guard, client-side.
    let mut tracker = WaitTracker::new(WaitState::Visible);
    let rows = vec![row(2, "Save", Some([16.0, 16.0, 16.0, 48.0]))];
    assert!(matches!(tracker.evaluate(&rows), Eval::Pending { .. }));
}

#[test]
fn hidden_is_satisfied_by_absence_of_any_visible_row() {
    // Satisfied by absence too (Playwright parity, the kit's WaitState
    // doc): no rows at all, or only bounds-less rows.
    let mut tracker = WaitTracker::new(WaitState::Hidden);
    assert!(matches!(tracker.evaluate(&[]), Eval::Satisfied(None)));
    let hidden_only = vec![row(6, "Invisible", None)];
    assert!(matches!(
        tracker.evaluate(&hidden_only),
        Eval::Satisfied(None)
    ));

    // A visible row keeps the wait pending.
    let visible = vec![row(2, "Save", Some(SAVE_BOUNDS))];
    assert!(matches!(tracker.evaluate(&visible), Eval::Pending { .. }));
}

#[test]
fn stable_needs_two_consecutive_identical_frame_observations() {
    // The §8.3 Stable semantics (core's StabilityTracker, mirrored over
    // the wire): the tracker is frame-fed only — a tree wake re-pins but
    // never advances the count — and two consecutive identical bounds
    // across presented frames confirm.
    let mut tracker = WaitTracker::new(WaitState::Stable);
    let rows = vec![row(2, "Save", Some(SAVE_BOUNDS))];

    // Entry evaluation: the visible precondition holds, but no frame has
    // confirmed anything yet.
    assert!(matches!(tracker.evaluate(&rows), Eval::Pending { .. }));
    // First presented frame: the bounds become the baseline, not a pass.
    assert!(matches!(tracker.observe_frame(&rows), Eval::Pending { .. }));
    // Second identical frame: confirmed.
    match tracker.observe_frame(&rows) {
        Eval::Satisfied(Some(satisfied)) => assert_eq!(satisfied.node, rows[0].node),
        other => panic!("expected the stable row, got {other:?}"),
    }
}

#[test]
fn stable_resets_when_the_bounds_change() {
    let mut tracker = WaitTracker::new(WaitState::Stable);
    let save = vec![row(2, "Save", Some(SAVE_BOUNDS))];
    assert!(matches!(tracker.evaluate(&save), Eval::Pending { .. }));
    assert!(matches!(tracker.observe_frame(&save), Eval::Pending { .. }));
    assert!(matches!(tracker.observe_frame(&save), Eval::Satisfied(_)));

    // The node moves: the changed frame becomes the new baseline (the
    // core rule) — the old confirmation says nothing about the moved
    // rect, and the moved rect needs its own baseline + confirm pair.
    let moved = vec![row(2, "Save", Some([16.0, 96.0, 136.0, 128.0]))];
    match tracker.observe_frame(&moved) {
        Eval::Pending { reason } => assert!(reason.contains("next frame"), "{reason:?}"),
        other => panic!("expected a reset, got {other:?}"),
    }
    match tracker.observe_frame(&moved) {
        Eval::Satisfied(Some(satisfied)) => {
            assert_eq!(satisfied.bounds, Some([16.0, 96.0, 136.0, 128.0]))
        }
        other => panic!("expected the moved row, got {other:?}"),
    }
}

#[test]
fn stable_repins_when_the_pinned_row_leaves_the_set() {
    // A waiter resets on identity change (§8.2): the pinned node gone
    // re-pins to the set's visible row and the count starts over — the
    // tree grew a new first match, so the old confirmation says nothing
    // about the new node.
    let mut tracker = WaitTracker::new(WaitState::Stable);
    let save = vec![row(2, "Save", Some(SAVE_BOUNDS))];
    assert!(matches!(tracker.evaluate(&save), Eval::Pending { .. }));
    assert!(matches!(tracker.observe_frame(&save), Eval::Pending { .. }));

    // The set now leads with a different visible node (node 3 "Cancel").
    let changed = vec![
        row(3, "Cancel", Some([16.0, 56.0, 136.0, 88.0])),
        row(2, "Save", Some(SAVE_BOUNDS)),
    ];
    match tracker.observe_frame(&changed) {
        Eval::Pending { reason } => assert!(reason.contains("re-pinned"), "{reason:?}"),
        other => panic!("expected a re-pin, got {other:?}"),
    }
    // One identical frame on the new pin is a baseline, not a pass — the
    // reset is real, not cosmetic.
    let cancel_only = vec![row(3, "Cancel", Some([16.0, 56.0, 136.0, 88.0]))];
    assert!(matches!(
        tracker.observe_frame(&cancel_only),
        Eval::Pending { .. }
    ));
    match tracker.observe_frame(&cancel_only) {
        Eval::Satisfied(Some(satisfied)) => assert_eq!(satisfied.node, cancel_only[0].node),
        other => panic!("expected the new pin, got {other:?}"),
    }
}

#[test]
fn stable_stays_pending_without_a_visible_row() {
    let mut tracker = WaitTracker::new(WaitState::Stable);
    let hidden_only = vec![row(6, "Invisible", None)];
    match tracker.evaluate(&hidden_only) {
        Eval::Pending { reason } => assert!(reason.contains("no visible"), "{reason:?}"),
        other => panic!("expected pending, got {other:?}"),
    }
    assert!(matches!(
        tracker.observe_frame(&hidden_only),
        Eval::Pending { .. }
    ));
}

#[test]
fn states_render_their_playwright_names() {
    assert_eq!(WaitState::Attached.describe(), "attached");
    assert_eq!(WaitState::Visible.describe(), "visible");
    assert_eq!(WaitState::Hidden.describe(), "hidden");
    assert_eq!(WaitState::Stable.describe(), "stable");
}
