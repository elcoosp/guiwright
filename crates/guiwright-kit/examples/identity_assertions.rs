//! The identity-assertion surface (§16 assertions row, M38): the
//! accessible-identity reads — `expect(locator).to_have_role(...)`,
//! `.to_have_accessible_name(...)` and `.to_have_accessible_description(...)`
//! — over the same event-driven wait as every other `to_*` assertion (no
//! polling; the same §8.3 vocabulary the actionability gate speaks).
//!
//! ```text
//! cargo run -p guiwright-kit --example identity_assertions
//! ```
//!
//! The family is deliberately narrower than `to_have_text` (M0's
//! label-or-value read): the accessible name matches `label()` only and the
//! accessible description matches `description()` only, so a text input's
//! *value* never satisfies a name assertion. Asserts the three rows against
//! the fixture, prints the failure shape the waitable paths produce, then
//! leaves a `gwtrace/v1` archive of the session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The described fixture: the M0 buttons fixture plus "Install", a
    // button whose AccessKit node carries an accessible description. Any
    // real adapter exposing roles/labels/descriptions through its
    // AccessKit tree drops in unchanged.
    let fixture = guiwright_core::test_util::Fixture::with_description();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("identity_assertions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("identity_assertions_demo.zip"));
    app.wait_for_settle();

    // The role row (§8 check vocabulary, M38). Completes exactly when the
    // role holds — here, on the initial evaluation.
    guiwright_kit::expect(app.get_by_role(Role::Button, "Install"))
        .to_have_role(Role::Button)
        .expect("install is a button");
    println!("to_have_role                  → ok (Install is Role::Button)");

    // The accessible-name row: the label only — never the value.
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .to_have_accessible_name("Email")
        .expect("the label is the accessible name");
    println!("to_have_accessible_name       → ok (the Email input's label)");

    // The accessible-description row.
    guiwright_kit::expect(app.get_by_role(Role::Button, "Install"))
        .to_have_accessible_description("Installs the latest build")
        .expect("the description rides the described node");
    println!("to_have_accessible_description → ok (Install's description)");

    // The distinction that names the family: the email input's *value*
    // ("ada@example.dev") satisfies `to_have_text` (label-or-value) but
    // not the accessible name — the identity read stays identity-only.
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .with_timeout(std::time::Duration::from_millis(300))
        .to_have_accessible_name("ada@example.dev")
        .expect_err("the value is not the accessible name");
    println!("\nfailure shape (name ≠ value): {err}");

    // The family composes with the M24 negation modifier like every
    // other `to_*`: satisfied while the identity does not hold.
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_role(Role::CheckBox)
        .expect("save is not a checkbox");
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_accessible_description("Installs the latest build")
        .expect("save carries no such description");
    println!("not() composition             → ok (Save is not a checkbox, carries no description)");

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
