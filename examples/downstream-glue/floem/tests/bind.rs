//! The downstream floem binding proof (the contract's "canonical binding
//! example" test half): the framework-free engine
//! (`guiwright-adapter-floem`) binds to the floem-shaped mirror and a
//! real `Session` drives the counter app end to end — resolve → strict
//! resolution → click through the real hit-testing pipeline → `expect`
//! over the synthesized Level-2 tree, plus the engine tiny-skia pixel
//! path. The compile-fail doc test in `src/lib.rs` pins the negative
//! side (a non-floem harness cannot bind); this file is the positive.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use guiwright_core::selector::Selector;
use guiwright_downstream_glue_floem::FloemDownstreamGlue;
use guiwright_downstream_glue_floem::adapter::counter_app;
use guiwright_driver::adapter::{FrameworkAdapter, SemAction};
use guiwright_driver::session::ActionOpts;

/// The §6.2 matrix floem column + the PaintState patch, realized: the
/// adapter advertises logical_snapshot/input/render_frame/clock and no
/// tree_update — and the session sees exactly that.
#[test]
fn adapter_caps_match_the_contract() {
    let count = Rc::new(RefCell::new(0_u64));
    let label = Rc::new(RefCell::new("Count: 0".to_string()));
    let adapter = FloemDownstreamGlue::boot((480.0, 240.0), || {
        counter_app(Rc::clone(&count), Rc::clone(&label))
    })
    .expect("boot");
    let caps = adapter.meta().caps;
    assert!(!caps.tree_update, "Level-2: no AccessKit of its own (§6.1)");
    assert!(caps.logical_snapshot, "the ViewId walk is the tree path");
    assert!(caps.input, "dispatch_event is the real pipeline (§4.7)");
    assert!(
        caps.render_frame,
        "the engine tiny-skia PaintState patch is on (§4.7 contribution 2)"
    );
    assert!(!caps.last_frame, "headless: no GPU surface to read back");
    assert!(caps.clock, "the harness-timer plan maps (§14)");
}

/// The end-to-end bind: Session over the mirror app — the same
/// conversation the real `glue/floem` tests drive against the real
/// harness, unchanged.
#[test]
fn session_drives_the_counter_app_end_to_end() {
    let count = Rc::new(RefCell::new(0_u64));
    let label = Rc::new(RefCell::new("Count: 0".to_string()));
    let adapter = FloemDownstreamGlue::boot((480.0, 240.0), || {
        counter_app(Rc::clone(&count), Rc::clone(&label))
    })
    .expect("boot");
    let mut session = guiwright_driver::Session::new(adapter);
    session.pump();

    // Resolve through the synthesized Level-2 tree (the test id lands in
    // the session's O(1) hash from the engine's TestIdRegistry fold).
    let increment = Selector::test_id("increment");
    let resolved = session.resolve(&increment).expect("the button resolves");
    assert!(
        resolved.bounds.is_some(),
        "the walk carries get_visual_rect geometry"
    );

    // Strict-mode read of the label's shared state (the SemReader seam).
    let count_text = Selector::test_id("count_text");
    let label_node = session.resolve(&count_text).expect("the label resolves");

    // Click through the REAL (mirrored) hit-testing pipeline: move →
    // press → release at the visual rect center; the app state mutates.
    let outcome = session
        .perform_action(&increment, SemAction::Click, &ActionOpts::default())
        .expect("the click applies at a frame");
    assert_eq!(outcome.detail, "clicked");
    assert_eq!(*count.borrow(), 1, "the app state mutated through the UI");
    let _ = label_node;

    // The §16 retry assertion over the synthesized tree: the next
    // introspection pass sees the updated signal.
    session
        .expect(Selector::test_id("count_text"))
        .to_have_text("Count: 1")
        .expect("the label text rides the app state");

    // Fill is gated with the m80 evidence (the headless-IME blocker),
    // identically to the real glue.
    let err = session
        .perform_action(
            &count_text,
            SemAction::SetValue { text: "Ada".into() },
            &ActionOpts::default(),
        )
        .expect_err("the fill blocker gates SetValue");
    assert!(
        err.to_string().contains("IME"),
        "the Unsupported carries the contract evidence: {err}"
    );

    // The clock plan: install + one frame-sized step (§14).
    session
        .clock_fast_forward(Duration::from_millis(16))
        .expect("clock");

    // The engine pixel path: one tiny-skia PaintState render over the
    // laid-out view geometry, PNG-encoded for the wire (§10).
    let shot = session.screenshot_window().expect("render_frame");
    assert_eq!((shot.width, shot.height), (480, 240));
    assert!(shot.png.len() > 64, "the PNG is real bytes");
}
