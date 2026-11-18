//! The editable/read-only assertion surface (§16 assertions row, M28):
//! the consumer's effective read-only state (`is_read_only()` — the
//! AccessKit `read_only` flag, the role defaults and the read-only-
//! supported set) read where tests live — the kit's locator surface. The
//! mock adapter records nothing here on purpose: these are pure reads of
//! the tree, so the tests pin the two assertions (and their `not()`
//! composition) against the fixture's input pair and the failure wording
//! end-to-end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

/// The read-only fixture app, settled the way `launch_fixture` does (two
/// presented frames → the stability baseline + confirmation).
fn readonly_app(test_name: &str) -> App {
    let fixture = guiwright_core::test_util::Fixture::with_readonly();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    App::launch(test_name, adapter)
}

#[test]
fn the_editable_input_asserts_editable() {
    let mut app = readonly_app("editable_ok");
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Bio"))
        .to_be_editable()
        .expect("bio is editable");
}

#[test]
fn the_readonly_input_asserts_readonly() {
    let mut app = readonly_app("readonly_ok");
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .to_be_readonly()
        .expect("serial is read-only");
}

#[test]
fn negation_inverts_the_editable_check() {
    // The M24 decorator composes: `not().to_be_editable()` on the
    // read-only input passes, `not().to_be_readonly()` on the editable
    // input passes — the same predicate, inverted.
    let mut app = readonly_app("editable_negated");
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .not()
        .to_be_editable()
        .expect("serial is not editable");
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Bio"))
        .not()
        .to_be_readonly()
        .expect("bio is not read-only");
}

#[test]
fn a_wrong_editable_state_is_waitable_and_names_the_header() {
    let mut app = readonly_app("editable_timeout");
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .with_timeout(Duration::from_millis(250))
        .to_be_editable()
        .expect_err("serial stays read-only");
    let msg = err.to_string();
    // The header names the assertion and the locator (the M20 wrong-state
    // convention; the pending reason rides the waiter's describe, not the
    // header — ChecksWaiter's timeout shape is unchanged since M22).
    assert!(msg.contains("editable"), "{msg}");
    assert!(msg.contains("Serial"), "{msg}");
}

#[test]
fn a_wrong_readonly_state_is_waitable_too() {
    let mut app = readonly_app("readonly_timeout");
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Bio"))
        .with_timeout(Duration::from_millis(250))
        .to_be_readonly()
        .expect_err("bio stays editable");
    let msg = err.to_string();
    assert!(msg.contains("read-only"), "{msg}");
    assert!(msg.contains("Bio"), "{msg}");
}

#[test]
fn a_readonly_flip_completes_the_wait() {
    // The scripted channel emulates the app clearing the input's
    // read-only flag 100 ms in; the assertion completes on that tree-diff
    // event, never by polling.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = guiwright_core::test_util::Fixture::with_readonly();
    let serial = fixture.readonly_input.expect("readonly fixture");
    let flip = fixture
        .input_readonly_update(serial, false)
        .expect("serial is in the pair");
    let mut adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("editable_flip", adapter);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(flip);
    });
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .to_be_editable()
        .expect("serial becomes editable on the scripted diff");
}

#[test]
fn a_button_reads_read_only_not_editable() {
    // The predicate is the consumer's effective read-only state, which is
    // total over every node: a button does not support read-only, so it
    // reports read-only (there is no editable value on it) — exactly the
    // shape `expect(button).not().toBeEditable()` has in Playwright.
    // Both directions stay waitable, never fail-fast; the timeout header
    // names the assertion and the locator (the M20 convention).
    let mut app = readonly_app("editable_button");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_be_readonly()
        .expect("a button carries no editable value");
    let err = guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_millis(250))
        .to_be_editable()
        .expect_err("a button is not editable");
    assert!(err.to_string().contains("editable"), "{err}");
}
