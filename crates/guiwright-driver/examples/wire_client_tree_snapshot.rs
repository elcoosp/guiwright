//! The tree-snapshot row (m57): [`ClientConn::tree_snapshot`] carries the
//! §7 summary read as typed sugar — one call answering the
//! `TreeSnapshotSummary` carrier (the tree generation and the whole tree's
//! node count), the same shape the TypeScript client has spoken since its
//! M3 landing (`treeSnapshot()`). Session-wide by design — the connection's
//! window scope narrows locators, not whole-tree reads — and a read, not a
//! mutation pump: back-to-back reads agree with each other and with the
//! m49 dump row's walk. No wire change: the read rides the `TreeSnapshot`
//! domain byte-for-byte as M0 blessed it; every [`ClientConn`] impl
//! inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_tree_snapshot
//! # prints the summary read twice (same generation, same count), the
//! # m49 dump row's agreeing count, and the window-scoped read's whole tree
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-client-tree-snapshot-demo/0.1")
            .await
            .expect("handshake");

        // Two summary reads back to back: a read pumps, it never mutates —
        // the generation and the count stay put.
        let first = client.tree_snapshot().await.expect("first summary read");
        assert!(first.tree_seq >= 1, "the attach pump applied the fixture");
        let second = client.tree_snapshot().await.expect("second summary read");
        assert_eq!(first, second, "a read is not a mutation");
        println!(
            "summary → tree_seq {} at {} nodes",
            first.tree_seq, first.node_count
        );

        // The summary counts what the m49 dump row walks: the two reads
        // agree at the same generation.
        let dump = client.tree_dump().await.expect("the m49 full dump");
        assert_eq!(dump.tree_seq, first.tree_seq, "the same generation");
        assert_eq!(dump.node_count, first.node_count, "the same count");
        println!(
            "dump    → {} nodes, agreeing with the summary",
            dump.node_count
        );

        // Session-wide by design: aiming the connection at a window still
        // reads the whole tree.
        let windows = client.window_list().await.expect("the window list");
        let aimed = client
            .window_use(windows[0].handle.clone())
            .await
            .expect("the window aim");
        assert_eq!(aimed, windows[0].handle);
        let scoped = client
            .tree_snapshot()
            .await
            .expect("the scoped summary read");
        assert_eq!(
            scoped.node_count, first.node_count,
            "scope narrows locators"
        );
        println!(
            "scoped  → still {} nodes (the whole tree)",
            scoped.node_count
        );

        handle.shutdown();
    });
}
