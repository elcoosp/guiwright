//! The kit multi-window handle surface (§16 v0.4, the kit half of M6):
//! `app.windows()` enumerates the adapter's window targets, `app.window(handle)`
//! aims a borrow-and-act handle at one target, and every locator born from
//! that handle resolves **inside the window's subtree only** — the same
//! scoping the wire's `Window::Use` gives a remote connection, on the
//! embedded surface. The fixture's two windows both carry a "Save" button
//! (independent subtree namespaces), so unscoped resolution is ambiguous by
//! construction and the scope is the discriminator.

use accesskit::Role;
use guiwright_core::CoreError;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, WaitState, expect};

/// Launch the two-window fixture and return the app plus both handles as
/// strings (the `TreeId` uuids, the same vocabulary `window list` prints).
fn multi_window_app(test_name: &str) -> (App<MockAdapter>, MultiWindowFixture, String, String) {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(test_name, MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();
    (app, fx, w1, w2)
}

#[test]
fn windows_reports_the_adapter_targets() {
    let (mut app, _fx, w1, w2) = multi_window_app("kit_windows_list");

    let windows = app.windows();
    assert_eq!(windows.len(), 2, "the fixture grafts two windows");
    let by_handle: Vec<(&str, &str, bool)> = windows
        .iter()
        .map(|w| (w.handle.as_str(), w.title.as_str(), w.focused))
        .collect();
    assert!(
        by_handle.contains(&(w1.as_str(), "Editor", true)),
        "Editor focused: {by_handle:?}"
    );
    assert!(
        by_handle.contains(&(w2.as_str(), "Preview", false)),
        "Preview unfocused: {by_handle:?}"
    );
}

#[test]
fn window_handle_carries_the_target_metadata() {
    let (mut app, _fx, w1, w2) = multi_window_app("kit_window_metadata");

    let (editor_title, editor_focused, editor_handle) = {
        let editor = app.window(&w1).expect("Editor is a live target");
        (
            editor.title().to_string(),
            editor.is_focused(),
            editor.handle().to_string(),
        )
    };
    assert_eq!(editor_handle, w1);
    assert_eq!(editor_title, "Editor");
    assert!(editor_focused);

    let (preview_title, preview_focused) = {
        let preview = app.window(&w2).expect("Preview is a live target");
        (preview.title().to_string(), preview.is_focused())
    };
    assert_eq!(preview_title, "Preview");
    assert!(!preview_focused);
}

#[test]
fn unknown_window_fails_fast_before_any_op() {
    let (mut app, _fx, _w1, _w2) = multi_window_app("kit_window_unknown");

    let err = app
        .window("a6000000-dead-beef-0000-000000000000")
        .expect_err("a dead handle must fail at aim time");
    let message = err.to_string();
    assert!(
        message.contains("a6000000-dead-beef-0000-000000000000"),
        "the error names the handle: {message}"
    );
}

#[test]
fn scoped_locator_resolves_inside_its_window() {
    let (mut app, fx, w1, w2) = multi_window_app("kit_window_scoped_resolve");

    // Unscoped, the two "Save" buttons (independent subtree namespaces,
    // same local NodeId) are ambiguous — strict mode fails.
    let unscoped = app.get_by_role(Role::Button, "Save").resolve();
    assert!(
        matches!(
            unscoped,
            Err(guiwright_kit::KitError::Core(CoreError::Ambiguous { .. }))
        ),
        "two Save buttons without a scope: {unscoped:?}"
    );

    // The scope is the discriminator: each handle resolves its own subtree's
    // button, and the keys are the AccessKit (tree, node) identities.
    let in_w1 = app
        .window(&w1)
        .expect("w1")
        .get_by_role(Role::Button, "Save")
        .resolve()
        .expect("Save inside Editor");
    assert_eq!(in_w1.key, fx.w1_save_key(), "w1's Save");

    let in_w2 = app
        .window(&w2)
        .expect("w2")
        .get_by_role(Role::Button, "Save")
        .resolve()
        .expect("Save inside Preview");
    assert_eq!(in_w2.key, fx.w2_save_key(), "w2's Save");
    assert_ne!(in_w1.key, in_w2.key);
}

#[test]
fn scoped_locator_click_stays_inside_its_window() {
    let (mut app, _fx, _w1, w2) = multi_window_app("kit_window_scoped_click");

    // The actionability gate + click run inside the Preview scope — the
    // resolved node is w2's Save, so the click succeeds without ambiguity.
    app.window(&w2)
        .expect("w2")
        .get_by_role(Role::Button, "Save")
        .click()
        .expect("scoped click lands on Preview's Save");
}

#[test]
fn scoped_expect_scopes_its_assertion() {
    let (mut app, _fx, w1, w2) = multi_window_app("kit_window_scoped_expect");

    // "Preview text" lives only in the Preview window: visible in scope,
    // absent from Editor's scope (timed out, not polluted by the other
    // window's tree).
    expect(app.window(&w2).expect("w2").get_by_text("Preview text"))
        .to_be_visible()
        .expect("Preview text is visible inside Preview");

    let err = expect(app.window(&w1).expect("w1").get_by_text("Preview text"))
        .with_timeout(std::time::Duration::from_millis(250))
        .to_be_visible()
        .expect_err("Editor's scope has no Preview text");
    assert!(
        matches!(err, CoreError::Timeout { .. }),
        "scoped absence is a timeout, not a cross-window hit: {err:?}"
    );
}

#[test]
fn scoped_wait_for_scopes_its_wait() {
    let (mut app, _fx, w1, w2) = multi_window_app("kit_window_scoped_wait_for");

    app.window(&w2)
        .expect("w2")
        .get_by_text("Preview text")
        .wait_for_with_timeout(WaitState::Visible, std::time::Duration::from_millis(250))
        .expect("Preview text visible in Preview");

    let err = app
        .window(&w1)
        .expect("w1")
        .get_by_text("Preview text")
        .wait_for_with_timeout(WaitState::Visible, std::time::Duration::from_millis(250))
        .expect_err("Editor's scope never sees Preview text");
    assert!(matches!(err, CoreError::Timeout { .. }), "{err:?}");
}

#[test]
fn window_mouse_is_pre_pinned() {
    let (mut app, _fx, _w1, w2) = multi_window_app("kit_window_mouse_pinned");

    app.window(&w2)
        .expect("w2")
        .mouse()
        .click(50.0, 50.0)
        .expect("click through the pre-pinned handle");

    let logged = app.session().adapter().window_events();
    assert!(!logged.is_empty(), "the click reached the adapter");
    assert!(
        logged.iter().all(|(w, _)| *w == w2),
        "every injected event is tagged with the aimed window: {logged:?}"
    );
}

#[test]
fn codegen_records_the_window_shape() {
    let (mut app, _fx, _w1, w2) = multi_window_app("kit_window_codegen");

    app.enable_codegen();
    app.window(&w2)
        .expect("w2")
        .get_by_role(Role::Button, "Save")
        .click()
        .expect("scoped click");
    let (code, _warnings) = app
        .codegen_code()
        .expect("codegen was enabled and an action was recorded");
    assert_eq!(
        code,
        format!("app.window({w2:?}).get_by_role(Role::Button, \"Save\").click();\n"),
        "the recorded line reads like the new surface: {code}"
    );
}

#[test]
fn closed_window_target_stops_resolving() {
    let (mut app, fx, w1, w2) = multi_window_app("kit_window_closed");

    // The app closes the Preview window: the graft node leaves the root
    // tree, so the handle is dead even if adapter metadata lingers —
    // the tree is the source of truth (§16 v0.4).
    app.session()
        .adapter_mut()
        .push_update(fx.close_w2_update());
    app.wait_for_settle();

    assert!(
        app.window(&w2).is_err(),
        "the closed window's handle is no longer a live target"
    );
    // Editor keeps working through its handle…
    app.window(&w1)
        .expect("w1 still live")
        .get_by_role(Role::Button, "Save")
        .resolve()
        .expect("Editor's Save still resolves");
    // …and unscoped resolution is unambiguous again (one window left).
    app.get_by_role(Role::Button, "Save")
        .resolve()
        .expect("single remaining Save");
}

#[test]
fn main_window_target_works_on_single_window_apps() {
    let mut app = App::launch_fixture("kit_window_main");
    app.wait_for_settle();

    let mut main = app.window("main").expect("single-window apps report main");
    assert_eq!(main.handle(), "main");
    main.get_by_role(Role::Button, "Save")
        .resolve()
        .expect("scoped-by-main resolution is the whole tree");
}
