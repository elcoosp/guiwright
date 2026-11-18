//! The value-first bulk read (§16 assertions row — the m29 record's
//! named follow-up: "`all_values()` (a value-first bulk read for form
//! dumps) is a one-line variation of the same walk — deliberately not
//! added until a suite asks for it"): `locator.all_values()` reads the
//! value of every match in document order as one snapshot — the
//! [`all_text_contents`][all] rule with the source order flipped for
//! form semantics.
//!
//! [all]: ../all_text_contents/index.html
//!
//! The tests pin the read end-to-end on the mock adapter: value-first
//! over the fixture's Email input (where `all_text_contents` reads the
//! label), the label fallback, the empty set, the narrowing and window
//! pin scoping, and the positional-stability rule (a node with neither
//! value nor label reads as an empty string).

use accesskit::Role;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

#[test]
fn the_input_set_reads_values_not_labels() {
    // The form-dump semantics: an input's value is what the form
    // carries — the read takes it first, where `all_text_contents`
    // takes the label (the getByValue source order, flipped).
    let mut app = App::launch_fixture("all_values_input");
    let values = app
        .get_by_role(Role::TextInput, "Email")
        .all_values()
        .expect("the value read");
    assert_eq!(
        values,
        vec!["ada@example.dev".to_string()],
        "value-first: the carried value, not the visible label"
    );
}

#[test]
fn the_read_falls_back_to_the_label_for_valueless_nodes() {
    // Buttons carry labels, no values: the fallback keeps every node
    // answering (the totality rule — positions stay aligned with the
    // set read's document order).
    let mut app = App::launch_fixture("all_values_fallback");
    let values = app
        .role_locator(Role::Button)
        .all_values()
        .expect("the fallback read");
    assert_eq!(
        values,
        vec![
            "Save".to_string(),
            "Cancel".to_string(),
            "Invisible".to_string(),
            "Confirm".to_string(),
        ],
        "label fallback, document order, hidden included"
    );
}

#[test]
fn an_empty_set_enumerates_to_no_values() {
    let mut app = App::launch_fixture("all_values_empty");
    let values = app
        .get_by_role(Role::Button, "Ghost")
        .all_values()
        .expect("the empty read");
    assert!(values.is_empty(), "{values:?}");
}

#[test]
fn narrowing_scopes_the_read() {
    let mut app = App::launch_fixture("all_values_narrowed");
    let values = app
        .get_by_role(Role::TextInput, "Email")
        .first()
        .all_values()
        .expect("the narrowed read");
    assert_eq!(values, vec!["ada@example.dev".to_string()]);
}

#[test]
fn the_window_pin_scopes_the_read() {
    // Two windows, both with a "Save" button: the pin scopes the set to
    // one subtree (the window-handle discriminator).
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("all_values_window", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    let values = app
        .window(&w2)
        .expect("w2 aims")
        .role_locator(Role::Button)
        .all_values()
        .expect("the scoped read");
    assert_eq!(values, vec!["Save".to_string()], "w2's Save only");
}

#[test]
fn a_node_with_neither_value_nor_label_reads_as_empty() {
    // The positional-stability rule (the m29 text read's rule, kept): a
    // node that carries neither datum reads as an empty string, so the
    // Vec positions stay 1:1 with the document order the set read
    // established. The fixture's unlabeled group is the neither-node.
    let mut app = App::launch_fixture("all_values_empty_string");
    let values = app
        .role_locator(Role::Group)
        .all_values()
        .expect("the neither-node read");
    assert_eq!(values, vec![String::new()], "empty, not skipped");
}
