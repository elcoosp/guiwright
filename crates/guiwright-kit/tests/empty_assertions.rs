//! The empty assertion (§16 assertions row, M28's honest list): the
//! label/value composite read where tests live — the kit's locator
//! surface. The mock adapter records nothing here on purpose: these are
//! pure reads of the tree, so the tests pin the assertion (and its
//! `not()` composition) against the fixture's empty region and the
//! failure wording end-to-end.
//!
//! Plan pin (docs/m28-assertions-editable.md §5, remaining gaps):
//! *"`to_be_empty` (the third Playwright state assertion in this family:
//! no text and no value) is a label/value composite, not an effective-
//! state predicate — it lands on its own milestone if a suite asks for
//! it, riding the `HasText` machinery rather than this check's."*
//! The tests below are that ask: both facts must read empty-or-absent
//! for the assertion to hold, the same node reads `HasText` makes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

/// The empty-region fixture app, settled the way `launch_fixture` does
/// (two presented frames → the stability baseline + confirmation).
fn empty_app(test_name: &str) -> App {
    let fixture = guiwright_core::test_util::Fixture::with_empty();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    App::launch(test_name, adapter)
}

#[test]
fn an_empty_region_asserts_empty() {
    let mut app = empty_app("empty_ok");
    guiwright_kit::expect(app.role_locator(Role::GenericContainer))
        .to_be_empty()
        .expect("the region carries no text and no value");
}

#[test]
fn a_labeled_node_is_not_empty() {
    let mut app = empty_app("empty_label");
    guiwright_kit::expect(app.get_by_role(Role::Label, "Welcome, Ada"))
        .not()
        .to_be_empty()
        .expect("a label is text");
    let err = guiwright_kit::expect(app.get_by_role(Role::Label, "Welcome, Ada"))
        .with_timeout(Duration::from_millis(250))
        .to_be_empty()
        .expect_err("a labeled node is not empty");
    let msg = err.to_string();
    // The header names the assertion and the locator (the M20 wrong-state
    // convention — ChecksWaiter's timeout shape is unchanged since M22).
    assert!(msg.contains("empty"), "{msg}");
    assert!(msg.contains("Welcome, Ada"), "{msg}");
}

#[test]
fn a_valued_input_is_not_empty() {
    let mut app = empty_app("empty_value");
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .not()
        .to_be_empty()
        .expect("a carried value is content");
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .with_timeout(Duration::from_millis(250))
        .to_be_empty()
        .expect_err("a valued input is not empty");
    let msg = err.to_string();
    assert!(msg.contains("empty"), "{msg}");
    assert!(msg.contains("Email"), "{msg}");
}

#[test]
fn a_named_button_is_not_empty_too() {
    // The composite is total over every node: an accessible name is
    // content the same way a label is, so a button never reads empty —
    // the honest shape `expect(button).not().toBeEmpty()` has in
    // Playwright.
    let mut app = empty_app("empty_button");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_be_empty()
        .expect("an accessible name is content");
}

#[test]
fn a_label_arriving_completes_the_not_empty_wait() {
    // The scripted channel emulates the app painting text into the
    // region 100 ms in; the assertion completes on that tree-diff event,
    // never by polling.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = guiwright_core::test_util::Fixture::with_empty();
    let region = fixture.empty_region.expect("empty fixture");
    let paint = fixture
        .region_label_update(region, Some("Loaded"))
        .expect("the region is in the fixture");
    let mut adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("empty_flip", adapter);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(paint);
    });
    guiwright_kit::expect(app.role_locator(Role::GenericContainer))
        .not()
        .to_be_empty()
        .expect("the region stops being empty on the scripted diff");
}

#[test]
fn an_empty_region_stays_empty_through_the_wait() {
    // The positive direction over the same scripted channel: the region
    // starts empty and stays empty, so the assertion completes on the
    // initial evaluation — no update scripted at all.
    let mut app = empty_app("empty_steady");
    guiwright_kit::expect(app.role_locator(Role::GenericContainer))
        .to_be_empty()
        .expect("an unlabeled, valueless region reads empty");
}
