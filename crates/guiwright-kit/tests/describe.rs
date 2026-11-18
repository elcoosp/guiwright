//! The kit node-describe surface (M36): the §8.1 identity facts — role,
//! accessible name, value, bounds — read through
//! [`guiwright_kit::Locator::describe`], the embedded half of the wire's
//! `Describe` domain (the m32 record's standing gap). The debugging read
//! beside the bulk readers: `all_text_contents()` (M29) answers "what
//! does the set say", `describe()` answers "what IS the node I just
//! resolved". The read composes with every narrowing (`.first()`,
//! `.nth()`, `.has_text(...)`) because it resolves through the same
//! strict-mode surface every locator action aims through, and with the
//! window pin (the §16 v0.4 rule): a pinned locator describes only
//! inside its subtree.

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn fixture_app(test_name: &str) -> App<MockAdapter> {
    let mut app = App::launch(test_name, MockAdapter::with_fixture(&Fixture::buttons()));
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
fn describe_reads_the_resolved_node() {
    let mut app = fixture_app("kit_describe_save");
    let row = app
        .get_by_role(Role::Button, "Save")
        .describe()
        .expect("the read");
    assert_eq!(row.role, "Button");
    assert_eq!(row.name.as_deref(), Some("Save"));
    assert_eq!(row.value, None, "a button carries no value");
    assert_eq!(
        row.bounds,
        Some([16.0, 16.0, 136.0, 48.0]),
        "the bounds ride the read (the resolve rect, restated)"
    );
    assert_eq!(row.generation, 0, "the handle was live here");
}

#[test]
fn describe_reads_the_value_an_input_carries() {
    let mut app = fixture_app("kit_describe_email");
    let row = app
        .get_by_role(Role::TextInput, "Email")
        .describe()
        .expect("the read");
    assert_eq!(row.role, "TextInput");
    assert_eq!(row.name.as_deref(), Some("Email"));
    assert_eq!(
        row.value.as_deref(),
        Some("ada@example.dev"),
        "the value the getByValue locators match (§8.1)"
    );
}

#[test]
fn describe_composes_with_narrowing_and_strict_mode() {
    let mut app = fixture_app("kit_describe_narrowing");
    // The buttons fixture carries Save + Cancel: strict mode fails the
    // bare role locator (two matches), `.first()` narrows to the first.
    let ambiguous = app.role_locator(Role::Button).describe();
    assert!(
        ambiguous.is_err(),
        "the describe resolves through strict mode like every locator read"
    );
    let first = app
        .role_locator(Role::Button)
        .first()
        .describe()
        .expect("the narrowed read");
    assert_eq!(first.name.as_deref(), Some("Save"), "document order");
}

#[test]
fn the_window_pin_scopes_the_describe() {
    let (mut app, _w1, w2) = multi_window_app("kit_describe_window");
    {
        let mut preview = app.window(&w2).expect("Preview is a live target");
        let row = preview
            .get_by_role(Role::Button, "Save")
            .describe()
            .expect("the pinned read");
        let guiwright_core::NodeKey::AccessKit { tree, .. } = row.key else {
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
fn a_missing_match_fails_the_read() {
    let mut app = fixture_app("kit_describe_missing");
    let err = app
        .get_by_text("No Such Node")
        .describe()
        .expect_err("a locator that resolves nothing cannot describe");
    assert!(err.to_string().contains("no node matches"), "{err}");
}
