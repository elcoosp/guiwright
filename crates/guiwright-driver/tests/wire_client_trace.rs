//! The wire Trace domain over the client seam (§7 "Domain: trace &
//! inspector", `Trace { Start(opts) | Stop | Attach(stream) }`): the
//! **Start/Stop** arms ride a new `RequestV1::Trace` domain answered by a
//! driver-side `gwtrace/v1` recorder — the §12 archive, recorded on the
//! session side (the kit's embedded `enable_trace` path, now over any
//! door). The **Attach** arm shipped with M18 (the live feed over the §7
//! event channel); this milestone pays the control half the §7 sketch
//! named and no milestone had delivered:
//!
//! ```text
//! Trace { Start(opts) | Stop | Attach(stream) }        (docs/architecture.md §7)
//! ```
//!
//! The seam mirrors the m55 video row: one call taking the wire's own
//! [`TraceV1`] enum and answering the [`TraceReply`] union — the start
//! arm acks `Started` (the wire `Pong`), the stop arm carries the
//! finalized archive (path + the §12 stats the recorder reports). The
//! client owns the artifact dir (Playwright parity, the video rule); the
//! connection's window scope tags the archive (the m6 §5 handle
//! vocabulary the video scope rides).

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::TraceReply;
use guiwright_proto::schema::{ErrorCode, SelectorV1, TextMatchV1, TraceOptsV1, TraceV1};

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-trace/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn_pixels() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        // The script pixels give the adapter the §6.2 pixel row the frame
        // feed rides — the recorder's `frames/<seq>.png` rows are real
        // rasters, hash-deduped by the writer (§12).
        || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
                24,
                12,
                vec![64; 24 * 12 * 4],
            )
        },
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts")
}

fn spawn_bare() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        // No frame pixels: the §6.2 pixel rows stay false, so the archive
        // records the session's events and tree diffs honestly (frames 0)
        // — never a guessed raster.
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts")
}

fn save_selector() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: "Save".into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

/// Start → drive → stop: the start arm's ack is the [`TraceReply::Started`]
/// union arm (the wire `Pong`), the stop arm carries the finalized
/// [`guiwright_proto::client::TraceOutput`] — the `guiwright-<test>-<ts>.zip`
/// archive path (§12) under the client-owned dir, the protocol calls the
/// drive made recorded as `events.ndjson` rows, the presented frame
/// recorded in the hash-deduped `frames/` set, and a file that really
/// exists at that path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_acks_and_stop_carries_the_finalized_archive() {
    let (handle, connect) = spawn_pixels();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    let started = client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "wire_client_trace".into(),
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the start acks");
    assert_eq!(started, TraceReply::Started, "start answers the ack arm");

    // One resolve pumps the adapter once: the tree update feeds the
    // recorder's diff rows, the presented frame feeds the PNG feed (the
    // same drive `wire_client_video.rs` uses at the video seam).
    let _ = client
        .resolve(save_selector(), 2000)
        .await
        .expect("resolve pumps a frame");

    let stopped = client.trace(TraceV1::Stop).await.expect("the stop");
    let file = match stopped {
        TraceReply::File(file) => file,
        other @ TraceReply::Started => panic!("expected the File arm, got {other:?}"),
    };
    assert!(
        file.events >= 2,
        "the protocol calls the drive made are recorded (the resolve and \
         the stop's own dispatch): {}",
        file.events
    );
    assert!(
        file.frames >= 1,
        "at least the first presented frame recorded"
    );
    assert_eq!(
        file.records_dropped, 0,
        "a two-request session never drops records"
    );
    assert!(
        file.archive_bytes > 0,
        "the finalized archive carries bytes"
    );
    assert!(
        std::path::Path::new(&file.path).starts_with(dir.path()),
        "the archive lives in the client-owned dir: {}",
        file.path
    );
    assert!(
        std::path::Path::new(&file.path).exists(),
        "archive exists at {}",
        file.path
    );

    handle.shutdown();
}

/// A pixel-less adapter (no §6.2 pixel row) records the session honestly:
/// the events and tree diffs land, the frame feed contributes nothing —
/// the archive is never a guessed raster (the §10 one-render rule's
/// trace-side shape).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pixelless_adapter_records_zero_frames() {
    let (handle, connect) = spawn_bare();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "pixelless".into(),
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the start acks");
    let _ = client
        .resolve(save_selector(), 2000)
        .await
        .expect("resolve pumps a frame");

    let stopped = client.trace(TraceV1::Stop).await.expect("the stop");
    let file = match stopped {
        TraceReply::File(file) => file,
        other @ TraceReply::Started => panic!("expected the File arm, got {other:?}"),
    };
    assert!(file.events >= 1, "the protocol calls are recorded");
    assert_eq!(file.frames, 0, "no pixel row, no frames — never guessed");

    handle.shutdown();
}

/// The session's single-slot rule, surfaced (not re-mapped): starting a
/// second archive while one records answers the wire's typed rejection —
/// the same honesty the video seam's double-start meets.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_start_answers_the_typed_rejection() {
    let (handle, connect) = spawn_pixels();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "first".into(),
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the first start");

    let err = client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "second".into(),
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect_err("double start is a typed rejection");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::Internal);
            assert!(
                message.contains("already running"),
                "the message names the cause: {message}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    handle.shutdown();
}

/// Stopping without an archive is the wire's clean typed rejection
/// ("idempotence via error", the video session rule) — the sugar surfaces
/// it verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_without_an_archive_answers_the_typed_rejection() {
    let (handle, connect) = spawn_bare();
    let mut client = connected(&connect).await;

    let err = client
        .trace(TraceV1::Stop)
        .await
        .expect_err("no archive recording");
    match err {
        GwpError::Rejected { message, .. } => {
            assert!(
                message.contains("no trace recording"),
                "the message names the cause: {message}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    handle.shutdown();
}
