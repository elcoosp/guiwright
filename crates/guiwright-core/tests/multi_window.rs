//! Multi-window integration tests (architecture §16 v0.4 "Multi-window /
//! tabs — targets per window", M6).
//!
//! AccessKit models multiple windows as **grafted subtrees**: window nodes
//! in the root tree carry a `tree_id` property naming the subtree that holds
//! the window's content. The consumer grafts them into one walkable tree, so
//! every node keeps a per-window identity (`NodeKey::AccessKit { tree, node }`
//! — the TreeId IS the window identity). These tests pin the tree-level half
//! of the feature: window enumeration and window-scoped resolution.

use accesskit::{NodeId, Point, Rect, Role, TreeId};
use guiwright_core::CoreError;
use guiwright_core::selector::Selector;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_core::tree::TreeService;

fn buttons_tree() -> TreeService {
    let fixture = Fixture::buttons();
    let mut svc = TreeService::new();
    svc.apply_update(fixture.initial_update());
    svc
}

fn multi_window_tree() -> (TreeService, MultiWindowFixture) {
    let fixture = MultiWindowFixture::new();
    let mut svc = TreeService::new();
    for update in fixture.initial_updates() {
        svc.apply_update(update);
    }
    (svc, fixture)
}

fn node_id(key: guiwright_core::NodeKey) -> NodeId {
    let guiwright_core::NodeKey::AccessKit { node, .. } = key else {
        panic!("fixture keys are accesskit keys");
    };
    node
}

fn tree_id_of(key: &guiwright_core::NodeKey) -> TreeId {
    let guiwright_core::NodeKey::AccessKit { tree, .. } = *key else {
        panic!("fixture keys are accesskit keys");
    };
    tree
}

// ---------------------------------------------------------------------------
// Window enumeration (TreeService::windows)
// ---------------------------------------------------------------------------

#[test]
fn single_window_app_reports_one_main_window() {
    let svc = buttons_tree();
    let windows = svc.windows();
    assert_eq!(windows.len(), 1, "a graft-free tree is one window");
    assert_eq!(windows[0].handle, "main");
    assert_eq!(windows[0].title.as_deref(), Some("Test App"));
    // The main scope is rooted at the tree root.
    assert_eq!(node_id(windows[0].root), NodeId(0));
}

#[test]
fn grafted_subtrees_are_enumerated_as_windows() {
    let (svc, fx) = multi_window_tree();
    let windows = svc.windows();
    assert_eq!(windows.len(), 2, "one target per grafted window subtree");
    assert_eq!(
        windows[0].handle,
        fx.w1.0.to_string(),
        "handle = subtree TreeId"
    );
    assert_eq!(windows[0].title.as_deref(), Some("Editor"));
    assert_eq!(windows[1].handle, fx.w2.0.to_string());
    assert_eq!(windows[1].title.as_deref(), Some("Preview"));
    // Window scope roots are the graft nodes themselves (root-tree nodes).
    assert_eq!(tree_id_of(&windows[0].root), TreeId::ROOT);
    assert_eq!(node_id(windows[0].root), NodeId(fx.w1_graft));
}

#[test]
fn ungrafted_tree_before_init_lists_no_windows() {
    let svc = TreeService::new();
    assert!(svc.windows().is_empty(), "no tree, no windows");
}

// ---------------------------------------------------------------------------
// Window-scoped resolution (targets per window, §16 v0.4)
// ---------------------------------------------------------------------------

#[test]
fn same_named_nodes_are_strict_without_a_window_scope() {
    let (svc, _) = multi_window_tree();
    // "Save" exists in BOTH windows — the unscoped (default) surface keeps
    // its strict-mode semantics and reports the ambiguity.
    let err = svc
        .resolve(&Selector::role_named(Role::Button, "Save"))
        .expect_err("two Save buttons across windows");
    match err {
        CoreError::Ambiguous { matches, .. } => assert_eq!(matches.len(), 2),
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn scope_narrows_resolution_to_one_window() {
    let (svc, fx) = multi_window_tree();
    let w1 = svc.window_scope(&fx.w1.0.to_string()).expect("w1 exists");
    let w2 = svc.window_scope(&fx.w2.0.to_string()).expect("w2 exists");

    let in_w1 = svc
        .resolve_scoped(&Selector::role_named(Role::Button, "Save"), w1.as_ref())
        .expect("scoped to the editor window");
    assert_eq!(in_w1.key, fx.w1_save_key());

    let in_w2 = svc
        .resolve_scoped(&Selector::role_named(Role::Button, "Save"), w2.as_ref())
        .expect("scoped to the preview window");
    assert_eq!(in_w2.key, fx.w2_save_key());

    // The two windows reuse the same local NodeIds — only the TreeId (the
    // window identity) tells them apart.
    assert_eq!(node_id(in_w1.key), node_id(in_w2.key));
    assert_ne!(tree_id_of(&in_w1.key), tree_id_of(&in_w2.key));
}

#[test]
fn scope_hides_other_windows_from_every_selector_form() {
    let (svc, fx) = multi_window_tree();
    let w1 = svc.window_scope(&fx.w1.0.to_string()).expect("w1 exists");

    // Text that only exists in W2 must not resolve under a W1 scope.
    let err = svc
        .resolve_scoped(&Selector::text("Preview text"), w1.as_ref())
        .expect_err("W2-only label hidden from W1 scope");
    assert!(matches!(err, CoreError::NoMatch { .. }), "{err}");

    // And the W2-only label DOES resolve unscoped — scoping narrows, it
    // never deletes.
    svc.resolve(&Selector::text("Preview text"))
        .expect("visible without a scope");
}

#[test]
fn scoped_resolution_survives_incremental_updates() {
    let (mut svc, fx) = multi_window_tree();
    svc.apply_update(fx.w1_add_button_update("Export"));

    let w1 = svc.window_scope(&fx.w1.0.to_string()).expect("w1 exists");
    let w2 = svc.window_scope(&fx.w2.0.to_string()).expect("w2 exists");

    let export = Selector::role_named(Role::Button, "Export");
    let in_w1 = svc
        .resolve_scoped(&export, w1.as_ref())
        .expect("export landed in W1");
    assert_eq!(tree_id_of(&in_w1.key), fx.w1);

    let err = svc
        .resolve_scoped(&export, w2.as_ref())
        .expect_err("export is not in W2");
    assert!(matches!(err, CoreError::NoMatch { .. }), "{err}");
}

#[test]
fn unknown_window_scope_is_a_clean_error() {
    let (svc, _) = multi_window_tree();
    let err = svc
        .window_scope("00000000-0000-0000-0000-00000000dead")
        .expect_err("no such window");
    assert!(matches!(err, CoreError::UnknownWindow { .. }), "{err}");
}

#[test]
fn main_scope_is_noop_on_single_window_apps() {
    let svc = buttons_tree();
    let scope = svc.window_scope("main").expect("main exists");
    assert_eq!(scope, None, "the main scope is the whole tree: no filter");
    // Every resolvable selector still resolves.
    svc.resolve(&Selector::role_named(Role::Button, "Save"))
        .expect("unaffected");
}

#[test]
fn main_scope_is_unknown_on_multi_window_apps() {
    let (svc, _) = multi_window_tree();
    let err = svc.window_scope("main").expect_err("main is not a target");
    assert!(matches!(err, CoreError::UnknownWindow { .. }), "{err}");
}

// ---------------------------------------------------------------------------
// Per-window identity bookkeeping (generations stay per-tree, §8.2)
// ---------------------------------------------------------------------------

#[test]
fn closing_a_window_bumps_only_its_own_generations() {
    let (mut svc, fx) = multi_window_tree();
    let before = svc.resolve(&Selector::role_named(Role::Button, "Save"));
    assert!(before.is_err(), "ambiguous before a scope exists — sanity");
    let w1_save = fx.w1_save_key();

    // Close the Preview window: the app removes the graft node; the
    // consumer tears the subtree down.
    svc.apply_update(fx.close_w2_update());

    let windows = svc.windows();
    assert_eq!(windows.len(), 1, "one window left");
    assert_eq!(windows[0].handle, fx.w1.0.to_string());

    // The W1 button keeps its generation (untouched identity).
    assert_eq!(svc.generation(&w1_save), 0);
    // The W2 subtree is gone: resolving it scoped errors as unknown window.
    let err = svc
        .window_scope(&fx.w2.0.to_string())
        .expect_err("w2 closed");
    assert!(matches!(err, CoreError::UnknownWindow { .. }), "{err}");
    // Unscoped strict resolution now finds exactly ONE Save (W1's).
    let only = svc
        .resolve(&Selector::role_named(Role::Button, "Save"))
        .expect("unambiguous after W2 closed");
    assert_eq!(only.key, w1_save);
}

// ---------------------------------------------------------------------------
// Fixture sanity: the grafted updates are self-consistent
// ---------------------------------------------------------------------------

#[test]
fn fixture_updates_are_ordered_root_first() {
    let fx = MultiWindowFixture::new();
    let updates = fx.initial_updates();
    assert_eq!(updates.len(), 3, "root + two subtrees");
    assert_eq!(updates[0].tree_id, TreeId::ROOT);
    assert_eq!(updates[1].tree_id, fx.w1);
    assert_eq!(updates[2].tree_id, fx.w2);
    for u in &updates {
        assert!(u.tree.is_some(), "initial updates are full updates");
    }
}

/// Guard against accidental fixture drift: node reuse across windows is the
/// POINT of the fixture (keys must not collide because TreeIds differ).
#[test]
fn fixture_windows_reuse_local_node_ids() {
    let fx = MultiWindowFixture::new();
    assert_eq!(node_id(fx.w1_save_key()), node_id(fx.w2_save_key()));
    assert_ne!(fx.w1_save_key(), fx.w2_save_key());
}

/// Bounds remain window-relative (AccessKit convention, §9): the same
/// window-relative rect may appear in two windows.
#[test]
fn scoped_resolution_reports_window_relative_bounds() {
    let (svc, fx) = multi_window_tree();
    let w2 = svc.window_scope(&fx.w2.0.to_string()).expect("w2 exists");
    let resolved = svc
        .resolve_scoped(&Selector::role_named(Role::Button, "Save"), w2.as_ref())
        .expect("save in w2");
    let bounds = resolved.bounds.expect("fixture buttons carry bounds");
    assert_eq!(
        bounds,
        Rect {
            x0: 12.0,
            y0: 416.0,
            x1: 132.0,
            y1: 448.0,
        }
    );
}

#[test]
fn hit_test_finds_nodes_in_both_windows() {
    // The unscoped hit-test walks every window (input routing is a
    // per-window concern of the adapter, §9).
    let (svc, fx) = multi_window_tree();
    let w1_hit = svc
        .hit_test(Point::new(20.0, 24.0))
        .expect("inside w1 save");
    assert_eq!(tree_id_of(&w1_hit), fx.w1);
    let w2_hit = svc
        .hit_test(Point::new(20.0, 24.0 + 400.0))
        .expect("inside w2 save");
    assert_eq!(tree_id_of(&w2_hit), fx.w2);
}
