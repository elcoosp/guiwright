//! Per-window recording on the kit surface (the m6 §5/m7 §4 deferred
//! half, embedded half): `app.window(handle).video()` pins the recording
//! to that window's subtree — every presented frame is cropped to the
//! window's bounds, a change outside the scope sheds its frames — and
//! `app.window(handle).enable_trace(path)` starts the session trace with
//! the window tag in the archive's `meta.json`, so the artifact names
//! the stream it recorded. The unscoped surfaces keep their exact
//! pre-scope shape (`app.video()` / `app.enable_trace()`).
//!
//! The multi-window fixture grafts two windows whose scripted bounds are
//! disjoint (Editor's Save at 16,16→136,48; Preview's Save + label at
//! 12,416→212,480), so the crop digests discriminate the scopes exactly
//! like the driver-level test (`window_scoped_video.rs`) proves at the
//! session boundary.

use accesskit::Role;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

const CANVAS: (u32, u32) = (256, 512);

fn fill_rect(rgba: &mut [u8], width: u32, rect: (f64, f64, f64, f64), value: u8) {
    let (x0, y0, x1, y1) = rect;
    for y in y0 as u32..y1 as u32 {
        for x in x0 as u32..x1 as u32 {
            let idx = ((y * width + x) * 4) as usize;
            rgba[idx] = value;
        }
    }
}

/// base → W1-only change → W2-only change → second W1-only change: the
/// mock advances one step per `last_frame` render, so a recording's crop
/// digests move exactly when the scoped window's pixels do.
fn scripted_steps() -> Vec<Vec<u8>> {
    let (w, _h) = CANVAS;
    let base = vec![10u8; (CANVAS.0 * CANVAS.1 * 4) as usize];
    let mut w1_change = base.clone();
    fill_rect(&mut w1_change, w, (16.0, 16.0, 136.0, 48.0), 90);
    let mut w2_change = w1_change.clone();
    fill_rect(&mut w2_change, w, (12.0, 416.0, 212.0, 480.0), 90);
    let mut w1_again = w2_change.clone();
    fill_rect(&mut w1_again, w, (16.0, 16.0, 136.0, 48.0), 140);
    // The mock's render advances the sequence BEFORE returning — the
    // base frame is listed twice so the first crop is the unchanged
    // canvas (the driver-level test's script, mirrored).
    vec![base.clone(), base, w1_change, w2_change, w1_again]
}

fn multi_window_pixel_app() -> (App<MockAdapter>, MultiWindowFixture, String, String) {
    let fx = MultiWindowFixture::default();
    let adapter = MockAdapter::with_multi_window(&fx).with_frame_pixel_steps(
        CANVAS.0,
        CANVAS.1,
        scripted_steps(),
    );
    let mut app = App::launch("kit_window_recording", adapter);
    app.wait_for_settle();
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();
    (app, fx, w1, w2)
}

/// The window-scoped video stream: the Editor window's crop moves twice
/// across the scripted stream and the Preview-only change sheds — the
/// same per-window answer the session boundary gives, through the kit's
/// borrow-and-act handle.
#[test]
fn window_handle_video_records_its_window() {
    let (mut app, _fx, w1, _w2) = multi_window_pixel_app();
    let dir = tempfile::tempdir().expect("tmpdir");

    // The handle is a borrow-and-act view: the start pins the scope at
    // the session (validated there), the pumps ride `app.session()`, and
    // a re-aimed handle finalizes — no live borrow spans the pumps.
    {
        let mut editor = app.window(&w1).expect("Editor");
        editor.video().start_in(dir.path(), 25).expect("scoped start");
    }
    app.session().pump();
    app.session().pump();
    app.session().pump();
    app.session().pump();
    let summary = app
        .window(&w1)
        .expect("the Editor target stays live")
        .video()
        .stop()
        .expect("stop");

    assert_eq!(
        summary.frames, 3,
        "base + the two Editor changes (the stop pump carries the last)"
    );
    assert_eq!(
        summary.dropped, 2,
        "the base repeat + the Preview change shed"
    );
    assert!(
        summary.path.exists(),
        "finalized at {}",
        summary.path.display()
    );
}

/// The unscoped kit surface keeps its exact pre-scope shape: stamp-hash
/// dedup over the full canvas, no crop (static pixels — the script's
/// hashes, not pixel content, decide what survives).
#[test]
fn the_unscoped_kit_video_keeps_its_shape() {
    let fx = MultiWindowFixture::default();
    let adapter =
        MockAdapter::with_multi_window(&fx).with_frame_pixels(320, 480, vec![18; 320 * 480 * 4]);
    let mut app = App::launch("kit_unscoped_video", adapter);
    app.wait_for_settle();
    let dir = tempfile::tempdir().expect("tmpdir");

    app.video().start_in(dir.path(), 25).expect("start");
    app.session().adapter_mut().push_frame(0x11);
    app.session().pump();
    app.session().adapter_mut().push_frame(0x11);
    app.session().pump();
    app.session().adapter_mut().push_frame(0x22);
    app.session().pump();
    let summary = app.video().stop().expect("stop");

    assert_eq!(summary.frames, 3, "0x11 + 0x22 + the stop-pump frame");
    assert_eq!(summary.dropped, 1, "the static span collapsed");
}

/// The window-tagged trace: the archive's `meta.json` names the window
/// the stream recorded; an untagged trace carries no window key.
#[test]
fn window_handle_trace_names_its_window() {
    use std::io::Read as _;

    let (mut app, _fx, w1, _w2) = multi_window_pixel_app();
    let dir = tempfile::tempdir().expect("tmpdir");
    let trace_path = dir.path().join("scoped-trace.zip");

    {
        let mut scoped = app.window(&w1).expect("Editor");
        scoped.enable_trace(&trace_path);
        scoped
            .get_by_role(Role::Button, "Save")
            .click()
            .expect("a scoped action rides the trace");
    }
    app.finish().expect("the trace finalizes");

    let archive_file = std::fs::File::open(&trace_path).expect("zip opens");
    let mut archive = zip::ZipArchive::new(archive_file).expect("zip reads");
    let mut meta_member = archive
        .by_name("meta.json")
        .expect("the archive carries meta.json");
    let mut meta_bytes = Vec::new();
    meta_member
        .read_to_end(&mut meta_bytes)
        .expect("meta bytes");
    let meta: serde_json::Value = serde_json::from_slice(&meta_bytes).expect("meta json parses");
    assert_eq!(
        meta.get("window").and_then(|w| w.as_str()),
        Some(w1.as_str()),
        "the archive names the window it recorded: {meta}"
    );
}

/// The untagged trace keeps the pre-tag shape: no `window` key in the
/// meta (the field is skipped when absent, so older readers of the
/// archive see byte-identical metadata).
#[test]
fn the_untagged_trace_has_no_window_key() {
    use std::io::Read as _;

    let fx = MultiWindowFixture::default();
    let adapter = MockAdapter::with_multi_window(&fx).with_frame_pixel_steps(
        CANVAS.0,
        CANVAS.1,
        scripted_steps(),
    );
    let mut app = App::launch("kit_untagged_trace", adapter);
    app.wait_for_settle();
    let dir = tempfile::tempdir().expect("tmpdir");
    let trace_path = dir.path().join("untagged-trace.zip");

    app.enable_trace(&trace_path);
    app.finish().expect("the trace finalizes");

    let archive_file = std::fs::File::open(&trace_path).expect("zip opens");
    let mut archive = zip::ZipArchive::new(archive_file).expect("zip reads");
    let mut meta_member = archive
        .by_name("meta.json")
        .expect("the archive carries meta.json");
    let mut meta_bytes = Vec::new();
    meta_member
        .read_to_end(&mut meta_bytes)
        .expect("meta bytes");
    let meta: serde_json::Value = serde_json::from_slice(&meta_bytes).expect("meta json parses");
    assert!(
        meta.get("window").is_none(),
        "an untagged archive carries no window key: {meta}"
    );
}
