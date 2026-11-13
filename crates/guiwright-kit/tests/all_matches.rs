//! The `all()` enumeration surface (§16 assertions row, the m22/m25
//! records' open kit gap): `locator.all(visit)` enumerates the locator's
//! current match set in document order, handing each match to the visitor
//! as an nth-pinned locator — Playwright's `Locator.all()` adapted to the
//! kit's borrow-and-act shape (a `Vec<Locator>` is impossible while every
//! locator holds the `&mut App`, so the matches arrive one at a time).
//!
//! The tests pin the enumeration end-to-end on the mock adapter: document
//! order, index pinning (the visited locator IS the nth item), the empty
//! set, the window pin scoping the set, data collection through the
//! generic return, and the read-not-action recording rule.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

#[test]
fn all_enumerates_the_whole_match_set_in_document_order() {
    let mut app = App::launch_fixture("all_document_order");
    // The fixture ships 4 buttons; strict mode would refuse a role
    // locator, but enumeration talks about the set (the `count()` seam).
    let keys = app
        .role_locator(Role::Button)
        .all(|item| Ok(item.resolve()?.key))
        .expect("enumeration resolves");
    assert_eq!(keys.len(), 4, "every button visited once");
    // Document order per the fixture: Save, Cancel, Invisible, Confirm.
    let fixture = Fixture::buttons();
    let expected: Vec<guiwright_core::NodeKey> = [
        fixture.save_button,
        fixture.cancel_button,
        fixture.hidden_button,
        fixture.disabled_button,
    ]
    .into_iter()
    .map(|node| guiwright_core::NodeKey::AccessKit {
        tree: accesskit::TreeId::ROOT,
        node,
    })
    .collect();
    assert_eq!(keys, expected, "document order, no re-sorting");
}

#[test]
fn all_items_are_nth_pinned_locators() {
    let mut app = App::launch_fixture("all_nth_pinned");
    // Each visited locator must be the match at its index: re-resolving
    // the visited item lands on exactly one node, and two visits to the
    // same set visit the same identities in the same order.
    let first_pass = app
        .role_locator(Role::Button)
        .all(|item| Ok(item.resolve()?.key))
        .expect("first pass");
    let second_pass = app
        .role_locator(Role::Button)
        .all(|item| Ok(item.resolve()?.key))
        .expect("second pass");
    assert_eq!(first_pass, second_pass, "stable across passes");

    // The pin is real: the visited locator's own selector carries the
    // nth filter (`.all()` is sugar over `.nth(i)`, Playwright parity).
    let pinned = app
        .role_locator(Role::Button)
        .all(|item| Ok(item.selector().render()))
        .expect("pinned selectors");
    assert_eq!(
        pinned,
        vec![
            r"getByRole(Button).nth(0)",
            r"getByRole(Button).nth(1)",
            r"getByRole(Button).nth(2)",
            r"getByRole(Button).nth(3)",
        ],
        "each visited selector is index-pinned"
    );
}

#[test]
fn all_is_empty_for_a_locator_that_matches_nothing() {
    let mut app = App::launch_fixture("all_empty");
    let visited = app
        .get_by_role(Role::Button, "Ghost")
        .all(|_| Ok(()))
        .expect("zero matches is a valid enumeration");
    assert!(visited.is_empty(), "no visits for an empty match set");
}

#[test]
fn all_completes_when_nodes_late_arrive() {
    // The enumeration is a snapshot read like `count()`: it enumerates
    // what the tree holds *now* (Playwright's `all()` does not wait), so
    // a locator whose node arrives later simply shows up on the next
    // enumeration.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture::buttons();
    let late = fixture.add_button_update("Export");
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut app = App::launch("all_late_node", adapter);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(late);
    });
    expect(app.role_locator(Role::Button))
        .to_have_count(5)
        .expect("the export button lands the fifth");
    let names = app
        .role_locator(Role::Button)
        .all(|item| {
            let key = item.resolve()?.key;
            let guiwright_core::NodeKey::AccessKit { node, .. } = key else {
                panic!("fixture keys are accesskit keys");
            };
            Ok(node.0)
        })
        .expect("enumeration after the late node");
    assert_eq!(names.len(), 5, "the late button joins the set");
}

#[test]
fn all_keeps_the_window_pin() {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("all_window_pin", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();

    // Scoped: the Preview window carries exactly one Save.
    let w2_keys = app
        .window(&w2)
        .expect("Preview is a live target")
        .get_by_role(Role::Button, "Save")
        .all(|item| Ok(item.resolve()?.key))
        .expect("scoped enumeration");
    assert_eq!(w2_keys, vec![fx.w2_save_key()], "the pin scopes the set");

    // Unscoped: the whole tree carries both windows' saves.
    let both = app
        .get_by_role(Role::Button, "Save")
        .all(|item| Ok(item.resolve()?.key))
        .expect("whole-tree enumeration");
    assert_eq!(both.len(), 2, "both windows' saves");
    assert!(both.contains(&fx.w1_save_key()));
    assert!(both.contains(&fx.w2_save_key()));
}

#[test]
fn all_collects_per_item_data_through_the_return_value() {
    let mut app = App::launch_fixture("all_collect");
    // The generic return: the visitor maps each match to whatever the
    // flow needs — here the rendered selector of every visited item.
    let rendered = app
        .get_by_role(Role::Button, "Cancel")
        .all(|item| Ok(item.selector().render()))
        .expect("one Cancel");
    assert_eq!(
        rendered,
        vec![r#"getByRole(Button, "Cancel" (fuzzy)).nth(0)"#],
        "the visited item renders its own pinned selector"
    );
}

#[test]
fn all_visited_actions_record_like_any_locator_action() {
    let mut app = App::launch_fixture("all_codegen");
    app.enable_codegen();
    // Enumeration itself is a read (the `count()` rule): no line of its
    // own. Each visited locator's actions record normally — the visited
    // item acts (and records) exactly like a hand-written `.nth(i)`
    // locator, and the codegen cost model picks the minimal stable
    // selector for the node the visit resolved (§13, the standing rule).
    let mut clicked = 0usize;
    app.role_locator(Role::Button)
        .all(|item| {
            if clicked == 0 {
                item.click()?;
            }
            clicked += 1;
            Ok(())
        })
        .expect("enumeration with one click");
    let (code, warnings) = app.codegen_finish().expect("recorder active");
    assert!(warnings.is_empty(), "no warnings: {warnings:?}");
    assert_eq!(
        code.lines().count(),
        1,
        "one replayable line for the one clicked visit:\n{code}"
    );
    assert!(
        code.contains(r#"get_by_role(Role::Button, "Save")"#),
        "the clicked visit records the minimal stable selector:\n{code}"
    );
}
