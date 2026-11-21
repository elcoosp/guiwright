//! The slint inspector overlay (§4.4 "top-level overlay window", §13, m84).
//!
//! Slint's software renderer exposes no adapter-level composition slot (no
//! egui painter layer, no masonry/freya overlay stack), so the §13 overlay
//! plan composites through a **borderless second `.slint` window**
//! registered in the same `TestingBackend` thread-local window registry as
//! the app window: it mirrors the app window's geometry so the §9
//! window-relative rects land 1:1, renders the hover outlines + role/TestId
//! badges from the plan, and is captured through the same
//! `Window::take_snapshot` raster path the app window uses — blended onto
//! the presented frame at capture time (straight-alpha source-over; the
//! software renderer's snapshot background is fully transparent, so only
//! the painted overlay moves the pixels).
//!
//! The tests assert the §13 evidence chain end to end: the plan reaches the
//! adapter through the session's `Inspect` domain, the composited pixels
//! move against the bare app (the §4.4 pixel-diff proof), the frame digest
//! moves, an empty plan leaves the frame byte-identical, and the paint log
//! records the draw calls the overlay made.

use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_adapter_slint_glue::harness::SlintAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
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

/// The §13 pixel-diff proof (the freya m83 shape): with the overlay session
/// on and a highlight resolved, the presented frame's pixels move against
/// the bare app.
#[test]
fn overlay_pixel_diff_is_non_zero() {
    let mut session = counter_session();

    // Baseline: the bare app, no overlay session (§10 capture path).
    let baseline = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render");
    let base = baseline.pixels.expect("software raster pixels");

    // The overlay session: badges on the interactive elements + the hover
    // outline on the resolved Increment button.
    session
        .inspect_enable_overlay()
        .expect("overlay session on");
    session
        .inspect_highlight(&increment())
        .expect("highlight resolves");

    let overlaid = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render with the overlay");
    let over = overlaid.pixels.expect("software raster pixels");

    assert_eq!(base.len(), over.len(), "same viewport");
    let diff = base.iter().zip(over.iter()).filter(|(a, b)| a != b).count();
    assert!(diff > 0, "the overlay painted onto the scene: {diff} bytes");
    assert_ne!(
        overlaid.stamp.hash, baseline.stamp.hash,
        "the frame digest moved"
    );
}

/// `DisableOverlay` ends the session: the overlay window leaves the
/// registry and the frame returns to the bare app's pixels.
#[test]
fn disable_overlay_restores_the_bare_frame() {
    let mut session = counter_session();

    let baseline = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render");
    let base = baseline.pixels.expect("software raster pixels");

    session
        .inspect_enable_overlay()
        .expect("overlay session on");
    session.inspect_highlight(&increment()).expect("highlight");
    let overlaid = session
        .adapter_mut()
        .render_frame(None)
        .expect("render with the overlay");
    assert_ne!(
        overlaid.pixels.expect("pixels").as_ref(),
        base.as_ref(),
        "the overlay painted"
    );

    session
        .inspect_disable_overlay()
        .expect("overlay session off");
    let cleared = session
        .adapter_mut()
        .render_frame(None)
        .expect("render after disable");
    assert_eq!(
        cleared.pixels.expect("pixels").as_ref(),
        base.as_ref(),
        "the bare app frame is restored byte-for-byte"
    );
}

/// The draw-call evidence (§13): the overlay's paint log names the outline
/// rect and the badge chips it composited — the same log shape the egui/
/// masonry/freya overlays keep.
#[test]
fn overlay_paint_log_names_the_draws() {
    let mut session = counter_session();

    session
        .inspect_enable_overlay()
        .expect("overlay session on");
    session.inspect_highlight(&increment()).expect("highlight");

    let log = session
        .adapter_mut()
        .take_overlay_paint_log()
        .expect("the overlay session painted");
    assert!(
        log.iter().any(|line| line.starts_with("outline ")),
        "the hover outline is logged: {log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.starts_with("badge ") && line.contains("increment-btn")),
        "the interactive button badge is logged (test ids beat names, §13): {log:?}"
    );

    session.inspect_disable_overlay().expect("session off");
    let cleared = session.adapter_mut().take_overlay_paint_log();
    assert!(
        cleared.is_none(),
        "a cleared session leaves no draws to log"
    );
}
