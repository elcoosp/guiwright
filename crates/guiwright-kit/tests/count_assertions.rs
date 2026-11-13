//! The count surface (§16 assertions row, M22): `expect(locator).to_have_count(n)`
//! and `locator.count()` — Playwright's escape hatch from strict mode. The
//! tests pin the selector-level assertion end-to-end on the mock adapter:
//! the whole match set is a valid answer (strict mode never consulted), the
//! count is waitable in both directions (nodes streaming in and out), the
//! timeout names the expected count, the locator and the count the last
//! evaluation found, window pins scope it, and the sync read records no
//! codegen line.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

#[test]
fn count_matches_immediately_on_the_whole_match_set() {
    let mut app = App::launch_fixture("count_immediate");
    // Four buttons would trip strict mode for every node-level assertion;
    // count talks about the set (Playwright's toHaveCount).
    expect(app.role_locator(Role::Button))
        .to_have_count(4)
        .expect("the fixture ships 4 buttons");
    expect(app.get_by_role(Role::Button, "Ghost"))
        .to_have_count(0)
        .expect("zero matches is a count too");
}

#[test]
fn count_completes_when_a_node_late_appears() {
    // The scripted channel emulates the app rendering the button 100 ms
    // in; the assertion completes on that tree-diff event, never by
    // polling (§15.3).
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let late = fixture.add_button_update("Export");
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("count_late_node", adapter);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(late);
    });
    expect(app.role_locator(Role::Button))
        .to_have_count(5)
        .expect("the export button lands the fifth");
}

#[test]
fn count_waits_for_a_removal_too() {
    // The mismatch is waitable in both directions: the app removes its
    // Save button 100 ms in and the count drops to 3.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let removal = fixture.remove_save_update();
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("count_removal", adapter);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(removal);
    });
    expect(app.role_locator(Role::Button))
        .to_have_count(3)
        .expect("the removal lands the third");
}

#[test]
fn a_count_timeout_names_the_expected_count_and_the_locator() {
    let mut app = App::launch_fixture("count_timeout");
    let err = expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_millis(250))
        .to_have_count(99)
        .expect_err("99 buttons never arrive");
    let msg = err.to_string();
    assert!(msg.contains("count 99"), "{msg}");
    assert!(msg.contains("getByRole"), "{msg}");
    assert!(msg.contains("found 4"), "{msg}");
}

#[test]
fn locator_count_reads_the_tree_without_strict_mode() {
    let mut app = App::launch_fixture("count_read");
    assert_eq!(
        app.role_locator(Role::Button).count().expect("count"),
        4,
        "the whole match set, no strict check"
    );
    assert_eq!(
        app.get_by_role(Role::Button, "Ghost")
            .count()
            .expect("count"),
        0,
        "reads are repeatable and absence is a count"
    );
}

#[test]
fn count_is_window_scoped_through_the_handle() {
    // The two-window fixture carries a "Save" per window (independent
    // subtree namespaces): unscoped the set is 2, scoped to one window's
    // subtree it is 1 — the same discrimination the strict surface has.
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("count_window_scoped", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    expect(app.get_by_role(Role::Button, "Save"))
        .to_have_count(2)
        .expect("both windows carry a Save");
    expect(
        app.window(&w2)
            .expect("w2")
            .get_by_role(Role::Button, "Save"),
    )
    .to_have_count(1)
    .expect("the pin scopes the count to Preview's subtree");
    assert_eq!(
        app.window(&w2)
            .expect("w2")
            .get_by_role(Role::Button, "Save")
            .count()
            .expect("scoped read"),
        1
    );
}

#[test]
fn count_records_no_codegen_line() {
    let mut app = App::launch_fixture("count_codegen");
    app.enable_codegen();
    app.role_locator(Role::Button).count().expect("count");
    let (code, warnings) = app
        .codegen_code()
        .expect("the recorder exists once enabled");
    assert!(
        code.trim().is_empty(),
        "reads are not replayable steps (resolution without an action records nothing): {code}"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}
