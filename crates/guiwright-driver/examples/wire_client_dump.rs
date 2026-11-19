//! The tree-dump seam rows (m49): [`ClientConn::tree_dump`] and
//! [`ClientConn::tree_dump_under`] carry the §7 tree-dump domain (M11/M13)
//! as typed sugar — the whole logical tree, or the depth-first walk rooted
//! at one resolved subtree, rebuilt into the [`TreeDump`] carrier without
//! a raw request or a variant destructure anywhere. No wire change: both
//! ride the `Snapshot` domain byte-for-byte as M11/M13 blessed them; every
//! [`ClientConn`] impl inherits the rows, the same shape the TypeScript
//! client has spoken since M11 (`treeDump()`) and M13 (`treeDumpUnder()`), and the
//! REPL since M11 (`snapshot` / `snapshot under`).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_dump
//! # prints the full walk's row count and the Save row's facts, the
//! # Save subtree's one-row walk, and the typed timeout for a dead locator
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{ErrorCode, SelectorV1, TextMatchV1};
use guiwright_proto::{DuplexClient, GwpError};

fn named_button(name: Option<&str>) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: name.map(|name| TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

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
            .handshake("wire-client-dump-demo/0.1")
            .await
            .expect("handshake");

        // The full walk: depth-first per window root, the children counts
        // rebuilding the hierarchy — the dump-row rule's typed answer.
        let dump = client.tree_dump().await.expect("the full dump answers");
        println!(
            "tree @{}: {} nodes (focus present: {})",
            dump.tree_seq,
            dump.node_count,
            dump.focus.is_some()
        );
        let save = dump
            .nodes
            .iter()
            .find(|row| row.name.as_deref() == Some("Save"))
            .expect("the Save row rides the walk");
        println!("  Save row — role {} bounds {:?}", save.role, save.bounds);

        // The subtree read: resolve (auto-wait inside the budget) then
        // dump only the match's walk — a leaf locator answers one row.
        let subtree = client
            .tree_dump_under(named_button(Some("Save")), 2000)
            .await
            .expect("the Save subtree dumps");
        println!(
            "under Save: {} row(s), children {}",
            subtree.node_count, subtree.nodes[0].children
        );

        // The resolve's honesty rides the sugar: a dead locator waits out
        // the budget and answers the wire's typed `Timeout`.
        match client
            .tree_dump_under(named_button(Some("Missing")), 300)
            .await
        {
            Err(GwpError::Rejected {
                code: ErrorCode::Timeout,
                message,
            }) => println!("missing locator → timeout: {message}"),
            other => panic!("expected the typed rejection, got {other:?}"),
        }

        handle.shutdown();
    });
}
