//! The editable/read-only assertion surface (§16 assertions row, M28): the
//! consumer's effective read-only state — the AccessKit `read_only` flag
//! plus the role rules the consumer applies (`should_have_read_only_
//! state_by_default` and the read-only-supported set) — read where tests
//! live: `expect(locator).to_be_editable()` / `.to_be_readonly()`, over
//! the same event-driven wait as every other `to_*` assertion (no
//! polling; the same §8.3 vocabulary the actionability gate speaks).
//!
//! ```text
//! cargo run -p guiwright-kit --example editable_assertions
//! ```
//!
//! Asserts both directions against the fixture's input pair (and a
//! button, for the effective-state rule), prints the failure shape the
//! waitable path produces, then leaves a `gwtrace/v1` archive of the
//! session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The read-only fixture: the M0 buttons fixture plus "Bio" (an
    // editable text input) and "Serial" (a text input with the AccessKit
    // `read_only` flag). Any real adapter exposing editable state through
    // its AccessKit tree drops in unchanged.
    let fixture = guiwright_core::test_util::Fixture::with_readonly();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("editable_assertions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("editable_assertions_demo.zip"));
    app.wait_for_settle();

    // The two directions (§8 check vocabulary, M28). Each completes
    // exactly when the state holds — here, on the initial evaluation,
    // because the fixture already carries its final states.
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Bio"))
        .to_be_editable()
        .expect("bio is editable");
    println!("to_be_editable   → ok (Bio has no read_only flag)");

    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .to_be_readonly()
        .expect("serial is read-only");
    println!("to_be_readonly   → ok (Serial is set_read_only)");

    // The effective-state rule: the consumer's read-only state is total
    // over every node, so a button (no editable value) reads read-only —
    // exactly the shape `expect(button).not().toBeEditable()` has in
    // Playwright.
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_be_readonly()
        .expect("a button carries no editable value");
    println!("to_be_readonly   → ok (a button reports read-only)");

    // The negation modifier composes (M24): `not().to_be_editable()` on
    // the read-only input holds right now, exactly like the direct
    // `to_be_readonly()`.
    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .not()
        .to_be_editable()
        .expect("serial is not editable");
    println!("not.to_be_editable → ok (Serial stays read-only)");

    // The failure shape: a state that has not flipped yet waits (retry
    // until the state holds or the timeout expires) and names the
    // assertion and the locator in the error header.
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Serial"))
        .with_timeout(std::time::Duration::from_millis(300))
        .to_be_editable()
        .expect_err("serial has not flipped");
    println!("\nfailure shape (waitable): {err}");

    // The trace: the session's event log names every assertion step.
    let trace = app.finish();
    if let Some(path) = trace {
        println!("\ntrace: {}", path.display());
    }
}
