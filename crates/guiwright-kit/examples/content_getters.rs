//! The content getters (§16 actions row — the m19 record's remaining-gaps
//! list, the [`all_values`](all_values) bulk readers' single-node
//! sibling): `locator.inner_text()` reads the text the text locators
//! match (the accessible label first, then the value — the `HasText`
//! check's source order) and `locator.input_value()` reads what the
//! form carries (the value fact only — a node without one refuses,
//! [`KitError::NoValue`]). Both ride the `describe` surface: strict
//! mode, the auto-wait, the window pin — and the read rule (nothing
//! recorded, no codegen line, no trace action).
//!
//! ```text
//! cargo run -p guiwright-kit --example content_getters
//! ```
//!
//! Reads the fixture's Email input both ways (label vs value — the two
//! getters can never silently agree on the wrong datum), the Welcome
//! label, a valueless button's typed refusal, and the empty-string
//! totality on the unnamed group.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("content_getters_demo", adapter);
    app.wait_for_settle();

    // The two getters on the same node: the text locators' match vs
    // what the form carries — the m31 flip rule at single-node width.
    let text = app
        .get_by_role(Role::TextInput, "Email")
        .inner_text()
        .expect("the label-first read");
    let value = app
        .get_by_role(Role::TextInput, "Email")
        .input_value()
        .expect("the value read");
    println!("get_by_role(TextInput, \"Email\").inner_text()  → {text:?}");
    println!("get_by_role(TextInput, \"Email\").input_value() → {value:?}");

    // The label's own text: the accessible name is the inner text.
    let label = app
        .get_by_role(Role::Label, "Welcome, Ada")
        .inner_text()
        .expect("the label read");
    println!("get_by_role(Label, \"Welcome, Ada\").inner_text() → {label:?}");

    // A node without a value fact refuses: the typed refusal names the
    // locator and the missing fact — never a silent label fallback.
    let refusal = app.get_by_role(Role::Button, "Save").input_value();
    match refusal {
        Ok(v) => println!("unexpected value on a button: {v:?}"),
        Err(err) => println!("get_by_role(Button, \"Save\").input_value() → refused: {err}"),
    }

    // The totality rule: neither fact → the empty string.
    let total = app
        .role_locator(Role::Group)
        .inner_text()
        .expect("the total read");
    println!("role_locator(Group).inner_text()               → {total:?} (neither fact)");
}
