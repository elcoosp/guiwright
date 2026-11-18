//! The kit locator filter surface (M34 — the §8.1 selector vocabulary
//! completed end-to-end): `locator.has_not(inner)` and
//! `locator.locator(child)` — the Playwright `filter({ hasNot })` and
//! chained `locator(selector)` shapes, adapted to the kit's
//! borrow-and-act doctrine. Both narrowings are pure composition over the
//! core resolver's `Filter { has_not }` / `All(chain)` machinery (which
//! the proto + wire already carry end-to-end): the kit locator is the
//! only consumer surface that could not reach them — `first`/`last`/
//! `nth`/`has_text` were exposed, the inverted filter and the descendant
//! chain were not, so a suite that needed either had to hand-build a
//! `Selector` value and leave the locator DSL.
//!
//! Semantics pinned here (the core resolver's contract, exercised through
//! the kit):
//!
//! * `has_not(inner)` keeps only matches whose subtree (the node
//!   included) matches no node of `inner` — the inverted `has_text`;
//! * `locator(child)` resolves the locator's own set first, then narrows
//!   to `child` nodes inside each match's subtree, document order kept;
//! * both compose with the existing narrowing (`first`/`nth`/`has_text`
//!   wrap the composed selector in another `Filter` — the resolver nests
//!   them), the window pin scopes the composed set, and every action
//!   through a chained locator rides the same strict-mode resolve +
//!   actionability gate as any locator action.
//!
//! The fixture puts Save, Cancel, the Welcome label, the Email input and
//! the hidden/disabled buttons under one group: the group is the subtree
//! discriminator (its subtree matches `Button "Confirm"`, every leaf's
//! subtree is itself).

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
fn has_not_drops_matches_whose_subtree_fits_the_inner_selector() {
    let mut app = fixture_app("kit_filter_has_not");

    // The group's subtree holds Confirm: the inverted filter drops it.
    assert_eq!(app.role_locator(Role::Group).count().unwrap(), 1);
    assert_eq!(
        app.role_locator(Role::Group)
            .has_not(&Selector::role_named(Role::Button, "Confirm"))
            .count()
            .unwrap(),
        0,
        "the group's subtree matches Button \"Confirm\" — dropped"
    );
    assert_eq!(
        app.role_locator(Role::Group)
            .has_not(&Selector::role_named(Role::Button, "Export"))
            .count()
            .unwrap(),
        1,
        "no Export anywhere in the subtree — kept"
    );
    // The inner selector is a full selector: a role+name constraint, not
    // a text probe.
    assert_eq!(
        app.role_locator(Role::Group)
            .has_not(&Selector::role(Role::TextInput))
            .count()
            .unwrap(),
        0,
        "the Email input lives in the group's subtree too"
    );
}

#[test]
fn has_not_matches_the_subtree_including_the_node_itself() {
    let mut app = fixture_app("kit_filter_has_not_self");

    // The leaf pin: a node's own subtree includes itself, so a button
    // filtered by `has_not(button)` excludes itself — the rule that makes
    // `has_not` the exact inverse of `has_text` (both read the same
    // subtree walk).
    assert_eq!(
        app.get_by_role(Role::Button, "Save")
            .has_not(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        0,
        "Save is itself a Button — its own subtree matches"
    );
    // The label's subtree has no button: kept.
    assert_eq!(
        app.get_by_text("Welcome, Ada")
            .has_not(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        1,
    );
}

#[test]
fn chained_locator_narrows_to_the_child_selector() {
    let mut app = fixture_app("kit_filter_chain");

    // group >> buttons: the four buttons under the group (the set read is
    // non-strict — hidden and disabled included, the count() semantics).
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role(Role::Button))
            .count()
            .unwrap(),
        4,
        "Save, Cancel, Invisible, Confirm — the group's whole button set"
    );
    // The chain narrows further: a named child, and a child that never
    // exists (the empty set is a valid read).
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role_named(Role::Button, "Save"))
            .count()
            .unwrap(),
        1,
    );
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role_named(Role::Button, "Ghost"))
            .count()
            .unwrap(),
        0,
        "an empty chain answer, not an error — the snapshot-read rule"
    );
}

#[test]
fn the_chain_resolves_the_first_button_in_document_order() {
    let mut app = fixture_app("kit_filter_chain_first");

    // The composed narrowing rides the same strict resolve: `first` over
    // the chain picks the document-order head (Save — node 2).
    let expected = app
        .key_of(&Selector::role_named(Role::Button, "Save"))
        .expect("Save resolves unscoped on the single-window fixture");
    let resolved = app
        .role_locator(Role::Group)
        .locator(&Selector::role(Role::Button))
        .first()
        .resolve()
        .expect("the chain + first resolves");
    assert_eq!(
        resolved.key, expected,
        "document order kept through the chain"
    );
}

#[test]
fn filters_compose_over_the_chain() {
    let mut app = fixture_app("kit_filter_compose");

    // has_not over a chain: the buttons that do not carry a Confirm in
    // their own subtree — every button except Confirm itself.
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role(Role::Button))
            .has_not(&Selector::role_named(Role::Button, "Confirm"))
            .count()
            .unwrap(),
        3,
        "Save, Cancel, Invisible — Confirm's subtree matches itself"
    );
    // has_text over a chain, the pre-existing narrowing composing with
    // the new one.
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role(Role::Button))
            .has_text("Cancel")
            .count()
            .unwrap(),
        1,
    );
}

#[test]
fn a_chained_locator_act_runs_the_full_strict_gate() {
    let mut app = fixture_app("kit_filter_chain_action");

    // The action path is unchanged: a chained locator clicks through the
    // same auto-waiting, actionability-gated resolve as any locator.
    let outcome = app
        .role_locator(Role::Group)
        .locator(&Selector::role_named(Role::Button, "Cancel"))
        .click()
        .expect("the chained locator clicks");
    assert_eq!(outcome.detail, "clicked", "{outcome:?}");
    assert_eq!(
        app.session().adapter().actions().len(),
        1,
        "the semantic action reached the adapter once"
    );

    // Strict mode still applies to the composed selector: two matches in
    // the chain answer fail the action (the ambiguity surface, §8.2).
    let err = app
        .role_locator(Role::Group)
        .locator(&Selector::role(Role::Button))
        .click()
        .expect_err("four buttons match the chain — strict mode refuses");
    assert!(err.to_string().contains("resolved to 4 nodes"), "{err}");
    assert_eq!(
        app.session().adapter().actions().len(),
        1,
        "a failed resolve synthesizes nothing"
    );
}

#[test]
fn the_window_pin_scopes_the_composed_set() {
    let (mut app, w1, w2) = multi_window_app("kit_filter_window");

    // Unscoped, both windows' content groups hold a Save: the chain
    // answers 2 (the set read is non-strict).
    assert_eq!(
        app.role_locator(Role::Group)
            .locator(&Selector::role_named(Role::Button, "Save"))
            .count()
            .unwrap(),
        2,
    );

    // Pinned, the same chain answers 1 — the pin scopes the walk.
    let fx = MultiWindowFixture::default();
    let mut editor = app.window(&w1).expect("Editor is a live target");
    let resolved = editor
        .role_locator(Role::Group)
        .locator(&Selector::role_named(Role::Button, "Save"))
        .resolve()
        .expect("the chain resolves inside the Editor subtree");
    assert_eq!(resolved.key, fx.w1_save_key(), "the scope discriminates");

    // And the Preview scope answers with the other window's Save.
    let mut preview = app.window(&w2).expect("Preview is a live target");
    let resolved = preview
        .role_locator(Role::Group)
        .locator(&Selector::role_named(Role::Button, "Save"))
        .resolve()
        .expect("the chain resolves inside the Preview subtree");
    assert_eq!(resolved.key, fx.w2_save_key(), "the scope discriminates");
}
