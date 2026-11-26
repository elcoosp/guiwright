//! The §12 archive's `logs.ndjson` row: while an archive records, the
//! session thread's `tracing` output lands in the zip. The plan row the
//! architecture doc's archive schema has carried since v0.1:
//!
//! ```text
//! logs.ndjson   `tracing` subscriber output (frameworks already use tracing)
//! ```
//!
//! (docs/architecture.md §12, the archive block). The m97 record's honest
//! list names the standing gap verbatim: *"The archive's `logs.ndjson` row
//! stays empty: no surface tees the session's `tracing` tap into the
//! recorder (the kit's archives don't either) — the live `EventV1::Log`
//! door (M14) remains the log path."* The recorder's `logs.ndjson` writer
//! and the `TraceLogLayer` have shipped in `guiwright-trace` since the
//! first archive cut — no session surface ever installed the layer. These
//! tests pin the tee at the driver: an adapter emitting on the session
//! thread's pump (the §12 claim — "frameworks already use tracing")
//! records into the archive while it records, the M14 live tail keeps
//! flowing beside it, and the tee leaves with the stop (a second archive
//! re-teed from empty, no stacked layers).

use std::io::Read as _;

use accesskit::{NodeId, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, FrameDeadline, FrameworkAdapter, PumpOutcome,
    SemAction,
};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::{GwpServerHandle, ServerOptions};
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{
    EventKindV1, EventV1, GwpEvent, MockV1, RequestV1, ResponseV1, SelectorV1, SubscribeV1,
    TextMatchV1, TraceOptsV1, TraceV1,
};

/// The §12 claim as a fixture: a Level-1 adapter whose framework emits
/// `tracing` records while it pumps ("frameworks already use tracing" —
/// the archive's logs row is the framework + driver diagnostics channel).
/// The tree rides a real `MockAdapter` fixture so the drive resolves.
struct LoggingAdapter(MockAdapter);

impl LoggingAdapter {
    fn new() -> Self {
        Self(MockAdapter::with_fixture(&Fixture::buttons()))
    }
}

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

fn spawn_logging() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(LoggingAdapter::new, guiwright_driver::SeamRegistry::new())
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

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-trace-logs/0.1")
        .await
        .expect("handshake");
    client
}

/// Read one finalized archive's `logs.ndjson` back as JSON lines.
fn archive_log_lines(path: &str) -> Vec<serde_json::Value> {
    let bytes = std::fs::read(path).expect("archive exists at the TraceFile path");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("gwtrace/v1 zip");
    let mut logs = String::new();
    zip.by_name("logs.ndjson")
        .expect("logs.ndjson row in the archive")
        .read_to_string(&mut logs)
        .expect("utf8");
    logs.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("one JSON line per record"))
        .collect()
}

/// The plan row, pinned: the archive records while the session records, so
/// the `tracing` records the session thread emits — here the adapter's own
/// framework diagnostics on the pump path — land in `logs.ndjson` with the
/// §12 vocabulary (`ts_ns`, `level`, `target`, `pid`, `message`, plus the
/// event's structured fields). Before the tee this archive's logs row was
/// empty on every surface (the m97 record's honest list).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_archive_carries_the_session_thread_tracing_output() {
    let (handle, connect) = spawn_logging();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "wire_trace_logs".into(),
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the start acks");

    // One resolve pumps the adapter once — the pump emits the framework's
    // `tracing` record on the session thread, the tee's subscriber scope.
    let _ = client
        .resolve(save_selector(), 2000)
        .await
        .expect("resolve pumps a frame");

    let stopped = client.trace(TraceV1::Stop).await.expect("the stop");
    let file = match stopped {
        guiwright_proto::client::TraceReply::File(file) => file,
        other @ guiwright_proto::client::TraceReply::Started => {
            panic!("expected the File arm, got {other:?}")
        }
    };

    let lines = archive_log_lines(&file.path);
    assert!(
        lines.iter().any(|line| {
            line["message"] == "the framework's own record" && line["level"] == "WARN"
        }),
        "the session thread's tracing record rides the archive's logs.ndjson \
         (§12 `tracing subscriber output`): {lines:?}"
    );
    let record = lines
        .iter()
        .find(|line| line["message"] == "the framework's own record")
        .expect("the record checked above");
    assert_eq!(record["target"], "wire_trace_logs", "the emitting target");
    assert_eq!(
        record["window"], "main",
        "the event's structured fields ride the line (the §12 vocabulary)"
    );
    assert_eq!(
        record["pid"],
        serde_json::json!(std::process::id()),
        "the pid field rides the line"
    );

    handle.shutdown();
}

/// The tee composes with the M14 live tail (the m97 record's other half —
/// "the live `EventV1::Log` door remains the log path"): with the tail on,
/// the session thread's record reaches the wire subscriber AND the archive —
/// two layers on one registry, the tail's flow intact while an archive
/// records.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tee_rides_beside_the_live_log_tap() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess_with(
        LoggingAdapter::new,
        ServerOptions {
            log_tail: true,
            ..ServerOptions::default()
        },
    )
    .expect("in-process server starts");
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "tee_beside_tap".into(),
            dir: Some(dir.path().display().to_string()),
        }))
        .await
        .expect("the start acks");

    // Subscribing Log while the archive records: the subscription
    // lifecycle's own record emits on the session thread — the tail's
    // m14 flow must keep flowing beside the tee.
    let ack = client
        .request(RequestV1::Subscribe(SubscribeV1::On {
            kinds: vec![EventKindV1::Log],
        }))
        .await
        .expect("the tail serves the Log kind");
    assert!(
        matches!(ack, ResponseV1::Subscribed { .. }),
        "the Log kind is served, not Unsupported: {ack:?}"
    );
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), client.next_event())
        .await
        .expect("the session's record arrives")
        .expect("event frame");
    let wire_record = match first {
        GwpEvent::V1(EventV1::Log { record }) => {
            let parsed: serde_json::Value = serde_json::from_str(&record).expect("one JSON line");
            assert_eq!(
                parsed["message"], "event subscription registered",
                "the m14 lifecycle record on the wire"
            );
            parsed
        }
        GwpEvent::V1(other) => panic!("expected the wire log record, got {other:?}"),
    };

    let _ = client
        .resolve(save_selector(), 2000)
        .await
        .expect("resolve pumps a frame");

    let stopped = client.trace(TraceV1::Stop).await.expect("the stop");
    let file = match stopped {
        guiwright_proto::client::TraceReply::File(file) => file,
        other @ guiwright_proto::client::TraceReply::Started => {
            panic!("expected the File arm, got {other:?}")
        }
    };

    let lines = archive_log_lines(&file.path);
    assert!(
        lines
            .iter()
            .any(|line| line["message"] == "event subscription registered"
                && line["level"] == "DEBUG"),
        "the session's own record lands in the archive beside the tail: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line["message"] == "the framework's own record"),
        "the adapter's record lands too"
    );
    // The wire record and the archive line are the same §12 vocabulary —
    // one event, two consumers (the m14 tail + the §12 tee).
    assert_eq!(
        wire_record["message"], "event subscription registered",
        "the wire record checked above"
    );

    handle.shutdown();
}

/// The tee leaves with the stop: a second archive re-teed from empty, no
/// stacked layers (one stop → one restore, the guard-scope rule). The
/// emission is seam-driven — one client `mock call` = exactly one
/// `tracing` record on the session thread — so each archive carries
/// exactly one copy; a leaked first-archive layer would double the
/// second archive's count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tee_restores_after_stop() {
    let mut seams = guiwright_driver::SeamRegistry::new();
    seams.register("emit_record", |_ctx, _scope| {
        tracing::warn!(window = "seam", "the framework's own record");
        Ok("emitted".into())
    });
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        // The runner context the call machinery demands (the §14 state
        // lives in the adapter; this seam ignores it, it only emits).
        || MockAdapter::with_fixture(&Fixture::buttons()).with_state(()),
        seams,
    )
    .expect("in-process server starts");
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    for test in ["first", "second"] {
        client
            .trace(TraceV1::Start(TraceOptsV1 {
                test: test.into(),
                dir: Some(dir.path().display().to_string()),
            }))
            .await
            .expect("the start acks");
        client
            .mock(MockV1::Call {
                id: "emit_record".into(),
                scope: "tee".into(),
            })
            .await
            .expect("the seam emits once on the session thread");
        let stopped = client.trace(TraceV1::Stop).await.expect("the stop");
        let file = match stopped {
            guiwright_proto::client::TraceReply::File(file) => file,
            other @ guiwright_proto::client::TraceReply::Started => {
                panic!("expected the File arm, got {other:?}")
            }
        };
        let copies = archive_log_lines(&file.path)
            .iter()
            .filter(|line| line["message"] == "the framework's own record")
            .count();
        assert_eq!(
            copies, 1,
            "one seam call, one line — no stacked tee layers in the {test} archive"
        );
    }

    handle.shutdown();
}

/// Sanity pin borrowed from the m97 suite: a pixel-less logging adapter
/// still records frames 0 honestly (the tee changes the logs row, never
/// the frame honesty rule).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pixelless_logging_adapter_still_records_zero_frames() {
    let (handle, connect) = spawn_logging();
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut client = connected(&connect).await;

    client
        .trace(TraceV1::Start(TraceOptsV1 {
            test: "pixelless-logging".into(),
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
        guiwright_proto::client::TraceReply::File(file) => file,
        other @ guiwright_proto::client::TraceReply::Started => {
            panic!("expected the File arm, got {other:?}")
        }
    };
    assert_eq!(file.frames, 0, "no pixel row, no frames — never guessed");
    assert!(
        std::path::Path::new(&file.path).exists(),
        "archive exists at {}",
        file.path
    );

    handle.shutdown();
}
