//! The video row (m55): [`ClientConn::video`] carries the §10 recording
//! domain as typed sugar — one call taking the wire's own [`VideoV1`] enum
//! (start / stop) and answering the [`VideoReply`] union (the start ack,
//! the finalized [`VideoOutput`]), the same shape the TypeScript client has
//! spoken since M3 (`videoStart()` / `videoStop()`) and the REPL since M3
//! (`video start` / `video stop`). No wire change: the command rides the
//! `Video` domain byte-for-byte as M3 blessed it; every [`ClientConn`] impl
//! inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_video
//! # prints the start ack landing, the stop's finalized artifact summary
//! # (path, encoded frames, dedup-dropped frames, wall-clock span), the
//! # typed rejection for a second concurrent start, and the typed
//! # rejection for a stop with nothing running
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::VideoReply;
use guiwright_proto::schema::{SelectorV1, TextMatchV1, VideoOptsV1, VideoV1};

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
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
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-client-video-demo/0.1")
            .await
            .expect("handshake");

        // The client owns the artifact dir (Playwright-style): the reply's
        // path is how the client learns where the file landed.
        let dir = std::env::temp_dir().join(format!(
            "guiwright-video-demo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).expect("artifact dir");

        // Start: the ack arm (the wire `Pong`) — the recorder is live.
        let started = client
            .video(VideoV1::Start(VideoOptsV1 {
                fps: 25,
                dir: Some(dir.display().to_string()),
            }))
            .await
            .expect("start");
        assert_eq!(started, VideoReply::Started);

        // The honesty, first half: a second start while a recording runs
        // answers the session's typed rejection (single-slot, the M3 rule)
        // — surfaced, never re-mapped.
        let err = client
            .video(VideoV1::Start(VideoOptsV1 {
                fps: 25,
                dir: Some(dir.display().to_string()),
            }))
            .await
            .expect_err("a recording is already running");
        println!("start?  → {err}");

        // Drive the session: each request pumps the adapter once and the
        // presented frames feed the recorder (the static span dedups).
        let _ = client
            .resolve(
                SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                2000,
            )
            .await
            .expect("resolve pumps a frame");

        // Stop: the finalized artifact rides the reply (§10 — the path,
        // the encoded frames, the dedup-dropped ones, the wall-clock span).
        let stopped = client.video(VideoV1::Stop).await.expect("stop");
        match stopped {
            VideoReply::File(file) => {
                println!(
                    "stop    → {} ({} frames, {} dropped, {}ms)",
                    file.path, file.frames, file.dropped, file.duration_ms
                );
                println!("exists  → {}", std::path::Path::new(&file.path).exists());
            }
            other => panic!("expected the File arm, got {other:?}"),
        }

        // The honesty, second half: a stop with nothing running answers
        // the same typed rejection ("idempotence via error", the M3 rule).
        let err = client
            .video(VideoV1::Stop)
            .await
            .expect_err("nothing running");
        println!("stop?   → {err}");

        let _ = std::fs::remove_dir_all(&dir);
        handle.shutdown();
    });
}
