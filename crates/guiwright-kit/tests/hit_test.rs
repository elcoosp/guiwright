//! The kit hit-test surface (M32): the §8.3 "receives events" mechanism —
//! the core `TreeService::hit_test` read — exposed as `App::hit_test`
//! (whole tree) and `WindowHandle::hit_test` (the aimed window's subtree,
//! the §16 v0.4 scoping rule). The point is physical px in the tree's
//! bounds space — the same space the resolve rects and dump rows report,
//! so a test can hit inside a rect it just resolved. A miss answers
//! `None` (the read is total, never an error); a hit returns the same
//! `Resolved` record the resolution pipeline returns (identity +
//! generation + bounds), ready for a follow-up action.
//!
//! The fixture puts the Save button at 16,16 → 136,48; the two-window
//! fixture puts a Save button in each window (Editor at 16,16, Preview at
//! 12,416), so the window pin is the discriminator.

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn fixture_app(test_name: &str) -> App<MockAdapter> {
    let mut app = App::launch(test_name, MockAdapter::with_fixture(&Fixture::buttons()));
    app.wait_for_settle();
    app
}

fn multi_window_app(test_name: &str) -> (App<MockAdapter>, MultiWindowFixture, String, String) {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(test_name, MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();
    (app, fx, w1, w2)
}

#[test]
fn hit_test_answers_the_node_under_the_point() {
    let mut app = fixture_app("kit_hit_test_hit");
    let resolved = app
        .hit_test(76.0, 32.0)
        .expect("the read is total")
        .expect("the save button sits under the point");
    let key = app
        .key_of(&guiwright_core::Selector::role_named(Role::Button, "Save"))
        .expect("save resolves");
    assert_eq!(resolved.key, key, "the hit is the Save button's identity");
    assert_eq!(resolved.generation, 0);
    assert_eq!(
        resolved.bounds,
        Some(accesskit::Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        }),
        "the bounds ride the read (the same rect `resolve` returns)"
    );
}

#[test]
fn a_miss_reads_none() {
    let mut app = fixture_app("kit_hit_test_miss");
    assert!(
        app.hit_test(400.0, 220.0).expect("total").is_none(),
        "empty space answers None, never an error"
    );
}

#[test]
fn the_window_handle_scopes_the_hit() {
    let (mut app, _fx, w1, w2) = multi_window_app("kit_hit_test_window");
    {
        let mut editor = app.window(&w1).expect("Editor is a live target");
        let hit = editor
            .hit_test(76.0, 32.0)
            .expect("total")
            .expect("Editor's Save sits under the point");
        assert_eq!(hit.generation, 0);
    }
    {
        let mut preview = app.window(&w2).expect("Preview is a live target");
        assert!(
            preview.hit_test(76.0, 32.0).expect("total").is_none(),
            "Editor's point reads None through Preview's handle (the pin rides the read)"
        );
        let hit = preview
            .hit_test(72.0, 432.0)
            .expect("total")
            .expect("Preview's own Save answers under the pin");
        let guiwright_core::NodeKey::AccessKit { tree, node } = hit.key else {
            panic!("accesskit fixture keys");
        };
        assert_eq!(
            tree.0.to_string(),
            w2,
            "the hit lives in the Preview subtree"
        );
        assert_eq!(node, accesskit::NodeId(2), "the subtree's Save button");
    }
}

#[test]
fn a_dead_window_handle_fails_at_aim_time() {
    let (mut app, _fx, _w1, _w2) = multi_window_app("kit_hit_test_dead_window");
    // The aim-time validation is the M6 rule, unchanged for the read: a
    // handle the tree does not know fails with UnknownWindow at aim time,
    // not with a cross-window hit.
    let err = app
        .window("00000000-0000-0000-0000-00000000dead")
        .expect_err("a dead handle is not a usable target");
    assert!(err.to_string().contains("no window"), "{err}");
}
