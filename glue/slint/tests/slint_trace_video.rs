//! The slint trace + video end-to-end validation (§12/§10, m85).
//!
//! The §12 pipeline over a REAL slint session: the kit records the App's
//! actions into a `gwtrace/v1` archive, `guiwright show-trace` renders
//! the scrubber HTML, and the HTML carries the slint adapter header plus
//! the recorded events. The §10 row: presented frames flow from the
//! software rasterizer through the recorder (hash-equality dedup on the
//! static span, encoding on the writer thread) and finalize a real
//! artifact — non-zero size, ≥2 distinct frames (the fill repaints), a
//! wall-clock duration, and a valid container (ffmpeg EBML when the
//! binary is present, the PNG frame folder + `video.json` manifest
//! otherwise).

use accesskit::Role;
use guiwright_adapter_slint_glue::harness::SlintAdapter;
use guiwright_kit::App;
use slint::include_modules;

include_modules!();

fn counter_app() -> App<SlintAdapter<CounterForm>> {
    let adapter = SlintAdapter::boot(CounterForm::new, (480.0, 320.0)).expect("harness boots");
    App::launch("slint_trace_video", adapter)
}

/// The §12 pipeline over a real slint session (m85): record → finalize →
/// `guiwright show-trace` renders the scrubber HTML.
#[test]
fn trace_archive_renders_through_show_trace() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let archive_path = dir.path().join("guiwright-slint-trace.zip");
    let mut app = counter_app();
    app.enable_trace(&archive_path);

    // Drive real actions: the click and the fill land as trace events.
    app.get_by_role(Role::Button, "Increment")
        .click()
        .expect("click Increment");
    app.get_by_test_id("CounterForm::name-field")
        .fill("Ada")
        .expect("fill the name");

    let archive = app.finish().expect("trace finalized");
    assert_eq!(archive, archive_path, "the explicit path is honored");
    let written = std::fs::metadata(&archive).expect("archive exists").len();
    assert!(written > 0, "the archive is not empty ({written} bytes)");

    let html = guiwright_cli::show_trace(&archive, None, false).expect("html rendered");
    let rendered = std::fs::read_to_string(&html).expect("html written next to the archive");
    // The self-contained viewer payloads (§12): the schema marker, the
    // adapter header the attach report carried, the recorded events.
    assert!(rendered.contains("gwtrace"), "schema marker present");
    assert!(rendered.contains("slint"), "adapter name in the header");
    assert!(rendered.contains("click"), "the click event is in the HTML");
    assert!(rendered.contains("fill"), "the fill event is in the HTML");
}

/// The §10 recorder over real slint pixels (m85): presented frames flow
/// through the harness's raster path into the recorder; the static span
/// dedups, the repaint records, and the stop finalizes a valid artifact.
#[test]
fn video_records_presented_pixels() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut app = counter_app();

    app.video().start_in(dir.path(), 25).expect("video starts");
    app.session().pump();
    // A real wall-clock span between the first and the last accepted
    // frame (the summary's duration metadata, §10).
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.get_by_test_id("CounterForm::name-field")
        .fill("Ada")
        .expect("fill repaints");
    let summary = app.video().stop().expect("video stops");

    assert!(
        summary.frames >= 2,
        "the static span deduped and the repaint recorded: {summary:?}"
    );
    assert!(
        summary.dropped >= 1,
        "hash-equality dedup collapsed the static span: {summary:?}"
    );
    assert!(
        summary.path.exists(),
        "finalized at {}",
        summary.path.display()
    );
    let size = std::fs::metadata(&summary.path)
        .expect("artifact exists")
        .len();
    assert!(size > 0, "the artifact is not empty ({size} bytes)");
    assert!(
        summary.duration_ms >= 40,
        "the duration metadata spans the recorded window: {summary:?}"
    );
    is_valid_video(&summary.path);
}

/// The artifact is a real video: ffmpeg writes an EBML `.webm` container;
/// the PNG-sequence fallback writes a `video.json` manifest whose frame
/// count matches the summary (the duration metadata rides fps × frames).
fn is_valid_video(path: &std::path::Path) {
    if path.is_file() {
        let bytes = std::fs::read(path).expect("read the webm");
        assert_eq!(
            bytes.get(0..4).expect("magic bytes"),
            &[0x1A, 0x45, 0xDF, 0xA3],
            "the webm carries the EBML magic"
        );
    } else {
        let manifest = std::fs::read_to_string(path.join("video.json")).expect("manifest");
        let manifest: serde_json::Value = serde_json::from_str(&manifest).expect("valid json");
        assert_eq!(manifest["schema"], "gwvideo/v1");
        assert_eq!(manifest["fps"], 25);
        assert!(
            manifest["frames"].as_u64().expect("frames") >= 2,
            "the manifest counts the distinct frames"
        );
    }
}
