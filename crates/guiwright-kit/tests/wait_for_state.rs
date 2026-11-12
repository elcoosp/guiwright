//! The locator wait-for surface (§16 auto-waiting row, M21):
//! Playwright's `locator.waitFor(state)` — explicit one-shot waits over
//! the session's event-driven loop (§15.3, zero polling). The tests pin
//! the four-state vocabulary end-to-end on the mock adapter: immediate
//! states complete on the initial evaluation, a late-appearing node
//! completes on the scripted tree diff, and the timeout names the state
//! and the locator.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, WaitState};

#[test]
fn immediate_states_complete_on_the_initial_evaluation() {
    let mut app = App::launch_fixture("wait_for_states");
    app.get_by_role(Role::Button, "Save")
        .wait_for(WaitState::Visible)
        .expect("save is visible");
    app.get_by_role(Role::Button, "Invisible")
        .wait_for(WaitState::Hidden)
        .expect("invisible button is hidden");
    app.get_by_role(Role::Button, "Save")
        .wait_for(WaitState::Attached)
        .expect("save is attached");
}

#[test]
fn stable_completes_on_two_presented_frames() {
    // Two frames feed the settle at launch; two more stay scripted and
    // arrive during the wait — the §8.3 Stable tracker needs exactly two
    // bounds-identical FramePresented events.
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("wait_for_stable", adapter);
    app.get_by_role(Role::Button, "Save")
        .wait_for(WaitState::Stable)
        .expect("save is stable across two frames");
}

#[test]
fn visible_completes_when_the_node_late_appears() {
    // The scripted channel emulates the app rendering the button 100 ms
    // in; the wait completes on that tree-diff event, never by polling.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let adapter = MockAdapter::with_fixture(&guiwright_core::test_util::Fixture::buttons())
        .with_shared_scripts(scripts.clone());
    let mut app = App::launch("wait_for_late_node", adapter);
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let late = fixture.add_button_update("Export");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(late);
    });
    app.get_by_role(Role::Button, "Export")
        .wait_for(WaitState::Visible)
        .expect("export appears and becomes visible");
}

#[test]
fn a_timeout_names_the_state_and_the_locator() {
    let mut app = App::launch_fixture("wait_for_timeout");
    let err = app
        .get_by_role(Role::Button, "Ghost")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_millis(250))
        .expect_err("the ghost button never becomes visible");
    let msg = err.to_string();
    assert!(msg.contains("visible"), "{msg}");
    assert!(msg.contains("Ghost"), "{msg}");
}

#[test]
fn wait_for_records_a_codegen_line() {
    let mut app = App::launch_fixture("wait_for_codegen");
    app.enable_codegen();
    app.get_by_role(Role::Button, "Save")
        .wait_for(WaitState::Visible)
        .expect("save is visible");
    let (code, warnings) = app.codegen_code().expect("codegen recorded");
    assert!(
        warnings.is_empty(),
        "the fixture resolves unambiguously: {warnings:?}"
    );
    assert!(code.contains(".wait_for();"), "{code}");
}
