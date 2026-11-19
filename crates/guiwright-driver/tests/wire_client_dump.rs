//! The client seam's dump rows over the wire (m49): [`ClientConn::tree_dump`]
//! and [`ClientConn::tree_dump_under`] carry the §7 tree-dump domain (M11/M13)
//! as typed seam sugar — the whole logical tree, or the depth-first walk
//! rooted at one resolved subtree, rebuilt into the [`TreeDump`] carrier
//! without a raw request or a variant destructure anywhere. **No wire
//! change**: both ride the `Snapshot` domain byte-for-byte as M11/M13
//! blessed them; every [`ClientConn`] impl inherits the rows, the same shape
//! the TypeScript client has spoken since M3 (`treeDump()` /
//! `treeDumpUnder()`) and the REPL since M11 (`snapshot` / `snapshot
//! under`). Driven here over the in-process door with the four-button
//! fixture — the same reality `tree_dump.rs` pins at the raw-request level.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{ErrorCode, NodeKeyV1, SelectorV1, TextMatchV1};
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

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-dump/0.1")
        .await
        .expect("handshake");
    client
}

/// The full walk on one seam call: the four-button fixture answers its
/// eight nodes, depth-first per window root — `node_count` matches the
/// row vec, the children counts rebuild the hierarchy, and the hidden
/// button rides the walk bounds-less (the non-strict dump contract).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_dumps_the_whole_tree_depth_first() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let dump = client.tree_dump().await.expect("the full dump answers");

    assert_eq!(
        dump.node_count,
        dump.nodes.len(),
        "the count is the row vec, redundancy for readers"
    );
    assert!(dump.tree_seq > 0, "the dump names the tree generation");

    // DFS arithmetic: the children counts rebuild the hierarchy without
    // a parent pointer per row (the M11 contract the REPL relies on).
    // Single-window fixture: one root, so every non-root row is exactly
    // one parent's child — the sum of children counts is len - 1.
    let children_sum: u32 = dump.nodes.iter().map(|row| row.children).sum();
    assert_eq!(
        usize::try_from(children_sum).expect("children counts fit"),
        dump.nodes.len() - 1,
        "the children counts rebuild the single-root hierarchy"
    );

    let save = dump
        .nodes
        .iter()
        .find(|row| row.name.as_deref() == Some("Save"))
        .expect("the Save row rides the walk");
    assert_eq!(save.role, "Button");
    assert!(save.bounds.is_some(), "the Save button is laid out");

    let hidden = dump
        .nodes
        .iter()
        .find(|row| row.name.as_deref() == Some("Invisible"))
        .expect("the hidden row rides the walk");
    assert!(
        hidden.bounds.is_none(),
        "a bounds-less node rides the dump (the non-strict contract)"
    );

    handle.shutdown();
}

/// The §8.1 subtree read on the seam: `tree_dump_under` resolves (the
/// driver's auto-wait inside `timeout_ms`, the connection's window scope
/// narrowing the match) and dumps only the match's subtree — a leaf
/// locator answers one row, itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_dumps_one_subtree_under_a_locator() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let dump = client
        .tree_dump_under(named_button(Some("Save")), 2000)
        .await
        .expect("the Save subtree dumps");
    assert_eq!(dump.nodes.len(), 1, "a leaf locator dumps itself");
    assert_eq!(dump.nodes[0].name.as_deref(), Some("Save"));
    assert_eq!(dump.nodes[0].children, 0, "a leaf closes the walk");

    // The full dump is strictly bigger — the seam carries both reads.
    let full = client.tree_dump().await.expect("the full dump answers");
    assert!(full.node_count > dump.node_count, "the walk roots higher");

    handle.shutdown();
}

/// The resolve's honesty rides the sugar: a locator that never matches
/// waits out the budget inside `timeout_ms` and answers the wire's typed
/// `Timeout` — the same rejection the raw `Snapshot::Filtered` carries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_subtree_times_out_with_the_typed_rejection() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let started = std::time::Instant::now();
    let err = client
        .tree_dump_under(named_button(Some("Missing")), 300)
        .await
        .expect_err("no such button");
    let elapsed = started.elapsed();
    match err {
        GwpError::Rejected { code, .. } => assert_eq!(code, ErrorCode::Timeout),
        other => panic!("expected the typed rejection, got {other:?}"),
    }
    assert!(
        elapsed >= Duration::from_millis(250),
        "the resolve actually waited, took {elapsed:?}"
    );

    handle.shutdown();
}

/// The dump-row rule, made typable: a seam dump row's key feeds
/// [`ClientConn::describe`] on the next line — the identity facts the
/// dump carried, re-read at the handle, no raw request anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dump_row_feeds_the_describe_read() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let dump = client
        .tree_dump_under(
            SelectorV1::Role {
                role: "TextInput".into(),
                name: Some(TextMatchV1 {
                    text: "Email".into(),
                    exact: true,
                    ignore_case: false,
                }),
            },
            2000,
        )
        .await
        .expect("the Email subtree dumps");
    let row = &dump.nodes[0];
    assert!(
        matches!(row.key, NodeKeyV1::AccessKit { .. }),
        "{:?}",
        row.key
    );

    // The Email input carries its value into the dump (the m11 identity
    // facts); the describe read re-reads the same facts at the handle.
    assert_eq!(row.value.as_deref(), Some("ada@example.dev"));
    let described = client
        .describe(row.key.clone(), 0)
        .await
        .expect("the dump row re-reads");
    assert_eq!(described.value.as_deref(), Some("ada@example.dev"));

    handle.shutdown();
}
