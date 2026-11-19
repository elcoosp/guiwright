//! The client seam's tree-snapshot row over the wire (m57):
//! [`ClientConn::tree_snapshot`] carries the §7 summary read as typed seam
//! sugar — one call answering the `TreeSnapshotSummary` carrier (the tree
//! generation and the node count), the same shape the TypeScript client has
//! spoken since its M3 landing (`treeSnapshot()`). **No wire change**: the
//! sugar rides the `TreeSnapshot` domain byte-for-byte as M0 blessed it;
//! every [`ClientConn`] impl inherits the row. Driven here over the
//! in-process door — the same reality `remote_session.rs` pins at the
//! raw-request level and `wire_client_dump.rs` pins at the m49 dump-row
//! level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-tree-snapshot/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts")
}

/// The summary read lands as the carrier: the M0 fixture's 8 nodes and the
/// tree generation the attach pump applied (`tree_seq >= 1`, the seq space
/// the `TreeChanged` events report).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_summary_read_names_the_tree_and_its_generation() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let summary = client.tree_snapshot().await.expect("the summary read");
    assert_eq!(
        summary.node_count, 8,
        "the M0 fixture's 8 nodes, the count the raw arm already reported"
    );
    assert!(
        summary.tree_seq >= 1,
        "the attach pump applied the fixture update"
    );

    handle.shutdown();
}

/// Two reads back to back are stable and agree with the m49 dump row: the
/// summary is a read, not a mutation pump — the tree generation does not
/// move between reads, and the count matches the full `TreeDump` walk
/// (one summary row, one depth-first walk, the same tree).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_reads_are_stable_and_agree_with_the_dump_row() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let first = client.tree_snapshot().await.expect("the first read");
    let dump = client.tree_dump().await.expect("the m49 full dump");
    let second = client.tree_snapshot().await.expect("the second read");

    assert_eq!(first.tree_seq, dump.tree_seq, "the same generation");
    assert_eq!(
        first.node_count, dump.node_count,
        "the summary counts what the dump walks"
    );
    assert_eq!(
        second.tree_seq, first.tree_seq,
        "a read pumps, it never mutates: {first:?} then {second:?}"
    );
    assert_eq!(second.node_count, first.node_count);

    handle.shutdown();
}

/// The summary read is session-wide by design (the §16 v0.4 rule the dump
/// rows carry: "the connection's resolve scope narrows locators, not
/// whole-tree reads"): aiming the connection at a window still reports the
/// whole tree's 8 nodes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_scoped_connection_still_reads_the_whole_tree() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let windows = client.window_list().await.expect("the window list");
    let target = windows
        .first()
        .expect("the fixture attaches one window")
        .handle
        .clone();
    let aimed = client.window_use(target).await.expect("the window aim");
    let summary = client
        .tree_snapshot()
        .await
        .expect("the scoped summary read");

    assert_eq!(aimed, windows[0].handle, "the aim answers the aimed handle");
    assert_eq!(
        summary.node_count, 8,
        "the whole tree, scope or no scope: {summary:?}"
    );

    handle.shutdown();
}
