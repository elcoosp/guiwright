//! Embedded slint adapter integration tests (architecture §4.4, §17 row 5,
//! m78).
//!
//! A real `i_slint_backend_testing` `TestingBackend` running a real
//! `.slint` app (stock `Button` + `LineEdit` + `Text`, compiled with
//! element debug info by the glue's build.rs) — driven through the
//! standard [`guiwright_driver::Session`] (embedded == remote semantics,
//! §5.1). Interactions inject real `WindowEvent`s through
//! `Window::dispatch_event` — "the same code path real winit events
//! take" (§4.4 door A).

use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_adapter_slint_glue::harness::SlintAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::{InputEvent, PointerButton};
use guiwright_driver::session::Session;
use slint::include_modules;

include_modules!();

fn counter_session() -> Session<SlintAdapter<CounterForm>> {
    let adapter = SlintAdapter::boot(CounterForm::new, (480.0, 320.0)).expect("harness boots");
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
fn logical_tree_resolves_slint_elements_with_bounds() {
    let mut session = counter_session();

    let resolved = session
        .resolve(&increment())
        .expect("slint button in the logical tree");
    let bounds = resolved.bounds.expect("button bounds");
    assert!(bounds.area() > 0.0, "logical-px bounds: {bounds:?}");

    session
        .resolve(&decrement())
        .expect("second button resolves");
    session.resolve(&name_input()).expect("line edit resolves");
    // Element names (debug info) land as test ids on the diff.
    let _ = &session;

    // The initial counter label reflects the boot state.
    session
        .resolve(&Selector::text("Count: 0"))
        .expect("counter label reflects boot state");
}

#[test]
fn semantic_click_drives_the_real_slint_counter() {
    let mut session = counter_session();

    session.click(&increment()).expect("click Increment");

    // The `clicked` callback flowed through the real event pipeline.
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
fn fill_types_through_the_real_line_edit() {
    let mut session = counter_session();

    session
        .fill(&name_input(), "Ada")
        .expect("fill the name field");

    // The two-way `text <=> name` binding updated the app property, and
    // the greeting label follows.
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("greeting reflects the typed name");
}

#[test]
fn synthetic_pointer_click_targets_the_button() {
    let mut session = counter_session();

    let resolved = session.resolve(&increment()).expect("resolve");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    // Real pointer pipeline: move → down → up (§9 fidelity rule) —
    // `WindowEvent::PointerMoved/Pressed/Released` through
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
    assert!(first > 5, "the full element tree is in the snapshot");
}

#[test]
fn mock_clock_advances_virtual_time() {
    let mut session = counter_session();

    session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::InstallVirtual)
        .expect("install");
    let state = session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::FastForward(
            Duration::from_millis(1_500),
        ))
        .expect("fast forward");
    let virtual_time = state.virtual_time.expect("virtual clock installed");
    assert!(
        virtual_time >= Duration::from_millis(1_500),
        "mock_elapsed_time advanced: {virtual_time:?}"
    );
}

#[test]
fn render_frame_produces_real_pixels() {
    let mut session = counter_session();

    let frame = session
        .adapter_mut()
        .render_frame(Some((240, 180)))
        .expect("headless render");
    assert_eq!(frame.size, (240, 180), "snapshot at the requested size");
    let pixels = frame
        .pixels
        .expect("the software rasterizer produced pixels");
    assert!(!pixels.is_empty());
    // Not a blank canvas: at least one non-transparent pixel.
    assert!(
        pixels.chunks_exact(4).any(|px| px[3] != 0),
        "the snapshot has painted pixels"
    );
}
