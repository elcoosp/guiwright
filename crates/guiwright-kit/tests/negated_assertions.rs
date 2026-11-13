//! The negated assertion surface (§16 assertions row, M24):
//! `expect(locator).not().to_*` — Playwright's `not` modifier. The tests
//! pin the decorator end-to-end on the mock adapter: the negation holds
//! whenever the positive condition does not (absent, hidden, disabled),
//! completes event-driven on a scripted world change, still fails on
//! strict-mode violations and on roles that can never expose the checked
//! state, and the timeout header names the inverted assertion.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

#[test]
fn not_visible_passes_for_absent_and_hidden() {
    let mut app = App::launch_fixture("not_visible_absent_hidden");
    // Settle first: the negatives below are evaluated against the rendered
    // fixture (an empty pre-render tree would satisfy any not() vacuously).
    app.wait_for_settle();
    // The ghost never attached: absence is not visible (Playwright parity
    // for `expect(locator).not.toBeVisible()`).
    expect(app.get_by_role(Role::Button, "Ghost"))
        .not()
        .to_be_visible()
        .expect("the absent button is not visible");
    // The filter-excluded button is not visible either.
    expect(app.get_by_role(Role::Button, "Invisible"))
        .not()
        .to_be_visible()
        .expect("the hidden button is not visible");
}

#[test]
fn not_enabled_passes_for_the_disabled_button() {
    let mut app = App::launch_fixture("not_enabled_disabled");
    app.wait_for_settle();
    expect(app.get_by_role(Role::Button, "Confirm"))
        .not()
        .to_be_enabled()
        .expect("confirm is not enabled (disabled)");
}

#[test]
fn not_visible_completes_on_a_scripted_removal() {
    // The Save button is visible now; the app removes it 100 ms in. The
    // negated assertion completes on that tree-diff event, never by
    // polling (§15.3) — the decorator rides the same zero-polling loop.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let removal = fixture.remove_save_update();
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("not_visible_removal", adapter);
    // Settle: Save must be visible before the test waits for its removal.
    app.wait_for_settle();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(removal);
    });
    expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_secs(2))
        .not()
        .to_be_visible()
        .expect("the removal lands and the negation holds");
}

#[test]
fn not_count_waits_until_the_count_moves() {
    // 4 buttons now; the export button streams in and the count leaves 4 —
    // the negated count is waitable in the same direction as the count
    // itself (M22's seam, inverted).
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let late = fixture.add_button_update("Export");
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("not_count_move", adapter);
    // Settle: the set must be 4 before the test waits for it to move.
    app.wait_for_settle();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(late);
    });
    expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_secs(2))
        .not()
        .to_have_count(4)
        .expect("the export button moves the count off 4");
}

#[test]
fn strict_mode_still_fails_under_not() {
    let mut app = App::launch_fixture("not_strict");
    // Settle: the four rendered buttons are what make the set ambiguous.
    app.wait_for_settle();
    // Four buttons: the resolution fails before any state is evaluated —
    // the negation must not launder a strict-mode violation into a pass.
    let err = expect(app.role_locator(Role::Button))
        .not()
        .to_be_visible()
        .expect_err("four buttons is ambiguous either way");
    let msg = err.to_string();
    assert!(msg.contains("resolved to 4 nodes"), "{msg}");
}

#[test]
fn a_non_checkable_role_fails_fast_under_not() {
    let mut app = App::launch_fixture("not_checked_fail_fast");
    // Settle: the fail-fast rule fires on the rendered (non-checkable)
    // node, not on a tree that has not rendered yet.
    app.wait_for_settle();
    // A plain button never exposes a checked state: not(checked) cannot
    // wait its way to a pass (the M20 rule holds under the decorator).
    let err = expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_be_checked()
        .expect_err("a button is not checkable either way");
    let msg = err.to_string();
    assert!(msg.contains("no checked state"), "{msg}");
}

#[test]
fn a_negated_timeout_names_the_inverted_assertion() {
    let mut app = App::launch_fixture("not_timeout");
    // Settle: Save must be visible for the negation to have something to
    // wait against (a pre-render tree would satisfy it vacuously).
    app.wait_for_settle();
    // Save stays visible: not(visible) never lands, and the header must
    // carry the inversion.
    let err = expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_millis(250))
        .not()
        .to_be_visible()
        .expect_err("save stays visible");
    let msg = err.to_string();
    assert!(msg.contains("not (visible"), "{msg}");
    assert!(msg.contains("getByRole"), "{msg}");
}

#[test]
fn negated_count_scopes_through_the_window_handle() {
    // The two-window fixture carries a "Save" per window: scoped to one
    // window's subtree the set is 1, so not(count 2) holds inside the pin
    // — the window-aware assertion surface rides the same decorator.
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("not_window_scoped", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_count(1)
        .expect("unscoped the set is 2, not 1");
    expect(
        app.window(&w2)
            .expect("w2")
            .get_by_role(Role::Button, "Save"),
    )
    .not()
    .to_have_count(2)
    .expect("scoped to Preview's subtree the set is 1, not 2");
}
