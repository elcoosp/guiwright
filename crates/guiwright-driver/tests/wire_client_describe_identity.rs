//! The single-node identity describe read over the wire (m66): the m64
//! record's named standing gap — the M36 `Describe` read keeps its
//! identity-four encoding and the description fact rides the identity
//! *set* read only — closed by the appended `DescribeIdentity` /
//! `DescribedIdentity` domain. The payloads reuse the shapes every
//! client already speaks (the M36 handle pair in, the M64 row out), so
//! the read costs one round trip where the set read walks, and the
//! handle guards are the [`RequestV1::Action`] boundary's exact typed
//! errors. Driven here over the in-process door with the described
//! fixture.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{ErrorCode, SelectorV1, TextMatchV1};

/// One named button — a narrowed set whose head is that row alone.
fn named_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-describe-identity-test/0.1")
        .await
        .expect("handshake");
    client
}

/// The described fixture — the buttons base plus the Install button
/// whose row carries `"Installs the latest build"` (the m38 extension
/// band, the fact the M36 identity four left out).
fn spawn_described() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::with_description()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

/// Satisfied on entry: a resolve-all row mints the handle (the dump-row
/// rule, M11), the identity read answers it — the M64 row's identity
/// four plus the description fact, one round trip instead of the
/// whole-set walk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_identity_answers_the_fact_and_the_identity_four() {
    let (handle, connect) = spawn_described();
    let mut client = connected(&connect).await;

    let rows = client
        .resolve_all(named_button("Install"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the Install button resolves");
    let described = client
        .describe_identity(row.node.clone(), row.generation)
        .await
        .expect("the identity read answers the handle");
    assert_eq!(described.node, row.node, "the handle echoes the request's");
    assert_eq!(described.generation, row.generation);
    assert_eq!(described.role, "Button");
    assert_eq!(described.name.as_deref(), Some("Install"));
    assert_eq!(described.value, None);
    assert!(
        described.bounds.is_some(),
        "the identity four ride beside the fact"
    );
    assert_eq!(
        described.description.as_deref(),
        Some("Installs the latest build"),
        "the fact the M36 encoding left out"
    );

    handle.shutdown();
}

/// The absent fact rides as `None` (the m38 read: the name and the value
/// never answer a description — the Save button has no description, and
/// the read still answers, a total read, never an error).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_absent_description_rides_as_none() {
    let (handle, connect) = spawn_described();
    let mut client = connected(&connect).await;

    let rows = client
        .resolve_all(named_button("Save"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the Save button resolves");
    let described = client
        .describe_identity(row.node.clone(), row.generation)
        .await
        .expect("the identity read answers the handle");
    assert_eq!(described.name.as_deref(), Some("Save"));
    assert_eq!(
        described.description, None,
        "the absent fact is None, never the name or the value"
    );

    handle.shutdown();
}

/// A handle the tree does not know rides the typed
/// `ErrorCode::UnknownNode` — the [`RequestV1::Action`] boundary's
/// honesty, the same error an action on the ghost row would meet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ghost_handle_rides_the_typed_unknown_node() {
    let (handle, connect) = spawn_described();
    let mut client = connected(&connect).await;

    let ghost = guiwright_proto::schema::NodeKeyV1::AccessKit {
        tree: [0xa5; 16],
        node: 4242,
    };
    let err = client
        .describe_identity(ghost, 0)
        .await
        .expect_err("the tree cannot describe a node it does not know");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::UnknownNode, "the typed code: {message}");
            assert!(
                message.contains("node"),
                "the boundary names the handle: {message}"
            );
        }
        other => panic!("expected the typed unknown-node error, got {other:?}"),
    }

    handle.shutdown();
}
