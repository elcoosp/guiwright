//! The identity-state read at the kit surface (M68): the debugging read
//! beside the assertion vocabulary — `describe_state()` answers "what is
//! this node **doing**" (focused, enabled, the checked tri-state) where
//! `describe()` answers "what IS the node". The facts are total reads,
//! never waits (the m68 rule): the words answer right now; the waitable
//! face stays `expect(locator).to_be_checked()` and friends. The wire's
//! `DescribeIdentityState` domain (M68) serves the same row to remote
//! clients.
//!
//! ```text
//! cargo run -p guiwright-kit --example describe_state_facts
//! ```
//!
//! Reads the checkbox trio's tri-state (the vocabulary word, never an
//! `Option<bool>`), a button's `NotCheckable` fact (never an error), and
//! prints the focused fact, then leaves a `gwtrace/v1` archive of the
//! session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The checkbox-trio fixture (M20): checked / unchecked / mixed, any
    // real adapter exposing its AccessKit tree drops in unchanged.
    let mut adapter =
        MockAdapter::with_fixture(&guiwright_core::test_util::Fixture::with_checkboxes());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("describe_state_facts_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("describe_state_facts_demo.zip"));
    app.wait_for_settle();

    // One node's state three beside the identity row: the tri-state as
    // the wire's four-way word — a mixed checkbox is not an unchecked
    // one (the m68 rule).
    for (name, expected) in [
        ("Notifications", "checked"),
        ("Dark mode", "unchecked"),
        ("Partially installed", "mixed"),
    ] {
        let row = app
            .get_by_role(Role::CheckBox, name)
            .describe_state()
            .expect("the read");
        println!(
            "{name} — checked: {:?} (expected {expected}), enabled: {}, focused: {}",
            row.checked, row.enabled, row.focused
        );
    }

    // Not a checkable role: the fact answers `NotCheckable`, never an
    // error — the total read the assertion vocabulary refuses fast.
    let button = app
        .get_by_role(Role::Button, "Save")
        .describe_state()
        .expect("the read");
    println!(
        "Save — checked: {:?}, enabled: {}, focused: {}",
        button.checked, button.enabled, button.focused
    );
}
