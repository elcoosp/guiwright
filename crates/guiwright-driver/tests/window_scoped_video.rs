//! Per-window recording scopes (the m6 §5/m7 §4 deferred half — "the
//! trace/video services record the session; window-tagged streams ride
//! the same handle vocabulary in a follow-up"): the session's video
//! service pins a window handle at start and crops every presented frame
//! to that window's subtree bounds, so a scoped recording is that
//! window's stream — a change outside the scope sheds its frames by the
//! §10 dedup instead of entering the artifact. Unscoped starts keep the
//! byte-compat whole-frame shape (the stamp-hash dedup), `main` is the
//! whole tree, and an unknown handle is the typed `UnknownWindow` —
//! validated before the recorder exists, never mid-stream.
//!
//! The adapter scripts pixel steps (each `last_frame` render advances
//! one), so the two windows' rects change independently and the crop
//! digests prove the scoping at the recorder boundary.

use guiwright_core::CoreError;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::Session;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{GestureV1, PointerButtonV1, VideoOptsV1, VideoV1};

/// The bench-shaped canvas: big enough for both fixture windows' rects,
/// which stay disjoint (W1's Save at 16,16→136,48; W2's Save + label at
/// 12,416→212,480 — the fixture's scripted bounds).
const CANVAS: (u32, u32) = (256, 512);

/// Fill one RGBA8 rect with a constant byte.
fn fill_rect(rgba: &mut [u8], width: u32, rect: (f64, f64, f64, f64), value: u8) {
    let (x0, y0, x1, y1) = rect;
    for y in y0 as u32..y1 as u32 {
        for x in x0 as u32..x1 as u32 {
            let idx = ((y * width + x) * 4) as usize;
            rgba[idx] = value;
        }
    }
}

/// The scripted animation: base, W1-only change, W2-only change, a second
/// W1-only change — each `last_frame` render advances one step, the last
/// sticks (the mock's contract), so a recording's crop digests move
/// exactly when the scoped window's pixels do.
fn scripted_steps() -> Vec<Vec<u8>> {
    let (w, h) = CANVAS;
    let base = vec![10u8; (w * h * 4) as usize];
    let mut w1_change = base.clone();
    fill_rect(&mut w1_change, w, (16.0, 16.0, 136.0, 48.0), 90);
    let mut w2_change = w1_change.clone();
    fill_rect(&mut w2_change, w, (12.0, 416.0, 212.0, 480.0), 90);
    let mut w1_again = w2_change.clone();
    fill_rect(&mut w1_again, w, (16.0, 16.0, 136.0, 48.0), 140);
    // The mock's render advances the sequence BEFORE returning, so the
    // first rendered frame is steps[1] — the base frame is listed twice
    // so the stream's first crop is the unchanged canvas.
    vec![base.clone(), base, w1_change, w2_change, w1_again]
}

fn two_window_session() -> (Session<MockAdapter>, MultiWindowFixture, tempfile::TempDir) {
    let fx = MultiWindowFixture::default();
    let adapter = MockAdapter::with_multi_window(&fx).with_frame_pixel_steps(
        CANVAS.0,
        CANVAS.1,
        scripted_steps(),
    );
    let mut session = Session::new(adapter);
    // Attach: the initial tree updates ride pumps — the window targets
    // the scope validates against do not exist before them. Settling
    // never touches pixels (the video capture calls `last_frame` only
    // while a recording runs), so the scripted steps stay fresh.
    session.settle(std::time::Instant::now() + std::time::Duration::from_secs(2));
    let dir = tempfile::tempdir().expect("tmpdir");
    (session, fx, dir)
}

/// The W1-scoped stream: three scripted pumps + the stop pump see the
/// base frame, a W1-only change (new crop → pushed), a W2-only change
/// (the W1 crop is unchanged → shed by the crop-digest dedup) and a
/// second W1 change (pushed). Three survivors, one honest drop.
#[test]
fn window_scoped_video_records_only_its_window() {
    let (mut session, fx, dir) = two_window_session();
    let w1 = fx.w1.0.to_string();

    session
        .video_start_in_window(dir.path(), 25, &w1)
        .expect("scoped start");
    session.pump();
    session.pump();
    session.pump();
    session.pump();
    let summary = session.video_stop().expect("stop");

    assert_eq!(
        summary.frames, 3,
        "base + the two W1 changes (the stop pump carries the last one)"
    );
    assert_eq!(
        summary.dropped, 2,
        "the base repeat + the W2-only change shed"
    );
    assert!(
        summary.path.exists(),
        "finalized at {}",
        summary.path.display()
    );
}

/// The same scripted stream scoped to W2 answers differently — the scope,
/// not the script, decides what the stream keeps.
#[test]
fn the_other_window_scopes_to_its_own_stream() {
    let (mut session, fx, dir) = two_window_session();
    let w2 = fx.w2.0.to_string();

    session
        .video_start_in_window(dir.path(), 25, &w2)
        .expect("scoped start");
    session.pump();
    session.pump();
    session.pump();
    session.pump();
    let summary = session.video_stop().expect("stop");

    assert_eq!(
        summary.frames, 2,
        "base + the one W2 change; the W1-only changes shed"
    );
    assert_eq!(
        summary.dropped, 3,
        "the base repeat + both W1-only changes shed by crop dedup"
    );
}

/// An unknown handle is a typed error before any recorder exists — the
/// scope is validated at start, never mid-stream.
#[test]
fn an_unknown_window_handle_is_a_typed_error() {
    let (mut session, _fx, dir) = two_window_session();
    let err = session
        .video_start_in_window(dir.path(), 25, "00000000-dead-beef-0000-000000000000")
        .expect_err("dead handle");
    match err {
        CoreError::UnknownWindow { handle } => {
            assert!(handle.contains("dead-beef"), "{handle}");
        }
        other => panic!("expected UnknownWindow, got {other:?}"),
    }
}

/// `main` is the whole tree (§16 v0.4 — the target a single-window
/// adapter reports): the recording keeps the byte-compat whole-frame
/// shape — stamp-hash dedup, no crop.
#[test]
fn the_main_target_keeps_the_whole_frame_shape() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    // Static pixels: the whole-frame dedup rides the stamp hash, so the
    // scripted hashes (not the pixel content) decide what survives — the
    // kit video test's exact script.
    let mut session = Session::new(MockAdapter::with_fixture(&fixture).with_frame_pixels(
        24,
        12,
        vec![64; 24 * 12 * 4],
    ));
    session.settle(std::time::Instant::now() + std::time::Duration::from_secs(2));
    let dir = tempfile::tempdir().expect("tmpdir");
    session
        .video_start_in_window(dir.path(), 25, "main")
        .expect("main scope = the whole tree");
    session.adapter_mut().push_frame(0x11);
    session.pump();
    session.adapter_mut().push_frame(0x11);
    session.pump();
    session.adapter_mut().push_frame(0x22);
    session.pump();
    let summary = session.video_stop().expect("stop");
    assert_eq!(summary.frames, 3, "0x11 + 0x22 + the stop-pump frame");
    assert_eq!(summary.dropped, 1, "the static span collapsed");
}

/// Over the wire (the m6 handoff): the connection's active window target
/// scopes the recording — no new wire shape, the same ride every
/// scope-narrowed read takes. The finalized artifact's encoded
/// dimensions are the window crop's, and an unscoped connection keeps
/// the full canvas (byte-compat behavior). Dimensions are read with
/// ffprobe when it is on PATH (the m77 dual-behaved pattern: the
/// counters carry the semantics regardless).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_connection_scope_scopes_the_wire_recording() {
    let fx = MultiWindowFixture::default();
    let scripted = fx.clone();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || {
            MockAdapter::with_multi_window(&scripted).with_frame_pixel_steps(
                CANVAS.0,
                CANVAS.1,
                scripted_steps(),
            )
        },
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = DuplexClient::connect(&connect).await.expect("client");
    client.handshake("wire-scoped-video/0.1").await.expect("hs");
    let w1 = fx.w1.0.to_string();
    client.window_use(&w1).await.expect("scope pins");

    let dir = tempfile::tempdir().expect("tmpdir");
    client
        .video(VideoV1::Start(guiwright_proto::schema::VideoOptsV1 {
            fps: 25,
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("scoped start acks");

    // Six gestures: each synthesizes and injects a real pointer path, so
    // the session pumps ≥ once per call — the scripted steps advance
    // through base → W1 → W2 → W1 and the crop dedup sheds the spans
    // where W1's crop is unchanged.
    for _ in 0..6 {
        let _ = client
            .gesture(GestureV1::Click {
                x: 60.0,
                y: 24.0,
                button: PointerButtonV1::Primary,
                count: 1,
            })
            .await
            .expect("the click pumps the scoped session");
    }

    let reply = client.video(VideoV1::Stop).await.expect("stop");
    let guiwright_proto::client::VideoReply::File(file) = reply else {
        panic!("expected the finalized file, got {reply:?}");
    };
    assert_eq!(
        file.frames, 3,
        "the W1 crop moved twice across the scripted stream"
    );
    assert!(
        file.dropped >= 1,
        "the W2-only span (and the static tail) shed: {}",
        file.dropped
    );

    if let Some((w, h)) = probe_dims(std::path::Path::new(&file.path)) {
        assert_eq!(
            (w, h),
            (120, 32),
            "the encoded stream is the Editor window's crop, not the canvas"
        );
    }

    handle.shutdown();
}

/// An unscoped connection keeps the whole-canvas recording — the
/// pre-scope byte-compat behavior, unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unscoped_connection_keeps_the_full_canvas() {
    let fx = MultiWindowFixture::default();
    let scripted = fx.clone();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || {
            MockAdapter::with_multi_window(&scripted).with_frame_pixel_steps(
                CANVAS.0,
                CANVAS.1,
                scripted_steps(),
            )
        },
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = DuplexClient::connect(&connect).await.expect("client");
    client
        .handshake("wire-unscoped-video/0.1")
        .await
        .expect("hs");
    let dir = tempfile::tempdir().expect("tmpdir");
    client
        .video(VideoV1::Start(guiwright_proto::schema::VideoOptsV1 {
            fps: 25,
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("start acks");
    for _ in 0..4 {
        let _ = client
            .gesture(GestureV1::Click {
                x: 60.0,
                y: 24.0,
                button: PointerButtonV1::Primary,
                count: 1,
            })
            .await
            .expect("click pumps");
    }
    let reply = client.video(VideoV1::Stop).await.expect("stop");
    let guiwright_proto::client::VideoReply::File(file) = reply else {
        panic!("expected the finalized file, got {reply:?}");
    };
    if let Some((w, h)) = probe_dims(std::path::Path::new(&file.path)) {
        assert_eq!(
            (w, h),
            (CANVAS.0, CANVAS.1),
            "unscoped recordings keep the full canvas"
        );
    }
    handle.shutdown();
}

/// Encoded stream dimensions via ffprobe (available on dev hosts and CI
/// image runners); `None` when the binary is missing — the counters
/// above carry the assertion in that case.
fn probe_dims(path: &std::path::Path) -> Option<(u32, u32)> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let (w, h) = text.split_once(',')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}
