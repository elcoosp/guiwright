//! The kit locator `has(...)` sugar (the m34 record's remaining-gaps
//! list: *"No `filter({ has })` positive-locator filter on the kit
//! surface yet (`has_text` covers the string form; the selector-valued
//! positive filter is `All`-chain shaped and reachable via `locator()`
//! for the chain case — a distinct `has(...)` sugar lands when a suite
//! asks"*): the Playwright `filter({ has })` shape — keep only the
//! matches whose **subtree** contains a node matching `inner`.
//!
//! Semantics pinned here (the positive dual of the m34 `has_not` rule,
//! read through the same subtree walk):
//!
//! * `has(inner)` keeps only matches whose subtree (the node included)
//!   matches at least one node of `inner` — the inverted `has_not`,
//!   which reads the same walk with the negated verdict;
//! * the leaf rule carries over: a leaf's subtree is itself, so
//!   `get_by_role(Button).has(&role(Button))` keeps every button;
//! * it composes with the existing narrowing (`has_not`/`has_text`/
//!   `first`/`nth`/`locator` nest by value), the window pin scopes the
//!   composed set, and every action through a `has`-filtered locator
//!   rides the same strict-mode resolve + actionability gate as any
//!   locator action.
//!
//! The fixture puts Save, Cancel, the Welcome label, the Email input and
//! the hidden/disabled buttons under one group: the group is the subtree
//! discriminator (its subtree matches `Button`, every leaf's subtree is
//! itself).

use accesskit::Role;
use guiwright_core::Selector;
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
    (app, fx.w1.0.to_string(), fx.w2.0.to_string())
}

#[test]
fn has_keeps_matches_whose_subtree_fits_the_inner_selector() {
    let mut app = fixture_app("kit_filter_has");

    // The group's subtree holds buttons: the positive filter keeps it.
    assert_eq!(app.role_locator(Role::Group).count().unwrap(), 1);
    assert_eq!(
        app.role_locator(Role::Group)
            .has(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        1,
        "the group's subtree matches Button — kept"
    );
    assert_eq!(
        app.role_locator(Role::Group)
            .has(&Selector::role_named(Role::Button, "Export"))
            .count()
            .unwrap(),
        0,
        "no Export anywhere in the subtree — dropped"
    );
    // The inner selector is a full selector: a role+name constraint, not
    // a text probe.
    assert_eq!(
        app.role_locator(Role::Group)
            .has(&Selector::role(Role::TextInput))
            .count()
            .unwrap(),
        1,
        "the Email input lives in the group's subtree too"
    );
}

#[test]
fn has_matches_the_subtree_including_the_node_itself() {
    let mut app = fixture_app("kit_filter_has_self");

    // The leaf pin, the has_not rule's positive dual: a node's own
    // subtree includes itself, so a button filtered by `has(button)`
    // keeps itself — `has` and `has_not` read the same subtree walk
    // with opposite verdicts.
    assert_eq!(
        app.get_by_role(Role::Button, "Save")
            .has(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        1,
        "Save is itself a Button — its own subtree matches"
    );
    // The label's subtree has no button: dropped.
    assert_eq!(
        app.get_by_text("Welcome, Ada")
            .has(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        0,
    );
}

#[test]
fn has_composes_over_the_chain_and_the_other_filters() {
    let mut app = fixture_app("kit_filter_has_compose");

    // has over a chain: the buttons that carry a button in their own
    // subtree — every button (the leaf rule at set width).
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role(Role::Button))
            .has(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        4,
        "Save, Cancel, Invisible, Confirm — each subtree matches itself"
    );
    // Composing with the inverted filter: the group keeps (its subtree
    // holds a button) and drops (its subtree holds Confirm) — 0.
    assert_eq!(
        app.role_locator(Role::Group)
            .has(&Selector::role(Role::Button))
            .has_not(&Selector::role_named(Role::Button, "Confirm"))
            .count()
            .unwrap(),
        0,
        "has and has_not compose — the group fits both verdicts"
    );
    // ... and flips back when the inverted probe names a ghost.
    assert_eq!(
        app.role_locator(Role::Group)
            .has(&Selector::role(Role::Button))
            .has_not(&Selector::role_named(Role::Button, "Ghost"))
            .count()
            .unwrap(),
        1,
    );
    // `first` over the composed set keeps the document-order head — and
    // the head of a `has` set is the ANCESTOR (the group), the exact
    // distinction from the `locator()` chain, whose head is the
    // descendant (Save).
    let expected = app
        .key_of(&Selector::role(Role::Group))
        .expect("the group resolves unscoped on the single-window fixture");
    let resolved = app
        .role_locator(Role::Group)
        .has(&Selector::role_named(Role::Button, "Save"))
        .first()
        .resolve()
        .expect("has + first resolves");
    assert_eq!(
        resolved.key, expected,
        "document order kept through has — the ancestor row is the answer"
    );
}

#[test]
fn a_has_filtered_locator_act_runs_the_full_strict_gate() {
    let mut app = fixture_app("kit_filter_has_action");

    // The action path is unchanged: a has-filtered locator clicks
    // through the same auto-waiting, actionability-gated resolve as any
    // locator. The leaf rule aims the click: Save kept by
    // `has(button)` — its own subtree matches.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .has(&Selector::role_named(Role::Button, "Save"))
        .click()
        .expect("the has-filtered locator clicks");
    assert_eq!(outcome.detail, "clicked", "{outcome:?}");
    assert_eq!(
        app.session().adapter().actions().len(),
        1,
        "the semantic action reached the adapter once"
    );

    // Strict mode still applies to the composed selector: the chain +
    // has set answers four buttons, and the action refuses (the
    // ambiguity surface, §8.2).
    let err = app
        .role_locator(Role::Group)
        .locator(&Selector::role(Role::Button))
        .has(&Selector::role(Role::Button))
        .click()
        .expect_err("four buttons match the has-filtered chain — strict mode refuses");
    assert!(err.to_string().contains("resolved to 4 nodes"), "{err}");
    assert_eq!(
        app.session().adapter().actions().len(),
        1,
        "a failed resolve synthesizes nothing"
    );
}

#[test]
fn the_has_set_refuses_ambiguity_and_scopes_by_window() {
    let fx = MultiWindowFixture::default();
    let (mut app, w1, w2) = multi_window_app("kit_filter_has_window");

    // Unscoped, both windows' content groups hold a button: the
    // has-filtered set answers 2 (the set read is non-strict).
    assert_eq!(
        app.role_locator(Role::Group)
            .has(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        2,
    );

    // Strict mode still applies to the composed selector: the unscoped
    // two-match set fails the action (the ambiguity surface, §8.2).
    let err = app
        .role_locator(Role::Group)
        .has(&Selector::role(Role::Button))
        .click()
        .expect_err("two groups match the has filter — strict mode refuses");
    assert!(err.to_string().contains("resolved to 2 nodes"), "{err}");
    assert_eq!(
        app.session().adapter().actions().len(),
        0,
        "a failed resolve synthesizes nothing"
    );

    // Pinned, the same has-filtered set answers the window's own group —
    // the pin scopes the walk exactly as it scopes every other
    // narrowing. Each window's content group is its subtree root, so the
    // two scopes resolve into different subtrees.
    let mut editor = app.window(&w1).expect("Editor is a live target");
    let resolved = editor
        .role_locator(Role::Group)
        .has(&Selector::role(Role::Button))
        .resolve()
        .expect("the has filter resolves inside the Editor subtree");
    let guiwright_core::NodeKey::AccessKit { tree: w1_tree, .. } = resolved.key else {
        panic!("multi-window fixture keys are accesskit keys");
    };
    assert_eq!(w1_tree, fx.w1, "the scope discriminates");

    let mut preview = app.window(&w2).expect("Preview is a live target");
    let resolved = preview
        .role_locator(Role::Group)
        .has(&Selector::role(Role::Button))
        .resolve()
        .expect("the has filter resolves inside the Preview subtree");
    let guiwright_core::NodeKey::AccessKit { tree: w2_tree, .. } = resolved.key else {
        panic!("multi-window fixture keys are accesskit keys");
    };
    assert_eq!(w2_tree, fx.w2, "the scope discriminates");
}
