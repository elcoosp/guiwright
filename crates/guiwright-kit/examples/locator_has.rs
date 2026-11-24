//! The locator `has(...)` filter (§16 locators row — the m34 record's
//! named follow-on): `locator.has(&inner)` — the Playwright
//! `filter({ has })` shape on the kit's borrow-and-act doctrine. Keep
//! only the matches whose subtree — the node included — contains a node
//! matching `inner`: the positive dual of `has_not`, which reads the
//! same subtree walk with the opposite verdict, and the exact inverse of
//! the `locator()` chain, which narrows to the descendant's rows where
//! `has` keeps the ancestor's.
//!
//! ```text
//! cargo run -p guiwright-kit --example locator_has
//! ```
//!
//! Filters the button fixture's group by the buttons its subtree holds,
//! pins the leaf rule (`Save.has(button)` keeps itself), composes with
//! `has_not`, and rides the strict gate through the filter.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("locator_has_demo", adapter);
    app.wait_for_settle();

    let button = Selector::role(Role::Button);
    let save = Selector::role_named(Role::Button, "Save");

    // The ancestor rows: the group's subtree holds buttons — kept.
    let kept = app
        .role_locator(Role::Group)
        .has(&button)
        .count()
        .expect("the group keeps");
    println!("group.has(button) → {kept} match(es) — the ancestor's row is the answer");

    // A name constraint the subtree cannot answer drops the match: the
    // empty set is the valid snapshot read, never an error.
    let dropped = app
        .role_locator(Role::Group)
        .has(&Selector::role_named(Role::Button, "Export"))
        .count()
        .expect("the ghost drops");
    println!("group.has(button \"Export\") → {dropped} match(es) — the empty set read");

    // The leaf rule: a leaf's subtree is itself — Save kept by
    // `has(button)`, the same walk `has_not` reads with the opposite
    // verdict.
    let leaf = app
        .get_by_role(Role::Button, "Save")
        .has(&button)
        .count()
        .expect("Save keeps itself");
    println!("Save.has(button) → {leaf} match(es) — the leaf's subtree is itself");

    // Composing: has and has_not ride the same locator — the group
    // keeps (its subtree holds a button) and drops (its subtree holds
    // Save) in one composed read.
    let composed = app
        .role_locator(Role::Group)
        .has(&button)
        .has_not(&save)
        .count()
        .expect("the composed read");
    println!("group.has(button).has_not(Save) → {composed} match(es) — the filters compose");

    // The composed selector renders like every other narrowing.
    let rendered = Selector::role(Role::Group)
        .has(button.clone())
        .has_not(save.clone());
    println!("selector render → {}", rendered.render());

    // Actions ride the same strict-mode resolve + actionability gate
    // through the filter.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .has(&save)
        .click()
        .expect("the has-filtered locator clicks");
    println!("Save.has(Save).click() → {}", outcome.detail);
}
