//! The §13 inspector overlay binding proof over the floem mirror (m86) —
//! the engine overlay host's present route, driven by a real `Session`:
//! the plan flows through `show_overlay` into PaintOps on the present
//! path, the composited frame differs from the bare app's pixels, the
//! draw-call log names the draws, and `DisableOverlay` restores the bare
//! frame byte-for-byte. This is the positive half of the 7/7 overlay
//! count; the `compile_fail` doc test in `src/lib.rs` pins the negative.

use guiwright_core::Selector;
use guiwright_downstream_glue_floem::adapter::counter_app;
use guiwright_downstream_glue_floem::FloemDownstreamGlue;
use guiwright_driver::session::Session;
use guiwright_driver::FrameworkAdapter;

use std::cell::RefCell;
use std::rc::Rc;

fn counter_session() -> Session<FloemDownstreamGlue> {
    let count = Rc::new(RefCell::new(0_u64));
    let label = Rc::new(RefCell::new("Count: 0".to_string()));
    let adapter = FloemDownstreamGlue::boot((480.0, 240.0), || {
        counter_app(Rc::clone(&count), Rc::clone(&label))
    })
    .expect("boot");
    let mut session = Session::new(adapter);
    session.pump();
    session
}

/// The §13 pixel-diff proof (the slint/iced/freya evidence shape): with
/// the overlay session on, the adapter composites the hover outline +
/// role badge onto the tiny-skia surface at present time — the frame
/// pixels differ from the bare app's baseline and the digest moves.
#[test]
fn overlay_pixel_diff_is_non_zero() {
    let mut session = counter_session();

    // Baseline: the bare app, no overlay session (§10 capture path).
    let baseline = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render");
    let base = baseline.pixels.expect("tiny-skia pixels");

    // The overlay session: badges on interactive widgets + the hover
    // outline on the resolved Increment button.
    session
        .inspect_enable_overlay()
        .expect("overlay session on");
    session
        .inspect_highlight(&Selector::test_id("increment"))
        .expect("highlight resolves");

    let overlaid = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render with the overlay");
    let over = overlaid.pixels.expect("tiny-skia pixels");

    assert_eq!(base.len(), over.len(), "same viewport");
    let diff = base.iter().zip(over.iter()).filter(|(a, b)| a != b).count();
    assert!(diff > 0, "the overlay painted onto the scene: {diff} bytes");
    assert!(
        overlaid.stamp.hash != baseline.stamp.hash,
        "frame digest moved"
    );

    // The §13 draw-call evidence: the host's present route logged the
    // draws (outline + badges).
    let log = session.adapter_mut().take_overlay_paint_log();
    assert!(
        log.iter().any(|line| line.starts_with("outline")),
        "the outline draw is logged: {log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains("badge")),
        "the badge draws are logged: {log:?}"
    );

    // `DisableOverlay` ends the session: the next render is the bare
    // frame again — byte-for-byte (the deterministic CPU raster).
    session
        .inspect_disable_overlay()
        .expect("session off");
    let restored = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render after disable");
    let back = restored.pixels.expect("tiny-skia pixels");
    assert_eq!(
        base.as_ref(),
        back.as_ref(),
        "DisableOverlay restores the bare frame"
    );
}
