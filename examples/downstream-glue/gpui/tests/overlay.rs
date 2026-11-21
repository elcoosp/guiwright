//! The §13 inspector overlay binding proof over the gpui mirror (m86) —
//! the engine overlay host's painted-quad route, driven by a real
//! `Session`: the plan flows through `show_overlay` into the mock
//! painted-quad scene, the composited raster differs from the bare
//! app's pixels, the draw-call log names the draws, and
//! `DisableOverlay` restores the bare frame byte-for-byte. This is the
//! positive half of the 7/7 overlay count; the `compile_fail` doc test
//! in `src/lib.rs` pins the negative.

use guiwright_core::Selector;
use guiwright_downstream_glue_gpui::mirror::{CounterState, Entity};
use guiwright_downstream_glue_gpui::GpuiDownstreamGlue;
use guiwright_driver::session::Session;
use guiwright_driver::FrameworkAdapter;

fn counter_session() -> Session<GpuiDownstreamGlue> {
    let entity = Entity::new(CounterState {
        count: 0,
        label: "Count: 0".to_string(),
    });
    let adapter = GpuiDownstreamGlue::boot(entity).expect("boot");
    let mut session = Session::new(adapter);
    session.pump();
    session
}

/// The §13 pixel-diff proof (the slint/iced/freya evidence shape): with
/// the overlay session on, the adapter composites the hover outline +
/// role badge onto the scripted test-support raster at present time —
/// the frame pixels differ from the bare app's baseline and the digest
/// moves.
#[test]
fn overlay_pixel_diff_is_non_zero() {
    let mut session = counter_session();

    // Baseline: the bare app, no overlay session (§10 capture path).
    let baseline = session
        .adapter_mut()
        .render_frame(None)
        .expect("test-support raster");
    let base = baseline.pixels.expect("scripted raster pixels");

    // The overlay session: badges on interactive widgets + the hover
    // outline on the resolved Increment button.
    session
        .inspect_enable_overlay()
        .expect("overlay session on");
    session
        .inspect_highlight(&Selector::role_named(
            accesskit::Role::Button,
            "Increment",
        ))
        .expect("highlight resolves");

    let overlaid = session
        .adapter_mut()
        .render_frame(None)
        .expect("test-support raster with the overlay");
    let over = overlaid.pixels.expect("scripted raster pixels");

    assert_eq!(base.len(), over.len(), "same viewport");
    let diff = base.iter().zip(over.iter()).filter(|(a, b)| a != b).count();
    assert!(diff > 0, "the overlay painted onto the scene: {diff} bytes");
    assert!(
        overlaid.stamp.hash != baseline.stamp.hash,
        "frame digest moved"
    );

    // The §13 draw-call evidence: the host's paint route logged the
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
    // frame again — byte-for-byte (the deterministic scripted raster).
    session.inspect_disable_overlay().expect("session off");
    let restored = session
        .adapter_mut()
        .render_frame(None)
        .expect("test-support raster after disable");
    let back = restored.pixels.expect("scripted raster pixels");
    assert_eq!(
        base.as_ref(),
        back.as_ref(),
        "DisableOverlay restores the bare frame"
    );
}
