//! The downstream gpui binding proof (the contract's "canonical binding
//! example" test half): the framework-free engine
//! (`guiwright-adapter-gpui`) binds to the gpui-shaped mirror and a real
//! `Session` drives the counter app end to end — resolve over the Level-1
//! a11y restatements → click through `dispatch_event` → `expect` over the
//! consumer tree → the §14 seam through `run_in`, plus the
//! test-support-gated pixel path with the strided digest. The
//! compile-fail doc test in `src/lib.rs` pins the negative side (a
//! non-gpui window cannot bind); this file is the positive.

use guiwright_core::MAIN_WINDOW;
use guiwright_core::selector::Selector;
use guiwright_downstream_glue_gpui::adapter::GpuiDownstreamGlue;
use guiwright_downstream_glue_gpui::mirror::{
    App, CounterState, Entity, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, Point,
    VisualTestContext, counter_window,
};
use guiwright_driver::adapter::{FrameworkAdapter, SemAction};
use guiwright_driver::session::ActionOpts;

/// The §6.2 matrix GPUI column, realized with `test_support = true`: the
/// hybrid row (Level-1 a11y + input), the test-support-gated capture
/// rows, the dispatcher clock and the seam surface.
#[test]
fn adapter_caps_match_the_contract() {
    let entity = Entity::new(CounterState {
        count: 0,
        label: "Count: 0".to_string(),
    });
    let adapter = GpuiDownstreamGlue::boot(entity).expect("boot");
    let caps = adapter.meta().caps;
    assert!(caps.tree_update, "Level-1: unconditional AccessKit (§4.5)");
    assert!(
        caps.logical_snapshot,
        "hybrid: the logical engine is the fallback (§6.1)"
    );
    assert!(caps.input, "dispatch_event is the canonical seam (§4.5)");
    assert!(
        caps.render_frame && caps.last_frame,
        "test-support-gated capture rows ride with the feature on (§11)"
    );
    assert!(caps.clock, "the dispatcher plan maps regardless");
}

/// The end-to-end bind: Session over the mirror app — the same
/// conversation the real `glue/gpui` tests drive against the real
/// test-support platform, unchanged.
#[test]
fn session_drives_the_counter_app_end_to_end() {
    let entity = Entity::new(CounterState {
        count: 0,
        label: "Count: 0".to_string(),
    });
    let adapter = GpuiDownstreamGlue::boot(entity.clone()).expect("boot");
    let mut session = guiwright_driver::Session::new(adapter);
    session.pump();

    // Single-window persona (§16 v0.4 default).
    assert_eq!(session.windows()[0].handle, MAIN_WINDOW);

    // Resolve over the Level-1 restatement: the app's stable NodeIds are
    // the session's keys, bounds ride the a11y update.
    let increment = Selector::role_named(accesskit::Role::Button, "Increment");
    let resolved = session.resolve(&increment).expect("the button resolves");
    assert!(
        resolved.bounds.is_some(),
        "the restatement carries the a11y bounds"
    );

    // Click through the REAL (mirrored) dispatch pipeline: the a11y node
    // routes to synthesized mouse events at its bounds (§8.4); the
    // entity state mutates through the window's behavior.
    let outcome = session
        .perform_action(&increment, SemAction::Click, &ActionOpts::default())
        .expect("the click applies at a frame");
    assert_eq!(outcome.detail, "clicked");
    entity.read(|state| assert_eq!(state.count, 1, "the app state mutated through the UI"));

    // The §16 retry assertion over the consumer tree: the next pump
    // restates the label with the incremented count.
    session
        .expect(Selector::text("Count: 1"))
        .to_have_text("Count: 1")
        .expect("the restated a11y label carries the new count");

    // Fill is gated with the m80 evidence (the zed scheduler blocker),
    // identically to the real glue.
    let err = session
        .perform_action(
            &increment,
            SemAction::SetValue { text: "Ada".into() },
            &ActionOpts::default(),
        )
        .expect_err("the fill blocker gates SetValue");
    assert!(
        err.to_string().contains("scheduler"),
        "the Unsupported carries the contract evidence: {err}"
    );

    // The §14 seam through run_in: the hook writes the runner context
    // (the entity's state handed over as `&mut dyn Any`).
    let mut registry = guiwright_driver::SeamRegistry::new();
    registry.register("reset_state", |ctx, _scope| {
        let state = ctx
            .downcast_mut::<CounterState>()
            .ok_or("runner context is not the counter state")?;
        state.count = 0;
        state.label = "Count: 0".to_string();
        Ok("reset".to_string())
    });
    session.register_seams(&registry);
    let detail = session
        .run_seam("reset_state", "fresh")
        .expect("the seam runs in the runner context");
    assert_eq!(detail, "reset");
    entity.read(|state| assert_eq!(state.count, 0, "the seam reset the app state"));

    // The test-support pixel path: one rendered_frame readback, PNG
    // encoded for the wire (§10).
    let shot = session.screenshot_window().expect("render_frame");
    assert_eq!((shot.width, shot.height), (480, 240));
    assert!(shot.png.len() > 64, "the PNG is real bytes");
}

/// The `VisualTestContext` dispatch shape (the §4.5 test-support
/// vocabulary rides the same `PlatformInput` choke point): the restated
/// a11y bounds feed the window's hit-testing, so a synthesized click
/// through the context fires the button's behavior.
#[test]
fn visual_test_context_dispatches_through_the_window() {
    let entity = Entity::new(CounterState {
        count: 0,
        label: "Count: 0".to_string(),
    });
    let (window, entity) = counter_window(&entity);
    // The first a11y restatement (what a pump would take) — the bounds
    // the window's hit-tests read.
    window.borrow_mut().a11y_tree_update();
    let cx = App::new();
    let mut vt = VisualTestContext::from_window(&window, &cx);

    let moved = vt.dispatch(PlatformInput::MouseMove(MouseMoveEvent {
        position: Point { x: 8.0, y: 8.0 },
    }));
    assert!(moved.propagate, "moves propagate");

    let clicked = vt.dispatch(PlatformInput::MouseDown(MouseDownEvent {
        position: Point { x: 76.0, y: 32.0 },
        button: guiwright_downstream_glue_gpui::mirror::MouseButton::Left,
        click_count: 1,
    }));
    let _ = clicked;
    vt.dispatch(PlatformInput::MouseUp(MouseUpEvent {
        position: Point { x: 76.0, y: 32.0 },
        button: guiwright_downstream_glue_gpui::mirror::MouseButton::Left,
        click_count: 1,
    }));
    entity.read(|state| assert_eq!(state.count, 1, "the window's behavior fired"));
}
