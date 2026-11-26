//! The §12 archive's `logs.ndjson` row at the kit surface: the embedded
//! `enable_trace` path arms the same tee the driver session arms — while
//! the archive records, the test thread's `tracing` output lands in the
//! zip. The m97 record's honest list named the kit half verbatim (*"the
//! kit's archives don't either"*); the plan row is the architecture doc's
//! archive schema (§12):
//!
//! ```text
//! logs.ndjson   `tracing` subscriber output (frameworks already use tracing)
//! ```
//!
//! The embedded drive runs the adapter and the kit on the test thread
//! (§5.1), so the adapter's pump emissions land in the archive the same
//! way a real framework's `tracing` records would.

use std::io::Read;

use accesskit::{NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, FrameDeadline, FrameworkAdapter, PumpOutcome,
    SemAction,
};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

/// The §12 claim as a fixture: a Level-1 adapter whose framework emits
/// `tracing` records while it pumps, over a real `MockAdapter` fixture
/// tree (the drive resolves).
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

/// The embedded trace carries the test thread's `tracing` output: the
/// click's pumps emit, `finish` finalizes, and the archive's `logs.ndjson`
/// parses with the §12 vocabulary — before the tee this row was empty on
/// every surface.
#[test]
fn the_embedded_archive_carries_the_test_thread_tracing_output() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = App::launch("kit_trace_logs", LoggingAdapter::new());
    app.enable_trace(tmp.path().join("trace.zip"));
    app.get_by_role(Role::Button, "Save")
        .click()
        .expect("click drives a pump");
    let path = app.finish().expect("trace path");

    let bytes = std::fs::read(&path).expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut logs = String::new();
    archive
        .by_name("logs.ndjson")
        .expect("logs.ndjson row in the archive")
        .read_to_string(&mut logs)
        .expect("read");
    let lines: Vec<serde_json::Value> = logs
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("one JSON line per record"))
        .collect();
    let record = lines
        .iter()
        .find(|line| line["message"] == "the framework's own record")
        .expect("the pump emission landed in the archive: {lines:?}");
    assert_eq!(record["level"], "WARN");
    assert_eq!(record["window"], "main");
    assert_eq!(
        record["pid"],
        serde_json::json!(std::process::id()),
        "the pid field rides the line"
    );
}
