//! Event-driven waiting integration tests (architecture §8.3, §15.3).
//!
//! These tests drive waiters the way the session loop does: feed the events
//! that actually happen (tree diffs, presented frames) and assert outcomes.
//! They also pin the zero-polling contract: a waiter fed *no* events does no
//! work and never changes outcome on its own.

use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_core::actionability::{Check, action_checks};
use guiwright_core::expect::expect;
use guiwright_core::selector::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_core::tree::TreeService;
use guiwright_core::wait::{ChecksWaiter, WaitOutcome, Waiter, WaiterEvent};

/// Local helpers for readable assertions in this integration test.
trait OutcomeExt {
    fn expect_pending(&self);
    #[allow(dead_code)]
    fn expect_satisfied(self) -> Option<guiwright_core::Resolved>;
}

impl OutcomeExt for WaitOutcome {
    fn expect_pending(&self) {
        match self {
            WaitOutcome::Pending => {}
            other => panic!("expected pending, got {other:?}"),
        }
    }

    fn expect_satisfied(self) -> Option<guiwright_core::Resolved> {
        match self {
            WaitOutcome::Satisfied(r) => r,
            other => panic!("expected satisfied, got {other:?}"),
        }
    }
}

fn tree_with_fixture() -> (TreeService, Fixture) {
    let fixture = Fixture::buttons();
    let mut tree = TreeService::new();
    tree.apply_update(fixture.initial_update());
    (tree, fixture)
}

/// A waiter fed only timeout ticks (no tree/frame events) must stay pending
/// forever — that's what "no sleep-polling" means concretely.
#[test]
fn waiters_do_not_progress_without_events() {
    let (tree, _) = tree_with_fixture();
    let mut waiter = ChecksWaiter::new(
        Selector::role_named(Role::Button, "Save"),
        vec![Check::Stable],
    );
    // Simulate 100 "ticks" that are NOT frame presentations.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(5) {
        match waiter.on_event(WaiterEvent::Initial, &tree) {
            WaitOutcome::Pending => {}
            other => panic!("no-event ticks must never satisfy, got {other:?}"),
        }
    }
}

/// The auto-wait loop: the save button appears only after a diff, and the
/// waiter completes exactly at that diff (event-driven, §15.3).
#[test]
fn waiter_completes_exactly_when_the_world_changes() {
    let (mut tree, _) = tree_with_fixture();
    let mut waiter = ChecksWaiter::new(
        Selector::role_named(Role::Button, "Export"),
        vec![Check::Attached, Check::Visible],
    );
    waiter
        .on_event(WaiterEvent::Initial, &tree)
        .expect_pending();

    let fixture = Fixture::buttons();
    let diff = tree.apply_update(fixture.add_button_update("Export"));
    match waiter.on_event(WaiterEvent::TreeChanged(&diff), &tree) {
        WaitOutcome::Satisfied(Some(r)) => assert!(r.bounds.is_some()),
        other => panic!("export must satisfy at the diff that added it, got {other:?}"),
    }
}

/// Actionability for a click: hidden→visible transition satisfies without
/// any polling between diffs.
#[test]
fn click_actionability_waits_for_visible_then_frames() {
    let fixture = Fixture::buttons();
    let mut tree = TreeService::new();
    tree.apply_update(fixture.initial_update());
    tree.apply_update(fixture.hide_save_update());
    let mut waiter = ChecksWaiter::new(
        Selector::role_named(Role::Button, "Save"),
        action_checks(false),
    );
    // Hidden → fails fast? No: for actions we *wait* (Playwright waits for
    // visibility). Hidden is a `Failed` only for to_be_visible assertions...
    // For actions the check must stay Pending while the node is hidden.
    waiter
        .on_event(WaiterEvent::Initial, &tree)
        .expect_pending();

    // Un-hide the button (diff event).
    let update = {
        let mut save = accesskit::Node::new(accesskit::Role::Button);
        save.set_label("Save");
        save.set_bounds(accesskit::Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });
        accesskit::TreeUpdate {
            nodes: vec![(fixture.save_button, save)],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(0),
        }
    };
    let diff = tree.apply_update(update);
    waiter
        .on_event(WaiterEvent::TreeChanged(&diff), &tree)
        .expect_pending(); // attached/visible/enabled pass now; stable pending

    // Two frames confirm stability → satisfied.
    waiter
        .on_event(WaiterEvent::FramePresented { seq: 1, hash: 1 }, &tree)
        .expect_pending();
    match waiter.on_event(WaiterEvent::FramePresented { seq: 2, hash: 1 }, &tree) {
        WaitOutcome::Satisfied(Some(_)) => {}
        other => panic!("expected satisfied, got {other:?}"),
    }
}

/// Disabled action target keeps waiting (Playwright parity: retry until
/// enabled or timeout — the session loop owns the deadline).
#[test]
fn disabled_click_keeps_waiting() {
    let fixture = Fixture::buttons();
    let mut tree = TreeService::new();
    tree.apply_update(fixture.disable_save_update());
    let mut waiter = ChecksWaiter::new(
        Selector::role_named(Role::Button, "Save"),
        action_checks(false),
    );
    waiter
        .on_event(WaiterEvent::Initial, &tree)
        .expect_pending();
    // Once enabled (diff event), the waiter moves on to stability.
    let update = {
        let mut save = accesskit::Node::new(accesskit::Role::Button);
        save.set_label("Save");
        save.set_bounds(accesskit::Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });
        accesskit::TreeUpdate {
            nodes: vec![(fixture.save_button, save)],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(0),
        }
    };
    let diff = tree.apply_update(update);
    waiter
        .on_event(WaiterEvent::TreeChanged(&diff), &tree)
        .expect_pending();
}

/// Force mode skips stability + hit-test (§8.3 "Force mode").
#[test]
fn force_mode_satisfies_without_frames() {
    let (tree, _) = tree_with_fixture();
    let mut waiter = ChecksWaiter::new(
        Selector::role_named(Role::Button, "Save"),
        action_checks(true),
    );
    match waiter.on_event(WaiterEvent::Initial, &tree) {
        WaitOutcome::Satisfied(Some(_)) => {}
        other => panic!("force must satisfy immediately, got {other:?}"),
    }
}

/// expect().check_now() behaves on a settled tree; the timeout path carries
/// the last summary.
#[test]
fn expect_retry_semantics_report_last_state() {
    let (mut tree, _) = tree_with_fixture();
    let mut assertion = expect(Selector::role_named(Role::Button, "Export")).to_be_visible();
    let err = assertion
        .check_now(&tree)
        .expect_err("export does not exist yet");
    assert!(err.to_string().contains("8 nodes"), "{err}");

    let fixture = Fixture::buttons();
    tree.apply_update(fixture.add_button_update("Export"));
    assertion
        .check_now(&tree)
        .expect("export visible after diff");
}
