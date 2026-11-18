//! The node-describe surface (§8.1's identity facts, M36): the debugging
//! read beside the bulk readers — `describe()` answers "what IS the node
//! I just resolved" (role, accessible name, value, bounds) where
//! `all_text_contents()` answers "what does the set say". Over the same
//! embedded session loop as every locator read; the wire's `Describe`
//! domain (M36) serves the same row to remote clients.
//!
//! ```text
//! cargo run -p guiwright-kit --example describe_facts
//! ```
//!
//! Reads the resolved node's facts, the value an input carries, and
//! prints the strict-mode failure shape, then leaves a `gwtrace/v1`
//! archive of the session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The M0 buttons fixture: any real adapter exposing its AccessKit
    // tree drops in unchanged — the facts read straight off the tree.
    let mut adapter = MockAdapter::with_fixture(&guiwright_core::test_util::Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("describe_facts_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("describe_facts_demo.zip"));
    app.wait_for_settle();

    // One node's identity facts (the dump row's facts, one node).
    let row = app
        .get_by_role(Role::Button, "Save")
        .describe()
        .expect("the read");
    println!(
        "describe → {} {:?} (generation {}) rect={:?}",
        row.role,
        row.name.as_deref().unwrap_or("(unnamed)"),
        row.generation,
        row.bounds
    );

    // The value rides the row: the fact the `get_by_value` locators
    // match (§8.1).
    let input = app
        .get_by_role(Role::TextInput, "Email")
        .describe()
        .expect("the read");
    println!(
        "describe → {} {:?} value={:?}",
        input.role,
        input.name.as_deref().unwrap_or("(unnamed)"),
        input.value.as_deref().unwrap_or("(none)")
    );

    // The read resolves through strict mode like every locator read:
    // an ambiguous locator refuses, narrowing picks the row.
    let ambiguous = app.role_locator(Role::Button).describe();
    assert!(ambiguous.is_err(), "strict mode refuses the ambiguous set");
    let first = app
        .role_locator(Role::Button)
        .first()
        .describe()
        .expect("the narrowed read");
    println!(
        "describe (first) → {} {:?}",
        first.role,
        first.name.as_deref().unwrap_or("(unnamed)")
    );

    // A read, not a resolve-store: nothing was recorded, `get`/`hit`
    // keep aiming where they aimed (the m35 read rule).

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
