//! The video surface (m55): `app.video()` — the §10 presented-frame
//! recorder over the kit. Start the recorder on a pixel-capable adapter,
//! drive frames through the session pump, stop and read the finalized
//! `VideoSummary` (the artifact path, the encoded frames, the
//! dedup-dropped ones, the wall-clock span). Hash-equality dedup is the
//! §10 backpressure working: a static UI compresses to keyframe + no-ops,
//! the `dropped` count is the design working, never an anomaly. The
//! adapter needs the §6.2 pixel capability row — a hash-only adapter
//! answers the typed refusal.
//!
//! ```text
//! cargo run -p guiwright-kit --example record_video
//! # prints the start, each driven frame, and the finalized summary
//! # (frames, dropped, wall-clock span, artifact path) under the temp dir
//! ```

use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    // The pixel path the recorder needs (§6.2): scripted RGBA frames.
    let adapter =
        MockAdapter::with_fixture(&fixture).with_frame_pixels(16, 8, vec![120; 16 * 8 * 4]);
    let mut app = App::launch("record_video_demo", adapter);

    // The artifact dir is the caller's: a real suite points it at its
    // artifacts tree; the demo takes a unique temp subdir and cleans up.
    let dir = std::env::temp_dir().join(format!(
        "guiwright-record-video-demo-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("artifact dir");

    app.video().start_in(&dir, 25).expect("the recorder starts");

    // Drive the UI: two distinct frames, then a static span — the dedup
    // collapses the still span, the `dropped` count names it.
    app.session().adapter_mut().push_frame(0x11);
    app.session().pump();
    println!("frame   → hash 0x11 presented");
    app.session().adapter_mut().push_frame(0x22);
    app.session().pump();
    println!("frame   → hash 0x22 presented");
    app.session().adapter_mut().push_frame(0x22);
    app.session().pump();
    println!("frame   → hash 0x22 again (the static span)");

    let summary = app.video().stop().expect("the recorder finalizes");
    println!(
        "summary → {} frames encoded, {} dropped, {} ms — {}",
        summary.frames,
        summary.dropped,
        summary.duration_ms,
        summary.path.display()
    );
    assert!(summary.path.exists(), "the artifact was finalized");
    assert_eq!(
        summary.dropped, 1,
        "the static span collapsed by hash equality"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
