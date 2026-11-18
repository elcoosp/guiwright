//! The kit locator drag surface (M33 — the m19 parity stream's named
//! drag item): `source.drag_to(target_selector)` — the Playwright
//! `locator.dragTo(target)` shape, adapted to the kit's borrow-and-act
//! doctrine. The target is a [`Selector`], not a locator value: every
//! locator holds the `&mut App` it acts through (the `all()` rule), so
//! two locator values could never coexist — and the selector form is the
//! honest contract for a second reason: a drag is **one gesture in one
//! window's coordinate space**, so both ends resolve in the source
//! locator's own window context (the source's pin, §16 v0.4; unscoped
//! locators aim the whole tree). A cross-window drag has no coordinate
//! answer here by construction — the aim resolves inside the source's
//! subtree or it fails with the strict-mode `NoMatch` that names the
//! miss.
//!
//! Both ends auto-wait with the full strict-mode + actionability-free
//! resolve discipline; the drag runs from the source's bounds center to
//! the target's through the session's gesture synthesis (the adapter's
//! pacing table ratchets the pointer path, §9); the applied gesture
//! records exactly like the raw-input surface (§13 over §9: the raw
//! `mouse.drag` kit line while a codegen recording runs).
//!
//! The fixture puts Save at 16,16 → 136,48 (center 76,32) and Cancel at
//! 16,56 → 136,88 (center 76,72); the two-window fixture puts a "Save"
//! button in each window and a "Preview text" label only in Preview, so
//! the window scoping is the discriminator.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::{InputEvent, PointerButton};
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
fn drag_to_walks_the_pointer_from_source_to_target() {
    let mut app = fixture_app("kit_drag_to_path");

    let outcome = app
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Cancel"))
        .expect("the drag runs");

    assert_eq!(
        outcome.detail, "dragged (76, 32) → (76, 72) — 10 events",
        "source center → target center, the detail names both ends"
    );
    // The pointer path: press at the source, travel (the §9 ratchet cuts
    // the 40 px trip into 5 px steps), release at the target. The mock
    // records the window-aware hook's log (§9 per-window routing) — the
    // handle rides every entry, the order is the synthesis order.
    let events: Vec<&InputEvent> = app
        .session()
        .adapter()
        .window_events()
        .iter()
        .map(|(_, ev)| ev)
        .collect();
    assert_eq!(
        events.first(),
        Some(&InputEvent::PointerDown {
            x: 76.0,
            y: 32.0,
            button: PointerButton::Primary,
        })
        .as_ref(),
        "press at the source's center"
    );
    assert_eq!(
        events.last(),
        Some(&InputEvent::PointerUp {
            x: 76.0,
            y: 72.0,
            button: PointerButton::Primary,
        })
        .as_ref(),
        "release at the target's center"
    );
    assert_eq!(events.len(), 10, "down + 8 ratchet moves + up");
}

#[test]
fn drag_to_auto_waits_a_late_target() {
    let fixture = Fixture::buttons();
    let scripts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("kit_drag_to_late", adapter);
    app.wait_for_settle();

    // The app adds the Export button 100 ms in — the target resolve
    // auto-waits on the tree-diff event, never by polling (§15.3).
    let add = fixture.add_button_update("Export");
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        scripts.lock().expect("script channel").push(add);
    });
    app.get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Export"))
        .expect("the target arrives and the drag runs");
}

#[test]
fn a_window_pinned_drag_resolves_both_ends_inside_the_subtree() {
    let (mut app, w1, _w2) = multi_window_app("kit_drag_to_window");

    // Editor-born drag: the aim resolves inside the Editor subtree, where
    // "Save" is unambiguous — the drag runs.
    let mut editor = app.window(&w1).expect("Editor is a live target");
    let outcome = editor
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Save"))
        .expect("the aim resolves inside the Editor subtree");
    assert!(outcome.detail.contains("dragged"), "{outcome:?}");

    // The same drag aimed at content that only exists in Preview: the
    // pin scopes the aim too, so the aim never arrives — the auto-wait
    // times out naming the aim (every locator action's behavior), never
    // a cross-window hit (one gesture, one coordinate space, by
    // construction).
    let mut editor = app.window(&w1).expect("Editor is a live target");
    let err = editor
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::text("Preview text"))
        .expect_err("Preview text never arrives inside the Editor subtree");
    let msg = err.to_string();
    assert!(msg.contains("timed out"), "{msg}");
    assert!(msg.contains("Preview text"), "{msg}");
}

#[test]
fn an_unscoped_drag_keeps_strict_mode_on_both_ends() {
    let (mut app, _w1, _w2) = multi_window_app("kit_drag_to_strict");

    // Unscoped, two windows carry a "Save" each: the source end itself is
    // ambiguous — strict mode fails before any gesture runs.
    let err = app
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Save"))
        .expect_err("two Save buttons, one per window");
    assert!(err.to_string().contains("resolved to 2 nodes"), "{err}");
    assert!(
        app.session().adapter().events().is_empty(),
        "no gesture ran"
    );
}

#[test]
fn a_node_without_bounds_cannot_be_dragged() {
    let mut app = fixture_app("kit_drag_to_no_bounds");

    // The fixture's group carries no bounds (only leaf widgets do) — an
    // honest error, not an empty drag (the §10 element-crop rule's shape).
    let err = app
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role(Role::Group))
        .expect_err("the group has no bounds");
    assert!(err.to_string().contains("no bounds"), "{err}");
}

#[test]
fn the_applied_drag_records_kit_code() {
    let mut app = fixture_app("kit_drag_to_codegen");
    app.enable_codegen();

    app.get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Cancel"))
        .expect("the drag runs");

    let (code, warnings) = app.codegen_code().expect("the recorder is on");
    assert!(
        code.contains("drag(76.0, 32.0, 76.0, 72.0)"),
        "the raw §13-over-§9 kit line: {code}"
    );
    assert!(warnings.is_empty(), "no strict-mode warnings: {warnings:?}");
}

#[test]
fn an_aim_that_no_longer_resolves_fails_the_drag() {
    let mut app = fixture_app("kit_drag_to_missing");

    let err = app
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Ghost"))
        .expect_err("the ghost never exists");
    // The aim auto-waits like every locator action: a node that never
    // arrives times out naming the aim — no silent miss, no cross-tree
    // fallback.
    let msg = err.to_string();
    assert!(msg.contains("timed out"), "{msg}");
    assert!(msg.contains("Ghost"), "{msg}");
    // A failed aim synthesizes no input: the adapter saw no events.
    assert_eq!(
        app.session().adapter().window_events(),
        [],
        "a failed aim synthesizes nothing"
    );
}
