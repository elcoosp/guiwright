//! The §12 `logs.ndjson` tee's consumer demo: one conversation over the
//! in-process door where the archive records the session thread's
//! `tracing` output beside its tree diffs, frames and protocol-call rows.
//! The adapter's pump emits a framework-shaped record ("frameworks
//! already use tracing" — the plan row the archive schema carries), the
//! stop arm finalizes, and the demo reads the `logs.ndjson` row back.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_trace_logs
//! # prints the start ack, the drive inside the recording window, the
//! # finalized archive's stats and the log rows the tee captured
//! ```

use std::io::Read as _;

use accesskit::{NodeId, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, FrameDeadline, FrameworkAdapter, PumpOutcome,
    SemAction,
};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::TraceReply;
use guiwright_proto::schema::{SelectorV1, TextMatchV1, TraceOptsV1, TraceV1};

/// The demo adapter: a real fixture tree plus the framework's own
/// diagnostics channel — one `tracing` record per pump, the shape a real
/// framework emits and the archive's logs row exists to carry.
struct LoggingAdapter(MockAdapter);

impl FrameworkAdapter for LoggingAdapter {
    fn meta(&self) -> &AdapterMeta {
        self.0.meta()
    }

    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome {
        tracing::warn!(window = "main", "the framework's own record");
        self.0.pump(deadline)
    }

    fn tree_update(&mut self) -> Option<TreeUpdate> {
        self.0.tree_update()
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        self.0.act(node, action)
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || LoggingAdapter(MockAdapter::with_fixture(&Fixture::buttons())),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-trace-logs-demo/0.1")
            .await
            .expect("handshake");

        let dir = tempfile::tempdir().expect("demo artifact dir");
        let started = client
            .trace(TraceV1::Start(TraceOptsV1 {
                test: "wire_trace_logs_demo".into(),
                dir: Some(dir.path().display().to_string()),
            }))
            .await
            .expect("the start acks");
        assert!(matches!(started, TraceReply::Started));
        println!("start  → archive recording (dir {})", dir.path().display());

        // Drive one resolve: the pumps emit the framework's `tracing`
        // record on the session thread — the tee's subscriber scope.
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
        let file = match stopped {
            TraceReply::File(file) => file,
            other => panic!("expected the File arm, got {other:?}"),
        };
        println!("stop   → archive at {}", file.path);
        println!(
            "         {} event rows, {} frames, {} records dropped, {} bytes",
            file.events, file.frames, file.records_dropped, file.archive_bytes
        );

        // Read the logs row back: the session thread's `tracing` output,
        // one JSON line per record (the §12 vocabulary).
        let bytes = std::fs::read(&file.path).expect("archive exists");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("gwtrace/v1 zip");
        let mut logs = String::new();
        zip.by_name("logs.ndjson")
            .expect("logs.ndjson row")
            .read_to_string(&mut logs)
            .expect("utf8");
        println!("         logs.ndjson (the §12 tee):");
        for line in logs.lines().filter(|line| !line.is_empty()) {
            println!("           {line}");
        }

        handle.shutdown();
    });
}
