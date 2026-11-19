//! The client seam's aim-and-act rows over the wire (m47): [`ClientConn::resolve`]
//! and [`ClientConn::action`] carry the §7 locate-and-act loop as typed seam
//! sugar — a strict-mode resolve with the driver's auto-wait answering an
//! `Action`-usable row (the dump-row rule, M11, made typable), and the action
//! applied on any row the client already holds. **No wire change**: both ride
//! the `Resolve` and `Action` domains byte-for-byte as M0/M17 blessed them;
//! every [`ClientConn`] impl inherits the rows, the same shape the
//! TypeScript client has spoken since M3 (`resolve()` / `action()`). Driven
//! here over the in-process door with the four-button fixture — the same
//! reality `wire_action_opts.rs` pins at the raw-request level.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{ErrorCode, NodeKeyV1, SelectorV1, SemActionV1, TextMatchV1};
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
        .handshake("wire-resolve-act/0.1")
        .await
        .expect("handshake");
    client
}

/// The happy §7 loop on two seam calls: a strict resolve answering the
/// Save row (handle + generation + bounds), then the action applied on
/// that row — `clicked` at a named frame, no raw request anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_resolves_and_aims_the_row_at_an_action() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let row = client
        .resolve(named_button(Some("Save")), 2000)
        .await
        .expect("the Save row resolves");
    assert!(
        matches!(row.node, NodeKeyV1::AccessKit { .. }),
        "{:?}",
        row.node
    );
    assert!(row.rect.is_some(), "the Save button is laid out");
    assert_eq!(row.generation, 0, "the first resolve generation");

    let receipt = client
        .action(
            row.node.clone(),
            row.generation,
            SemActionV1::Click,
            false,
            5000,
        )
        .await
        .expect("the action lands");
    assert_eq!(receipt.detail, "clicked");
    assert!(
        receipt.frame_seq > 0,
        "the action reports the frame it applied at"
    );

    handle.shutdown();
}

/// The seam's resolve is strict (the §8.2 policy the wire `Resolve`
/// serves): the bare role locator matches four buttons and answers the
/// typed `StrictModeViolation` — never a first-match guess.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_mode_violation_is_the_typed_rejection() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let err = client
        .resolve(named_button(None), 200)
        .await
        .expect_err("four buttons match");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::StrictModeViolation, "{message}");
            assert!(
                message.contains("resolved to"),
                "the violation names the match count: {message}"
            );
        }
        other => panic!("expected the typed rejection, got {other:?}"),
    }

    handle.shutdown();
}

/// The driver's auto-wait rides the seam resolve: a missing locator
/// waits out the budget inside `timeout_ms` and answers the wire's
/// typed `Timeout` — the same honesty the raw `Resolve` carries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_row_times_out_with_the_typed_rejection() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let started = std::time::Instant::now();
    let err = client
        .resolve(named_button(Some("Missing")), 300)
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

/// The Action boundary's generation guard (§8.2 over the wire): a row
/// whose generation has moved on answers `StaleGeneration`, and a dead
/// node answers `UnknownNode` — the seam carries the same honesty the
/// raw `Action` request has had since M17.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_and_dead_rows_are_the_typed_rejections() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let row = client
        .resolve(named_button(Some("Save")), 2000)
        .await
        .expect("the Save row resolves");

    // A moved-on generation: the ABA guard rejects before any act.
    let err = client
        .action(
            row.node.clone(),
            row.generation + 1,
            SemActionV1::Click,
            false,
            500,
        )
        .await
        .expect_err("the generation has moved on");
    match err {
        GwpError::Rejected { code, .. } => assert_eq!(code, ErrorCode::StaleGeneration),
        other => panic!("expected the typed rejection, got {other:?}"),
    }

    // A dead node: the fabricated handle is not in the consumer tree.
    // The Action boundary answers the wire's `StrictModeViolation` — the
    // M0 `NoMatch` mapping this arm has carried since the dump-row rule
    // made rows act handles, the message naming the dead handle (the
    // `Describe` boundary's `UnknownNode` is that read's own contract).
    let dead = NodeKeyV1::AccessKit {
        tree: match &row.node {
            NodeKeyV1::AccessKit { tree, .. } => *tree,
            other @ NodeKeyV1::Logical { .. } => {
                panic!("expected an AccessKit handle, got {other:?}")
            }
        },
        node: 9999,
    };

    let err = client
        .action(dead, row.generation, SemActionV1::Click, false, 500)
        .await
        .expect_err("no such node");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::StrictModeViolation, "{message}");
            assert!(
                message.contains("9999"),
                "the rejection names the dead handle: {message}"
            );
        }
        other => panic!("expected the typed rejection, got {other:?}"),
    }

    handle.shutdown();
}
