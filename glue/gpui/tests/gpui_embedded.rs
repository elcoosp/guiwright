//! Embedded gpui adapter integration tests (architecture §4.5, §17 row
//! 7, m80).
//!
//! A real gpui `test-support` platform — `TestAppContext::build` over a
//! seeded `TestDispatcher`, a real test window opened on the real
//! `TestPlatform` — running a real gpui counter+form app (`div()`-based
//! buttons with real `on_click` listeners, a focused text field with a
//! real `on_key_down` handler), driven through the standard
//! [`guiwright_driver::Session`] (embedded == remote semantics, §5.1).
//! Input goes through the canonical seam, `Window::dispatch_event`
//! (§4.5 "synthetic events are bit-identical to OS input"), and the
//! tree folds the framework's **real laid-out geometry** — the bounds
//! gpui records per debug element at paint — through the engine's
//! a11y-hybrid bookkeeping.

use std::time::{Duration, Instant};

use accesskit::Role;

use guiwright_adapter_gpui_glue::harness::GpuiAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::{ClockCmd, InputEvent, PointerButton};
use guiwright_driver::session::Session;

fn counter_session() -> Session<GpuiAdapter> {
    let adapter = GpuiAdapter::boot((480.0, 320.0), 0).expect("harness boots");
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    session
}

fn increment() -> Selector {
    Selector::role_named(Role::Button, "Increment")
}

fn decrement() -> Selector {
    Selector::role_named(Role::Button, "Decrement")
}

fn name_input() -> Selector {
    Selector::role(Role::TextInput)
}

#[test]
fn logical_tree_resolves_gpui_elements_with_bounds() {
    let mut session = counter_session();

    let resolved = session
        .resolve(&increment())
        .expect("gpui button in the logical tree");
    let bounds = resolved.bounds.expect("button bounds");
    assert!(bounds.area() > 0.0, "logical-px bounds: {bounds:?}");

    session
        .resolve(&decrement())
        .expect("second button resolves");
    session.resolve(&name_input()).expect("text field resolves");

    // The initial counter label reflects the boot state (a real
    // framework-painted fact, read through the app's real state).
    session
        .resolve(&Selector::text("Count: 0"))
        .expect("counter label reflects boot state");
}

#[test]
fn semantic_click_drives_the_real_gpui_counter() {
    let mut session = counter_session();

    session.click(&increment()).expect("click Increment");

    // The `on_click` listener flowed through the real event pipeline:
    // `Window::dispatch_event` → hit-testing → listener → entity update.
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");

    session.click(&decrement()).expect("click Decrement");
    session.click(&decrement()).expect("click Decrement again");
    session
        .resolve(&Selector::text("Count: -1"))
        .expect("counter decremented twice");
}

#[test]
fn synthetic_pointer_click_targets_the_button() {
    let mut session = counter_session();

    let resolved = session.resolve(&increment()).expect("resolve");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    // Real pointer pipeline: move → down → up (§9 fidelity rule) —
    // each step wraps a real `PlatformInput` and rides
    // `Window::dispatch_event`.
    session
        .adapter_mut()
        .input(InputEvent::PointerMove { x: cx, y: cy })
        .expect("move");
    session
        .adapter_mut()
        .input(InputEvent::PointerDown {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .expect("down");
    session
        .adapter_mut()
        .input(InputEvent::PointerUp {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .expect("up");
    session.settle(Instant::now() + Duration::from_secs(1));

    session
        .resolve(&Selector::text("Count: 1"))
        .expect("pointer pipeline incremented the counter");
}

#[test]
fn fill_reports_the_upstream_scheduler_blocker() {
    let mut session = counter_session();

    // The form field resolves with real bounds (tree/input are real —
    // §4.5's non-negotiable).
    let resolved = session.resolve(&name_input()).expect("text input resolves");
    // The empty field is content-sized (zero height until text lands);
    // the fact that it resolves with a recorded rect is the real part.
    assert!(
        resolved.bounds.is_some(),
        "the field carries laid-out bounds"
    );

    // `fill` is the second documented upstream blocker (m80): the real
    // KeyDown path panics inside zed's test scheduler —
    // `scheduler/src/test_scheduler.rs:232` (`state.next_session_id.0
    // += 1` overflow) — reproducible with three real `Keystroke`
    // dispatches after a real focus click. The adapter gates before
    // the sequence and reports the capability row honestly instead of
    // ever panicking or mocking.
    // Drive the adapter gate directly: the empty field is
    // content-sized (zero height), so the session's actionability gates
    // stop a `Session::fill` before `act` ever runs — the adapter's own
    // `SetValue` gate is the surface that carries the evidence.
    let node = match &resolved.key {
        guiwright_core::NodeKey::AccessKit { node, .. } => *node,
        other => panic!("unexpected node key: {other:?}"),
    };
    let err = session
        .adapter_mut()
        .act(
            node,
            guiwright_driver::adapter::SemAction::SetValue {
                text: "Ada".to_string(),
            },
        )
        .expect_err("fill is the documented upstream blocker");
    assert!(
        err.to_string().contains("test_scheduler.rs:232"),
        "the error carries the file-level evidence: {err}"
    );

    // The core loop keeps working around the blocked cell: the counter
    // still clicks through the real pipeline.
    session.click(&increment()).expect("click Increment");
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");
}

#[test]
fn logical_snapshot_is_stable_across_pumps() {
    let mut session = counter_session();
    let first = session
        .adapter_mut()
        .logical_snapshot()
        .expect("logical diff")
        .update
        .nodes
        .len();
    let second = session
        .adapter_mut()
        .logical_snapshot()
        .expect("logical diff")
        .update
        .nodes
        .len();
    assert_eq!(first, second, "tree size stable across re-snapshots");
    assert!(first > 4, "the full element set is in the snapshot");
}

#[test]
fn dispatcher_rounds_advance_virtual_time() {
    let mut session = counter_session();

    session
        .adapter_mut()
        .clock(ClockCmd::InstallVirtual)
        .expect("install");
    let state = session
        .adapter_mut()
        .clock(ClockCmd::FastForward(Duration::from_millis(1_500)))
        .expect("fast forward");
    let virtual_time = state.virtual_time.expect("virtual clock installed");
    assert!(
        virtual_time >= Duration::from_millis(1_500),
        "dispatcher rounds advanced virtual time: {virtual_time:?}"
    );
}

#[test]
fn scene_digest_reflects_painted_content() {
    let mut session = counter_session();

    // The §15.5 strided digest rides the real painted scene (the test
    // window's rendered frame), not a mock: clicking changes the
    // painted text, so the digest changes with it.
    let before = session
        .adapter_mut()
        .last_frame()
        .expect("scene frame")
        .stamp
        .hash;
    session.click(&increment()).expect("click Increment");
    let after = session
        .adapter_mut()
        .last_frame()
        .expect("scene frame")
        .stamp
        .hash;
    assert_ne!(before, after, "the painted scene digest follows content");
}

/// The m87 capture (§4.5): `render_frame` returns REAL pixels from the
/// test-support painted-quad scene — the raster at the requested size,
/// painted surfaces on it, and the §15.5 strided digest over the bytes
/// (the hash-only row ended).
#[test]
fn render_frame_rasterizes_the_painted_scene() {
    let mut session = counter_session();

    let frame = session
        .adapter_mut()
        .render_frame(Some((240, 180)))
        .expect("headless render");
    assert_eq!(frame.size, (240, 180), "raster at the requested size");
    let pixels = frame.pixels.expect("the capture produced pixels");
    assert!(!pixels.is_empty());
    // Not a blank canvas: the app paints a real base + buttons + field
    // + count bar (the m87 real-background app), so painted pixels
    // exist and nothing is fully transparent.
    assert!(
        pixels.as_chunks::<4>().0.iter().any(|px| px[3] != 0),
        "the snapshot has painted pixels"
    );
    assert!(
        pixels.as_chunks::<4>().0.iter().any(|px| *px != [0, 0, 0, 0]),
        "the raster carries real scene content"
    );

    // The viewport-sized readback matches the window's logical size.
    let sized = session.adapter_mut().render_frame(None).expect("render");
    assert_eq!(sized.size, (480, 320), "the viewport raster");
    assert!(sized.pixels.expect("pixels").len() >= 480 * 320 * 4);
}
