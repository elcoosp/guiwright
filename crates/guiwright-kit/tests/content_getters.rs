//! The content getters (§16 actions row — the m19 record's "Remaining
//! gaps (honest list)" item: *"`select_option`, `scroll_into_view_if_needed`,
//! `drag_to`, `wait_for(state)` and the content getters (`inner_text`,
//! `get_attribute`, `input_value`) stay §16 follow-ons"* — `select_option`
//! paid first (m89), `scroll_into_view_if_needed` second (m90), `drag_to`
//! (m33) and `wait_for(state)` (m21) before those; the getters are the
//! list's last never-cashed members). The kit surface gains the two
//! single-node reads the a11y substrate carries facts for:
//!
//! * `locator.inner_text()` — the text the text locators match (the
//!   accessible label first, then the value — the `HasText` check's
//!   source order, the [`all_text_contents`] head-of-set derivation);
//! * `locator.input_value()` — the value the form carries (the value
//!   fact only, never the label fallback — `all_values`' flip is the
//!   bulk read's shape, not the single-node getter's).
//!
//! [`all_text_contents`]: ../all_text_contents/index.html
//!
//! Both ride the [`describe`] surface: strict mode, the auto-wait
//! resolve, the window pin — and the read rule (nothing recorded, no
//! codegen line, no trace action). `get_attribute` stays the honest
//! cap: the a11y substrate carries no arbitrary attribute vocabulary
//! (the m19 deferral's "vocabulary the wire does not carry" applies —
//! the facts DOM attributes feed are the identity reads' name/value/
//! description, which `describe()` already answers).
//!
//! [`describe`]: ../describe/index.html

use accesskit::Role;
use guiwright_kit::App;

#[test]
fn inner_text_reads_the_text_the_text_locators_match() {
    // The HasText source order: the accessible label first, then the
    // value — the same derivation `all_text_contents` applies per set
    // row, now for the one match the strict resolve answers. The Email
    // input carries both: the label wins (where `all_values` reads the
    // value, this reads "Email" — the text locators' match).
    let mut app = App::launch_fixture("content_getters_inner_text");
    let text = app
        .get_by_role(Role::TextInput, "Email")
        .inner_text()
        .expect("the label-first read");
    assert_eq!(
        text, "Email",
        "label first: the text get_by_text matches, not the carried value"
    );
}

#[test]
fn inner_text_reads_the_rendered_identity_of_labels_and_buttons() {
    // The label and the button carry only their accessible names: the
    // read answers exactly the text a `get_by_text`/`get_by_role`
    // locator matches on them.
    let mut app = App::launch_fixture("content_getters_inner_text_identity");
    let label = app
        .get_by_role(Role::Label, "Welcome, Ada")
        .inner_text()
        .expect("the label read");
    assert_eq!(label, "Welcome, Ada", "the label's own text");
    let button = app
        .get_by_role(Role::Button, "Save")
        .inner_text()
        .expect("the button read");
    assert_eq!(button, "Save", "the button's accessible name");
}

#[test]
fn inner_text_is_a_total_read_on_factless_nodes() {
    // The unnamed group carries neither label nor value: the read
    // answers the empty string (the m29 totality rule — a node with
    // neither fact reads empty, never an error).
    let mut app = App::launch_fixture("content_getters_inner_text_total");
    let text = app
        .role_locator(Role::Group)
        .inner_text()
        .expect("the total read");
    assert_eq!(text, "", "neither fact → the empty string");
}

#[test]
fn input_value_reads_what_the_form_carries() {
    // Playwright's `inputValue()` shape: the value fact only. Where
    // `inner_text` reads "Email" on the same node, this reads the
    // carried value — the two getters can never silently agree on the
    // wrong datum (the m31 flip rule, single-node).
    let mut app = App::launch_fixture("content_getters_input_value");
    let value = app
        .get_by_role(Role::TextInput, "Email")
        .input_value()
        .expect("the value read");
    assert_eq!(
        value, "ada@example.dev",
        "the carried value, not the visible label"
    );
}

#[test]
fn input_value_refuses_a_node_without_a_value_fact() {
    // Playwright's inputValue throws for non-form elements; the honest
    // a11y read answers the typed refusal naming the locator and the
    // missing fact — never a silent label fallback, never an empty
    // string (a button's name is not a value).
    let mut app = App::launch_fixture("content_getters_input_value_refusal");
    let err = app
        .get_by_role(Role::Button, "Save")
        .input_value()
        .expect_err("a button carries no value fact");
    assert!(
        err.to_string().contains("getByRole"),
        "the refusal names the locator: {err}"
    );
    assert!(
        err.to_string().contains("value"),
        "the refusal names the missing fact: {err}"
    );
}

#[test]
fn the_getters_resolve_through_strict_mode() {
    // The describe() surface: the bare role locator is ambiguous on the
    // buttons fixture (four matches) — strict mode fails the read, the
    // same rule every single-node locator read carries.
    let mut app = App::launch_fixture("content_getters_strict");
    let ambiguous = app.role_locator(Role::Button).inner_text();
    assert!(
        ambiguous.is_err(),
        "strict mode gates the single-node reads like every locator read"
    );
    let value_ambiguous = app.role_locator(Role::Button).input_value();
    assert!(
        value_ambiguous.is_err(),
        "input_value rides the same strict resolve"
    );
    let first = app
        .role_locator(Role::Button)
        .first()
        .inner_text()
        .expect("the narrowed read");
    assert_eq!(first, "Save", "document order after the narrowing");
}

#[test]
fn the_reads_record_nothing_but_the_trace_event() {
    // The `count()` rule: a read is not a replayable step — nothing
    // lands in the codegen output (actions record; reads don't).
    let mut app = App::launch_fixture("content_getters_codegen");
    app.enable_codegen();
    let _ = app
        .get_by_role(Role::TextInput, "Email")
        .inner_text()
        .expect("the read");
    let _ = app
        .get_by_role(Role::TextInput, "Email")
        .input_value()
        .expect("the second read");
    let code = app.codegen_code().expect("recording is enabled");
    assert_eq!(code.0.trim().lines().count(), 0, "no lines: {code:?}");
}
