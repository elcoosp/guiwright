//! The kit identity-state surface (§8.1's state facts beside the
//! identity row, M68/M70 — the m68 record's named kit gap, closed): the
//! state three — focused, enabled, the checked tri-state — read through
//! [`guiwright_kit::Locator::describe_state`] (one handle) and
//! [`guiwright_kit::Locator::describe_all_state`] (the set walk), the
//! embedded half of the wire's `DescribeIdentityState` /
//! `DescribeAllIdentityState` domains. The facts are total reads, never
//! waits (the m68 rule): the vocabulary words answer right now — the
//! waitable face stays the kit's `to_be_checked` / `to_be_focused` /
//! `to_be_enabled` assertions, unchanged. Both reads compose with every
//! narrowing (`.first()`, `.nth()`, `.has_text(...)`) through the same
//! strict-mode surface, and with the window pin (the §16 v0.4 rule): a
//! pinned locator reads only inside its subtree.

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;
use guiwright_proto::schema::CheckedStateV1;

fn fixture_app(test_name: &str) -> App<MockAdapter> {
    let mut app = App::launch(
        test_name,
        MockAdapter::with_fixture(&Fixture::with_checkboxes()),
    );
    app.wait_for_settle();
    app
}

fn multi_window_app(test_name: &str) -> (App<MockAdapter>, String, String) {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(test_name, MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();
    (app, w1, w2)
}

#[test]
fn describe_state_reads_the_resolved_node_state() {
    let mut app = fixture_app("kit_describe_state_checked");
    let row = app
        .get_by_role(Role::CheckBox, "Notifications")
        .describe_state()
        .expect("the read");
    assert_eq!(row.row.role, "CheckBox");
    assert_eq!(row.row.name.as_deref(), Some("Notifications"));
    assert_eq!(
        row.checked,
        CheckedStateV1::Checked,
        "the tri-state vocabulary word, never an Option<bool>"
    );
    assert!(row.enabled, "a live checkbox is enabled");
    assert!(
        !row.focused,
        "an unfocused node answers false, never an error"
    );
}

#[test]
fn describe_state_answers_the_full_tri_state() {
    let mut app = fixture_app("kit_describe_state_trio");
    let dark = app
        .get_by_role(Role::CheckBox, "Dark mode")
        .describe_state()
        .expect("the read");
    assert_eq!(dark.checked, CheckedStateV1::Unchecked);
    let mixed = app
        .get_by_role(Role::CheckBox, "Partially installed")
        .describe_state()
        .expect("the read");
    assert_eq!(mixed.checked, CheckedStateV1::Mixed);
    // A button exposes no checked state: the fact answers, never an
    // error (the M68 `NotCheckable` rule at kit width).
    let button = app
        .get_by_role(Role::Button, "Save")
        .describe_state()
        .expect("the read");
    assert_eq!(
        button.checked,
        CheckedStateV1::NotCheckable,
        "the fact, not an error"
    );
}

#[test]
fn describe_state_carries_the_identity_row() {
    let mut app = fixture_app("kit_describe_state_identity");
    let row = app
        .get_by_role(Role::CheckBox, "Notifications")
        .describe_state()
        .expect("the read");
    assert_eq!(
        row.row.bounds,
        Some([16.0, 204.0, 136.0, 236.0]),
        "the identity facts ride beside the state three (the M68 row shape)"
    );
    assert_eq!(row.row.generation, 0, "the handle was live here");
}

#[test]
fn describe_state_composes_with_narrowing_and_strict_mode() {
    let mut app = fixture_app("kit_describe_state_narrowing");
    // The checkbox trio: strict mode fails the bare role locator (three
    // matches), `.first()` narrows to the first (document order).
    let ambiguous = app.role_locator(Role::CheckBox).describe_state();
    assert!(
        ambiguous.is_err(),
        "the state read resolves through strict mode like every locator read"
    );
    let first = app
        .role_locator(Role::CheckBox)
        .first()
        .describe_state()
        .expect("the narrowed read");
    assert_eq!(
        first.row.name.as_deref(),
        Some("Notifications"),
        "document order"
    );
}

#[test]
fn a_missing_match_fails_the_state_read() {
    let mut app = fixture_app("kit_describe_state_missing");
    let err = app
        .get_by_role(Role::CheckBox, "Ghost")
        .describe_state()
        .expect_err("nothing matches");
    assert!(err.to_string().contains("no node matches"), "{err}");
}

#[test]
fn the_window_pin_scopes_the_state_describe() {
    let (mut app, _w1, w2) = multi_window_app("kit_describe_state_window");
    {
        let mut preview = app.window(&w2).expect("Preview is a live target");
        let row = preview
            .get_by_role(Role::Button, "Save")
            .describe_state()
            .expect("the pinned read");
        let guiwright_core::NodeKey::AccessKit { tree, .. } = row.row.key else {
            panic!("accesskit fixture keys");
        };
        assert_eq!(
            tree.0.to_string(),
            w2,
            "the described node lives in the Preview subtree"
        );
    }
}

#[test]
fn describe_all_state_carries_the_walk_rows_in_document_order() {
    let mut app = fixture_app("kit_describe_all_state_set");
    let rows = app
        .role_locator(Role::CheckBox)
        .describe_all_state()
        .expect("the walk");
    assert_eq!(
        rows.iter().map(|r| r.checked.clone()).collect::<Vec<_>>(),
        vec![
            CheckedStateV1::Checked,
            CheckedStateV1::Unchecked,
            CheckedStateV1::Mixed,
        ],
        "the tri-state per row, document order, one pass"
    );
    assert!(rows.iter().all(|r| r.enabled), "a live checkbox is enabled");
    assert_eq!(
        rows.first().and_then(|r| r.row.name.as_deref()),
        Some("Notifications"),
        "the identity row rides beside the state three"
    );
}

#[test]
fn describe_all_state_answers_an_empty_set_with_an_empty_list() {
    let mut app = fixture_app("kit_describe_all_state_empty");
    let rows = app
        .get_by_role(Role::CheckBox, "Ghost")
        .describe_all_state()
        .expect("the walk");
    assert!(
        rows.is_empty(),
        "the m30 snapshot rule at kit width: no rows, no error"
    );
}

#[test]
fn describe_all_state_composes_with_narrowing_and_the_window_pin() {
    let (mut app, _w1, w2) = multi_window_app("kit_describe_all_state_window");
    {
        let mut preview = app.window(&w2).expect("Preview is a live target");
        let rows = preview
            .role_locator(Role::Button)
            .first()
            .describe_all_state()
            .expect("the pinned walk");
        let guiwright_core::NodeKey::AccessKit { tree, .. } = rows[0].row.key else {
            panic!("accesskit fixture keys");
        };
        assert_eq!(tree.0.to_string(), w2, "the walk stayed in the subtree");
        assert_eq!(rows.len(), 1, "the narrowed walk");
    }
}
