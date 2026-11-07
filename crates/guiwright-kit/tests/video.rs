//! Kit M3 tests (§10): the `app.video()` surface — records presented
//! frames between start and stop, dedups static spans, finalizes a real
//! artifact (ffmpeg `.webm` when the binary is available, PNG frame
//! folder otherwise), and lands `video` events in the trace archive.

use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn pixel_app(test: &str) -> App<MockAdapter> {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    // No pre-queued frames: the video tests script their own hashes, and
    // the recording window must start from a clean queue.
    let adapter =
        MockAdapter::with_fixture(&fixture).with_frame_pixels(16, 8, vec![120; 16 * 8 * 4]);
    App::launch(test, adapter)
}

#[test]
fn video_surface_records_and_finalizes() {
    let mut app = pixel_app("video_surface");
    let dir = tempfile::tempdir().expect("tmpdir");

    app.video().start_in(dir.path(), 25).expect("start");

    // Drive the UI: two distinct frames + one static span.
    app.session().adapter_mut().push_frame(0x11);
    app.session().pump();
    app.session().adapter_mut().push_frame(0x11);
    app.session().pump();
    app.session().adapter_mut().push_frame(0x22);
    app.session().pump();

    let summary = app.video().stop().expect("stop");
    assert_eq!(
        summary.frames, 3,
        "0x11 + 0x22 + the stop-pump frame (hash 0)"
    );
    assert_eq!(summary.dropped, 1, "the static span collapsed");
    assert!(
        summary.path.exists(),
        "finalized at {}",
        summary.path.display()
    );
}

#[test]
fn video_refuses_adapters_without_pixels() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut app = App::launch("video_no_pixels", MockAdapter::with_fixture(&fixture));
    let dir = tempfile::tempdir().expect("tmpdir");
    let err = app
        .video()
        .start_in(dir.path(), 25)
        .expect_err("hash-only adapter");
    assert!(err.to_string().contains("pixel"), "{err}");
}

#[test]
fn video_events_land_in_the_trace_archive() {
    let mut app = pixel_app("video_trace");
    let dir = tempfile::tempdir().expect("tmpdir");
    let trace = dir.path().join("trace.zip");
    app.enable_trace(&trace);

    app.video().start_in(dir.path(), 25).expect("start");
    app.session().adapter_mut().push_frame(0x33);
    app.session().pump();
    let summary = app.video().stop().expect("stop");

    let path = app.finish().expect("trace written");
    let zip = std::fs::File::open(&path).expect("archive");
    let mut zip = zip::ZipArchive::new(zip).expect("zip");
    let mut events = String::new();
    use std::io::Read as _;
    zip.by_name("events.ndjson")
        .expect("events stream")
        .read_to_string(&mut events)
        .expect("read");
    assert!(events.contains("\"video\""), "video events recorded");
    assert!(events.contains("start(dir="), "{events}");
    assert!(events.contains(&summary.frames.to_string()), "{events}");
}
