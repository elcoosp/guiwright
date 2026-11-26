//! The wire Trace domain's consumer demo (M97, §7 "Domain: trace &
//! inspector"): one conversation over the in-process door —
//! [`ClientConn::trace`] starts the session-side `gwtrace/v1` archive
//! (§12), the drive's requests land as the archive's event rows, and the
//! stop arm answers the finalized archive (path + the recorder stats).
//! The Attach arm stays the M18 live feed over the event channel,
//! unchanged here.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_trace
//! # prints the start ack, the drive inside the recording window and the
//! # finalized archive's path + stats
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::TraceReply;
use guiwright_proto::schema::{SelectorV1, TextMatchV1, TraceOptsV1, TraceV1};

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            // The script pixels give the adapter the §6.2 pixel row the
            // frame feed rides — the archive's `frames/<seq>.png` rows
            // are real rasters, hash-deduped by the writer (§12).
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
            .handshake("wire-client-trace-demo/0.1")
            .await
            .expect("handshake");

        // Start the archive: the client owns the artifact dir (Playwright
        // parity, the video rule), the test label rides meta.json (§12).
        let dir = tempfile::tempdir().expect("demo artifact dir");
        let started = client
            .trace(TraceV1::Start(TraceOptsV1 {
                test: "wire_client_trace_demo".into(),
                dir: Some(dir.path().display().to_string()),
            }))
            .await
            .expect("the start acks");
        assert!(matches!(started, TraceReply::Started));
        println!("start  → archive recording (dir {})", dir.path().display());

        // Drive one resolve: the protocol call lands as the archive's
        // event row, the presented frame feeds the PNG set.
        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: false,
                ignore_case: true,
            }),
        };
        let _ = client.resolve(save, 2000).await.expect("resolve");
        println!("resolve → recorded inside the archive window");

        // Stop and finalize: the reply carries the path and the §12 stats.
        let stopped = client.trace(TraceV1::Stop).await.expect("the stop");
        match stopped {
            TraceReply::File(file) => {
                println!("stop   → archive at {}", file.path);
                println!(
                    "         {} event rows, {} frames, {} records dropped, {} bytes",
                    file.events, file.frames, file.records_dropped, file.archive_bytes
                );
                assert!(file.events >= 2, "the protocol calls are recorded");
                assert!(file.frames >= 1, "the presented frame is recorded");
            }
            other => panic!("expected the File arm, got {other:?}"),
        }

        handle.shutdown();
    });
}
