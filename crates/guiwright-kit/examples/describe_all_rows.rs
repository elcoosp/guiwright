//! The set-facts read at the kit surface (M39): `locator.describe_all()`
//! answers "what IS everything in the set" — one walk, one row per
//! match in document order, each carrying the identity facts (role,
//! accessible name, value, bounds) beside the generation the handle is
//! live at. Snapshot semantics: no strict mode, no waiting, an empty set
//! answers with an empty list, a read records nothing.
//!
//! ```text
//! cargo run -p guiwright-kit --example describe_all_rows
//! ```
//!
//! Walks the fixture's whole button set at row width, reads the Email
//! input's facts (the value rides beside the name — the fact the value
//! locator vocabulary matches), and walks an empty set (one empty list —
//! the read is total, never an error).

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let mut app = App::launch(
        "describe_all_rows_demo",
        MockAdapter::with_fixture(&Fixture::buttons()),
    );
    app.wait_for_settle();

    // The whole set at row width (M39): the four buttons, document
    // order — the facts the `describe` row prints, at set width.
    let rows = app
        .role_locator(accesskit::Role::Button)
        .describe_all()
        .expect("the set-facts read resolves");
    println!("{} button row(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {:?}{} (generation {})",
            row.role,
            row.name.as_deref().unwrap_or_default(),
            row.value
                .as_deref()
                .map_or_else(String::new, |v| format!(" value {v:?}")),
            row.generation
        );
    }

    // One row, the facts the value locator matches: the Email input's
    // name AND value ride the same row — the two facts the `texts` and
    // `values` reads pick between, here side by side.
    let email = app
        .get_by_label("Email")
        .describe_all()
        .expect("the single-row read resolves");
    for row in &email {
        println!(
            "email row — {} name {:?} value {:?}",
            row.role,
            row.name.as_deref().unwrap_or_default(),
            row.value.as_deref().unwrap_or_default()
        );
    }

    // The empty set: one empty list — the read is total, never an error.
    let none = app
        .get_by_test_id("ghost")
        .describe_all()
        .expect("the empty read resolves");
    println!("ghost set — {} row(s)", none.len());
}
