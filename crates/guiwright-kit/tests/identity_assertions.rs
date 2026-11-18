//! Identity assertions (§16 assertions row, M38): the accessible-identity
//! reads — `to_have_role`, `to_have_accessible_name`,
//! `to_have_accessible_description` — surfaced through the §8 check
//! vocabulary and read where tests live (the kit's locator surface). The
//! mock adapter records nothing here on purpose: these are pure reads of
//! the tree, so the tests pin the three assertions against the fixture's
//! identity rows and the failure wording end-to-end.
//!
//! The family is deliberately narrower than `to_have_text` (M0's label-or-
//! value read): the accessible name matches `label()` only and the
//! accessible description matches `description()` only, so a text input's
//! *value* never satisfies a name assertion and vice versa (Playwright
//! parity — `toHaveAccessibleName` is not `toHaveText`).

use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

/// The described-fixture app, settled the way `launch_fixture` does (two
/// presented frames → the stability baseline + confirmation).
fn described_app(test_name: &str) -> App {
    let fixture = guiwright_core::test_util::Fixture::with_description();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    App::launch(test_name, adapter)
}

/// The base `buttons()` fixture, settled — the role/name rows need no
/// extension band.
fn base_app(test_name: &str) -> App {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    App::launch(test_name, adapter)
}

#[test]
fn the_role_asserts_itself() {
    let mut app = base_app("role_row");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_have_role(Role::Button)
        .expect("save is a button");
    guiwright_kit::expect(app.get_by_text("Welcome, Ada"))
        .to_have_role(Role::Label)
        .expect("welcome is a label");
}

#[test]
fn a_wrong_role_times_out_naming_the_assertion_and_locator() {
    let mut app = base_app("role_timeout");
    let err = guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_millis(250))
        .to_have_role(Role::CheckBox)
        .expect_err("save is not a checkbox");
    let msg = err.to_string();
    assert!(msg.contains("role"), "{msg}");
    assert!(msg.contains("Save"), "{msg}");
}

#[test]
fn the_accessible_name_matches_the_label() {
    let mut app = base_app("name_row");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Cancel"))
        .to_have_accessible_name("Cancel")
        .expect("cancel's accessible name is exact");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Cancel"))
        .to_have_accessible_name("cance")
        .expect("the default TextMatch is substring + case-insensitive");
}

#[test]
fn the_accessible_name_never_reads_the_value() {
    let mut app = base_app("name_vs_value");
    // The email input's value ("ada@example.dev") satisfies HasText
    // (M0's label-or-value read) but not the accessible name: its label
    // is "Email". The identity read stays identity-only.
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .to_have_accessible_name("Email")
        .expect("the label is the accessible name");
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .with_timeout(Duration::from_millis(250))
        .to_have_accessible_name("ada@example.dev")
        .expect_err("the value is not the accessible name");
    let msg = err.to_string();
    assert!(msg.contains("accessible name"), "{msg}");
}

#[test]
fn the_accessible_description_matches_the_description() {
    let mut app = described_app("description_row");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Install"))
        .to_have_accessible_description("Installs the latest build")
        .expect("the description rides the described node");
}

#[test]
fn a_node_without_a_description_stays_pending_then_names_the_state() {
    let mut app = described_app("description_pending");
    // A node with no description may gain one (the world changes), so a
    // missing description is waitable — never fail-fast: the assertion
    // rides the timeout path (a fail-fast would abort early) and the
    // error names the assertion and the locator.
    let err = guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_millis(250))
        .to_have_accessible_description("anything")
        .expect_err("save carries no description");
    let msg = err.to_string();
    assert!(msg.contains("accessible description"), "{msg}");
    assert!(msg.contains("Save"), "{msg}");
}

#[test]
fn the_identity_family_composes_with_not() {
    let mut app = described_app("identity_not");
    // M24's decorator inverts every terminal assertion in the family.
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_role(Role::CheckBox)
        .expect("save is not a checkbox");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_accessible_name("Cancel")
        .expect("save is not named Cancel");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_accessible_description("Installs the latest build")
        .expect("save carries no such description");
}
