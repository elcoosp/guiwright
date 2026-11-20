//! The set-state read at the kit surface (M70): `locator.describe_all_state()`
//! answers "what is everything in the set **doing**" — one walk, one row
//! per match in document order, each carrying the state three (focused,
//! enabled, the checked tri-state) beside the identity facts. One pass
//! instead of one `describe_state()` round trip per row; the wire's
//! `DescribeAllIdentityState` domain (M70) serves the same rows to
//! remote clients. Snapshot semantics: no strict mode, no waiting, an
//! empty set answers with an empty list, a read records nothing.
//!
//! ```text
//! cargo run -p guiwright-kit --example describe_all_state_rows
//! ```
//!
//! Walks the fixture's checkbox trio at row width (the tri-state per
//! row in document order), narrows a window-pinned walk, and walks an
//! empty set (one empty list — the read is total, never an error).

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let mut app = App::launch(
        "describe_all_state_rows_demo",
        MockAdapter::with_fixture(&Fixture::with_checkboxes()),
    );
    app.wait_for_settle();

    // The whole set at state width (M70): the trio's tri-state per row,
    // document order — one pass, the m70 set-walk shape.
    let rows = app
        .role_locator(accesskit::Role::CheckBox)
        .describe_all_state()
        .expect("the set-state read resolves");
    println!("{} checkbox row(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {:?} — checked: {:?}, enabled: {}, focused: {}",
            row.row.role,
            row.row.name.as_deref().unwrap_or_default(),
            row.checked,
            row.enabled,
            row.focused
        );
    }

    // The narrowed walk: `.first()` rides the same snapshot semantics.
    let first = app
        .role_locator(accesskit::Role::CheckBox)
        .first()
        .describe_all_state()
        .expect("the narrowed read resolves");
    println!(
        "first checkbox — {:?} checked: {:?}",
        first[0].row.name.as_deref().unwrap_or_default(),
        first[0].checked
    );

    // The empty set: one empty list — the read is total, never an error.
    let none = app
        .get_by_role(accesskit::Role::CheckBox, "Ghost")
        .describe_all_state()
        .expect("the empty read resolves");
    println!("ghost set — {} row(s)", none.len());
}
