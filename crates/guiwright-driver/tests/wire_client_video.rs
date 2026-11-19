//! The client seam's video rows over the wire (m55): [`ClientConn::video`]
//! carries the §10 recording domain as typed seam sugar — one call taking
//! the wire's own [`VideoV1`] enum (start / stop) and answering the
//! [`VideoReply`] union (the start arm's ack, the stop arm's finalized
//! [`VideoOutput`]), the same shape the TypeScript client has spoken since
//! M3 (`videoStart()` / `videoStop()`) and the REPL since M3 (`video start`
//! / `video stop`). **No wire change**: the sugar rides the `Video` domain
//! byte-for-byte as M3 blessed it; every [`ClientConn`] impl inherits the
//! row. Driven here over the in-process door — the same reality
//! `remote_session.rs` pins at the raw-request level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::VideoReply;
use guiwright_proto::schema::{ErrorCode, SelectorV1, TextMatchV1, VideoOptsV1, VideoV1};

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-video/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        // The script pixels give the adapter the §6.2 pixel row the
        // recorder needs — uniform bytes hash-0, so the static span
        // records first frame + dedup no-ops (§10 backpressure).
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

/// Start → drive → stop: the start arm's ack is the [`VideoReply::Started`]
/// union arm (the wire `Pong`), the stop arm carries the finalized
/// [`VideoOutput`] — the artifact path the client owns the dir of
/// (Playwright-style), at least the first presented frame encoded, and a
/// file that really exists at that path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_acks_and_stop_carries_the_finalized_artifact() {
    let (handle, connect) = spawn();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    let started = client
        .video(VideoV1::Start(VideoOptsV1 {
            fps: 25,
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the start acks");
    assert_eq!(started, VideoReply::Started, "start answers the ack arm");

    // One resolve pumps the adapter once; the presented frame feeds the
    // recorder (the same drive `remote_session.rs` uses at the raw level).
    let _ = client
        .resolve(save_selector(), 2000)
        .await
        .expect("resolve pumps a frame");

    let stopped = client.video(VideoV1::Stop).await.expect("the stop");
    let file = match stopped {
        VideoReply::File(file) => file,
        other => panic!("expected the File arm, got {other:?}"),
    };
    assert!(
        file.frames >= 1,
        "at least the first presented frame recorded"
    );
    assert!(
        std::path::Path::new(&file.path).starts_with(dir.path()),
        "the artifact lives in the client-owned dir: {}",
        file.path
    );
    assert!(
        std::path::Path::new(&file.path).exists(),
        "artifact exists at {}",
        file.path
    );

    handle.shutdown();
}

/// The session's single-slot rule, surfaced (not re-mapped): starting a
/// second recording while one runs answers the wire's typed rejection —
/// the same honesty the REPL's `video start` row meets.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_start_answers_the_typed_rejection() {
    let (handle, connect) = spawn();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    client
        .video(VideoV1::Start(VideoOptsV1 {
            fps: 25,
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the first start");

    let err = client
        .video(VideoV1::Start(VideoOptsV1 {
            fps: 25,
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

/// Stopping without a recording is the wire's clean typed rejection
/// ("idempotence via error", the M3 session rule) — the sugar surfaces it
/// verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_without_a_recording_answers_the_typed_rejection() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let err = client
        .video(VideoV1::Stop)
        .await
        .expect_err("no recording running");
    match err {
        GwpError::Rejected { message, .. } => {
            assert!(
                message.contains("no video recording"),
                "the message names the cause: {message}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    handle.shutdown();
}
