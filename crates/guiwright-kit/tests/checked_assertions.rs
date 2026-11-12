//! Checked-state assertions (§16 assertions row, M20): the AccessKit
//! tri-state (`Toggled`) surfaced through the §8 check vocabulary and
//! read where tests live — the kit's locator surface. The mock adapter
//! records nothing here on purpose: these are pure reads of the tree, so
//! the tests pin the three assertions against the fixture's checkbox trio
//! and the failure wording end-to-end.

use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

/// The checkbox-trio fixture app, settled the way `launch_fixture` does
/// (two presented frames → the stability baseline + confirmation).
fn checkbox_app(test_name: &str) -> App {
    let fixture = guiwright_core::test_util::Fixture::with_checkboxes();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    App::launch(test_name, adapter)
}

#[test]
fn the_three_states_assert_their_tri_state() {
    let mut app = checkbox_app("checked_trio");
    guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Notifications"))
        .to_be_checked()
        .expect("notifications is checked");
    guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Dark mode"))
        .to_be_unchecked()
        .expect("dark mode is unchecked");
    guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Partially installed"))
        .to_be_checked_mixed()
        .expect("partially installed is mixed");
}

#[test]
fn a_wrong_state_times_out_naming_the_assertion_and_locator() {
    let mut app = checkbox_app("checked_timeout");
    let err = guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Dark mode"))
        .with_timeout(Duration::from_millis(250))
        .to_be_checked()
        .expect_err("dark mode is not checked");
    let msg = err.to_string();
    assert!(msg.contains("checked"), "{msg}");
    assert!(msg.contains("Dark mode"), "{msg}");
}

#[test]
fn a_node_without_a_checked_state_fails_fast() {
    let mut app = checkbox_app("checked_fail_fast");
    // A plain button never exposes a checked state: the assertion refuses
    // immediately (fail-fast `Failed`, not the five-second default wait —
    // the generous timeout proves the fast path).
    let err = guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_secs(5))
        .to_be_checked()
        .expect_err("a button is not checkable");
    assert!(err.to_string().contains("no checked state"), "{err}");
}
