//! Per-window recording on the kit surface (the m6 §5/m7 §4 deferred half
//! — "the trace/video services record the session; window-tagged streams
//! ride the same handle vocabulary in a follow-up"): the multi-window
//! fixture grafts two windows (Editor, Preview); the recording and the
//! trace each aim at one of them.
//!
//! ```text
//! cargo run -p guiwright-kit --example window_recording
//! ```
//!
//! The scoped video pins the Editor window at start (the typed
//! `UnknownWindow` refusal lands there for a dead handle) and every
//! presented frame crops to the window's subtree bounds — a Preview-only
//! change sheds its frames instead of entering the artifact. The scoped
//! trace stamps the archive's `meta.json` with the window handle, so the
//! artifact names the stream it recorded.

use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "window_recording",
        MockAdapter::with_multi_window(&fx).with_frame_pixels(320, 480, vec![18; 320 * 480 * 4]),
    );
    app.wait_for_settle();
    let editor = fx.w1.0.to_string();
    let preview = fx.w2.0.to_string();
    println!("targets: Editor {editor}, Preview {preview}");

    // --- The scoped video stream (the m6 §5/m7 §4 deferred half) -------
    let dir = std::env::temp_dir().join(format!("gw-window-recording-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    let mut video = app
        .window(&editor)
        .expect("Editor is a live target")
        .video();
    video
        .start_in(&dir, 25)
        .expect("the scoped recording starts");
    println!(
        "video start_in({}) — scoped to window {editor} (validated at start)",
        dir.display()
    );

    // Drive the session: frames crop to the Editor subtree; a
    // Preview-only change would shed its frames here.
    app.session().pump();
    app.session().pump();
    let summary = video.stop().expect("the scoped recording finalizes");
    println!(
        "video stop → {} (frames {}, dropped {}, {}ms) — the Editor crop",
        summary.path.display(),
        summary.frames,
        summary.dropped,
        summary.duration_ms
    );

    // The same handle aimed at the other window answers with its own
    // stream shape — the scope, not the script, decides what is kept.
    let mut preview_video = app.window(&preview).expect("Preview").video();
    preview_video.start_in(&dir, 25).expect("starts");
    app.session().pump();
    let preview_summary = preview_video.stop().expect("finalizes");
    println!(
        "video stop → {} (frames {}, dropped {}, {}ms) — the Preview crop",
        preview_summary.path.display(),
        preview_summary.frames,
        preview_summary.dropped,
        preview_summary.duration_ms
    );

    // --- The window-tagged trace stream --------------------------------
    let trace_path = dir.join("editor-trace.zip");
    {
        let mut scoped = app.window(&editor).expect("Editor");
        scoped.enable_trace(&trace_path);
        scoped
            .get_by_role(accesskit::Role::Button, "Save")
            .click()
            .expect("the scoped action rides the tagged trace");
    }
    app.finish().expect("the trace finalizes");
    println!(
        "trace → {} (meta.json carries window {editor})",
        trace_path.display()
    );

    // Read the tag back out of the archive's meta.
    use std::io::Read as _;
    let archive = zip::ZipArchive::new(std::fs::File::open(&trace_path).expect("zip"))
        .expect("archive opens");
    let mut meta_member = archive
        .by_name("meta.json")
        .expect("the archive carries meta.json");
    let mut meta_bytes = Vec::new();
    let _ = meta_member.read_to_end(&mut meta_bytes);
    let meta: serde_json::Value = serde_json::from_slice(&meta_bytes).expect("meta json parses");
    println!("meta.json window = {:?}", meta.get("window"));

    let _ = std::fs::remove_dir_all(&dir);
}
