//! The count comparators (§16 assertions row, M25): `expect(locator)
//! .to_have_count_greater_than(n)` / `.to_have_count_less_than(n)` — the
//! M22 record's reserved vocabulary widening on the count waiter. The
//! tests pin the comparators end-to-end on the mock adapter: satisfied on
//! the demanded side of the bound, waitable across scripted world changes
//! in both directions, the timeout naming the comparator and the count
//! the last evaluation found, composing with the M24 negation decorator
//! and scoping through the window handle.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

#[test]
fn greater_than_passes_when_the_set_clears_the_bound() {
    let mut app = App::launch_fixture("compare_gt_immediate");
    // Settle: the fixture's four rendered buttons are the set (an empty
    // pre-render tree would make any comparator vacuously true).
    app.wait_for_settle();
    expect(app.role_locator(Role::Button))
        .to_have_count_greater_than(3)
        .expect("4 buttons clear the > 3 bound");
}

#[test]
fn less_than_passes_when_the_set_sits_below_the_bound() {
    let mut app = App::launch_fixture("compare_lt_immediate");
    app.wait_for_settle();
    expect(app.role_locator(Role::Button))
        .to_have_count_less_than(5)
        .expect("4 buttons sit below the < 5 bound");
    // Zero matches sit below any positive bound too (strict mode never
    // consulted — the whole match set is the answer).
    expect(app.get_by_role(Role::Button, "Ghost"))
        .to_have_count_less_than(1)
        .expect("0 ghosts sit below the < 1 bound");
}

#[test]
fn greater_than_completes_when_a_node_late_appears() {
    // 4 buttons now; the export button streams in and the set clears the
    // > 4 bound on that tree diff (§15.3 — never by polling).
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let late = fixture.add_button_update("Export");
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("compare_gt_late_node", adapter);
    app.wait_for_settle();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(late);
    });
    expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_secs(2))
        .to_have_count_greater_than(4)
        .expect("the export button clears the > 4 bound");
}

#[test]
fn less_than_completes_when_a_node_leaves() {
    // The mismatch is waitable in the narrowing direction too: the app
    // removes its Save button 100 ms in and the set drops below < 4.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let removal = fixture.remove_save_update();
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("compare_lt_removal", adapter);
    app.wait_for_settle();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(removal);
    });
    expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_secs(2))
        .to_have_count_less_than(4)
        .expect("the removal drops the set below < 4");
}

#[test]
fn a_comparator_timeout_names_the_comparator_and_the_found_count() {
    let mut app = App::launch_fixture("compare_timeout");
    app.wait_for_settle();
    let err = expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_millis(250))
        .to_have_count_greater_than(99)
        .expect_err("4 buttons never clear > 99");
    let msg = err.to_string();
    assert!(msg.contains("count > 99"), "{msg}");
    assert!(msg.contains("getByRole"), "{msg}");
    assert!(msg.contains("found 4"), "{msg}");
}

#[test]
fn negated_comparators_compose_through_the_decorator() {
    // The M24 seam wraps any waiter, the comparator included: not(> 99)
    // holds right now (4 does not clear it)…
    let mut app = App::launch_fixture("compare_negated");
    app.wait_for_settle();
    expect(app.role_locator(Role::Button))
        .not()
        .to_have_count_greater_than(99)
        .expect("4 buttons do not clear > 99");
    // …and not(> 3) never lands while the set stays at 4: the timeout
    // header names the inverted comparator.
    let err = expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_millis(250))
        .not()
        .to_have_count_greater_than(3)
        .expect_err("4 buttons keep clearing > 3");
    assert!(
        err.to_string().contains("not (count > 3"),
        "{}",
        err.to_string()
    );
}

#[test]
fn comparators_scope_through_the_window_handle() {
    // The two-window fixture carries a "Save" per window: the unscoped
    // set is 2 (clears > 1), the subtree-scoped set is 1 (does not).
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("compare_window_scoped", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    expect(app.get_by_role(Role::Button, "Save"))
        .to_have_count_greater_than(1)
        .expect("both windows carry a Save: 2 > 1");
    expect(
        app.window(&w2)
            .expect("w2")
            .get_by_role(Role::Button, "Save"),
    )
    .to_have_count_less_than(2)
    .expect("the pin scopes the set to 1 < 2");
}
