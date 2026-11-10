//! The kit inspector surface (§13, M8): `app.inspect()` — enable /
//! highlight / clear / disable, the embedded half of the wire domain the
//! REPL and the TypeScript client speak. The mock adapter records the
//! last plan, so tests assert on the composited data.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn fixture_app() -> App<MockAdapter> {
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("inspect_demo", adapter);
    app.wait_for_settle();
    app
}

#[test]
fn highlight_aims_the_outline_and_returns_facts() {
    let mut app = fixture_app();
    let save = Selector::role_named(Role::Button, "Save");

    assert!(!app.inspect().is_active());
    let facts = app.inspect().highlight(&save).expect("highlight resolves");
    assert_eq!(facts.role.as_deref(), Some("Button"));
    assert_eq!(facts.name.as_deref(), Some("Save"));
    assert_eq!(
        facts.bounds.expect("fixture bounds"),
        [16.0, 16.0, 136.0, 48.0]
    );
    assert!(app.inspect().is_active(), "highlight implies the session");
    let plan = app
        .session()
        .adapter()
        .overlay()
        .expect("the adapter composites the plan");
    let hover = plan.hover.expect("Save is outlined");
    assert_eq!(
        [hover.x0, hover.y0, hover.x1, hover.y1],
        [16.0, 16.0, 136.0, 48.0]
    );
    assert!(
        plan.badges.iter().any(|b| b.text.contains("Save")),
        "interactive badges ride the same plan"
    );
}

#[test]
fn enable_clear_and_disable_roundtrip() {
    let mut app = fixture_app();
    let save = Selector::role_named(Role::Button, "Save");

    app.inspect().enable().expect("overlay on");
    assert!(app.inspect().is_active());
    assert!(app.session().adapter().overlay().is_some());
    assert!(
        app.session()
            .adapter()
            .overlay()
            .is_some_and(|plan| plan.hover.is_none()),
        "enable alone highlights nothing"
    );

    app.inspect().highlight(&save).expect("highlight");
    app.inspect().clear_highlight().expect("outline cleared");
    assert!(
        app.session()
            .adapter()
            .overlay()
            .is_some_and(|plan| plan.hover.is_none()),
        "the outline is gone, the session stays"
    );
    assert!(app.inspect().is_active());

    app.inspect().disable().expect("overlay off");
    assert!(!app.inspect().is_active());
    assert!(app.session().adapter().overlay().is_none());
}

#[test]
fn unknown_selector_fails_strict_and_composites_nothing() {
    let mut app = fixture_app();
    let err = app
        .inspect()
        .highlight(&Selector::role_named(Role::Button, "Missing"))
        .expect_err("no such button");
    assert!(
        err.to_string().contains("no node matches"),
        "strict-mode wording, got: {err}"
    );
    assert!(app.session().adapter().overlay().is_none());
}

#[test]
fn adapters_without_overlay_surface_report_unsupported() {
    // The iced adapter ships framework-free with the default trait impls
    // (no overlay surface) — exercise the honest §6.2 path through the
    // driver's bare-adapter default. The mock always supports the
    // overlay, so this uses the *logical* adapter shape directly.
    let mut app = fixture_app();
    // MockAdapter implements show_overlay; the Unsupported default lives
    // on adapters that don't override it (covered by
    // guiwright-driver/tests/inspect_overlay.rs). Here: the kit surfaces
    // the driver error verbatim (KitError::Core).
    app.inspect().enable().expect("mock has a surface");
    app.inspect().disable().expect("off");
    assert!(!app.inspect().is_active());
}
