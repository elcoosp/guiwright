//! The inspector overlay surface (§13 over the kit): `app.inspect()` —
//! enable / highlight / clear / disable, the embedded half of the wire
//! domain the REPL's `inspect` command and the TypeScript client's
//! `inspectHighlight` speak. The adapter composites hover outlines +
//! interactive-role badges at present time; the highlighted node's facts
//! come back for debug prints.
//!
//! ```text
//! cargo run -p guiwright-kit --example inspect_highlight
//! ```
//!
//! Prints the facts of every highlighted locator and the plan the adapter
//! received (outline + badges), then records a `gwtrace/v1` archive of
//! the session.

use accesskit::Role;

use guiwright_core::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The M1 fixture app: a "Save" button, a "Welcome, Ada" label and an
    // "Email" text input. Any adapter with an overlay surface (egui,
    // masonry, …) drops in unchanged — the surface is the kit's.
    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("inspect_highlight_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("inspect_highlight_demo.zip"));
    app.wait_for_settle();

    // Enable the overlay session: interactive-role badges track the tree
    // every frame (labels/containers stay badge-free — §13 interactive
    // mode; the full-tree mode lands with the viewer).
    app.inspect().enable().expect("overlay on");

    // Highlight locators: strict-mode resolution + the hover outline +
    // the facts an inspector UI prints.
    let save = Selector::role_named(Role::Button, "Save");
    let facts = app.inspect().highlight(&save).expect("Save resolves");
    println!("highlight {} → {}", save.render(), facts_summary(&facts));
    let plan = app
        .session()
        .adapter()
        .overlay()
        .expect("the adapter composites the plan");
    println!(
        "plan: hover={}, badges={}",
        plan.hover.is_some(),
        plan.badges.len()
    );

    let email = Selector::label("Email");
    let facts = app.inspect().highlight(&email).expect("Email resolves");
    println!("highlight {} → {}", email.render(), facts_summary(&facts));

    // Clear the outline (badges keep tracking), then end the session.
    app.inspect().clear_highlight().expect("outline cleared");
    app.inspect().disable().expect("overlay off");
    assert!(!app.inspect().is_active());

    let trace = app.finish();
    println!(
        "trace: {}",
        trace.map_or_else(|| "none".into(), |p| p.display().to_string())
    );
}

/// One inspector line: role, accessible name, physical-px bounds (§9
/// AccessKit convention — exactly what the overlay outline draws).
fn facts_summary(facts: &guiwright_inspect::codegen::NodeFacts) -> String {
    let bounds = facts.bounds.map_or_else(
        || "no bounds".into(),
        |[x0, y0, x1, y1]| format!("[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]"),
    );
    format!(
        "{} {:?} {bounds}",
        facts.role.as_deref().unwrap_or("?"),
        facts.name
    )
}
