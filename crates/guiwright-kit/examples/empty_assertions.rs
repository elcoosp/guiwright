//! The empty assertion (§16 assertions row, m28's honest list): the
//! label/value composite — `expect(locator).to_be_empty()` — read where
//! tests live, over the same event-driven wait as every other `to_*`
//! assertion (no polling; the same §8.3 vocabulary the actionability gate
//! speaks).
//!
//! Plan pin (docs/m28-assertions-editable.md §5, remaining gaps):
//! *"`to_be_empty` (the third Playwright state assertion in this family:
//! no text and no value) is a label/value composite, not an effective-
//! state predicate — it lands on its own milestone if a suite asks for
//! it, riding the `HasText` machinery rather than this check's."*
//!
//! ```text
//! cargo run -p guiwright-kit --example empty_assertions
//! ```
//!
//! Asserts both directions against the fixture's factless region (and a
//! labeled node, a valued input and a named button for the composite
//! rule), prints the failure shape the waitable path produces, then
//! leaves a `gwtrace/v1` archive of the session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The empty fixture: the M0 buttons fixture plus one bounded, visible
    // container that carries no label and no value. Any real adapter
    // exposing AccessKit trees drops in unchanged.
    let fixture = guiwright_core::test_util::Fixture::with_empty();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("empty_assertions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("empty_assertions_demo.zip"));
    app.wait_for_settle();

    // The factless region reads empty: its label and value are both
    // absent — the same node reads `to_have_text` makes, both empty.
    guiwright_kit::expect(app.role_locator(Role::GenericContainer))
        .to_be_empty()
        .expect("the region carries no text and no value");
    println!("to_be_empty      → ok (the region has no label and no value)");

    // The composite is total over every node: a label is text, a carried
    // value is content, an accessible name is content — each reads
    // not-empty exactly as Playwright's `not().toBeEmpty()` does.
    guiwright_kit::expect(app.get_by_role(Role::Label, "Welcome, Ada"))
        .not()
        .to_be_empty()
        .expect("a label is text");
    println!("not.to_be_empty  → ok (Welcome, Ada is a label)");

    guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .not()
        .to_be_empty()
        .expect("a carried value is content");
    println!("not.to_be_empty  → ok (Email carries a value)");

    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_be_empty()
        .expect("an accessible name is content");
    println!("not.to_be_empty  → ok (Save carries its name)");

    // The failure shape: content that has not cleared yet waits (retry
    // until the composite holds or the timeout expires) and names the
    // assertion and the locator in the error header.
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, "Email"))
        .with_timeout(std::time::Duration::from_millis(300))
        .to_be_empty()
        .expect_err("email keeps its value");
    println!("\nfailure shape (waitable): {err}");

    // The trace: the session's event log names every assertion step.
    let trace = app.finish();
    if let Some(path) = trace {
        println!("\ntrace: {}", path.display());
    }
}
