//! The node-describe read (M36): one node's identity facts — role, name,
//! value, bounds — as `Describe` → `Described` over GWP, with the typed
//! [`ClientConn::describe`] sugar riding the client seam. "What IS the
//! node this handle points at" is the debugging read the m32 record's
//! standing gap names: the dump carries role/name for whole-subtree
//! questions, but a one-row read did not exist, so the REPL's `hit`/`all`
//! rows could only render the bare handle (`accesskit#2`). The handle is
//! the dump-row form (directly usable as the `Action` node); the
//! generation guard is the Action boundary's — a stale handle is
//! `StaleGeneration`, a dead one `UnknownNode`.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    DescribeAllV1, DescribeV1, ErrorCode, NodeKeyV1, RequestV1, ResponseV1, SelectorV1,
};
use guiwright_proto::{ClientConn, DuplexClient};

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-describe-test/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn_fixture() -> (
    GwpServerHandle,
    guiwright_proto::duplex::DuplexConnect,
    std::sync::Arc<std::sync::Mutex<Vec<accesskit::TreeUpdate>>>,
) {
    let scripts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let queued = scripts.clone();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(queued),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, scripts)
}

/// Resolve a locator through the wire (strict mode) — the way a client
/// mints the handle a describe reads.
async fn resolve(
    client: &mut DuplexClient,
    selector: SelectorV1,
) -> (NodeKeyV1, u32, Option<[f64; 4]>) {
    match client
        .request(RequestV1::Resolve {
            selector,
            timeout_ms: 2000,
        })
        .await
        .expect("resolve round-trip")
    {
        ResponseV1::Resolved {
            node,
            generation,
            rect,
        } => (node, generation, rect),
        other => panic!("expected Resolved, got {other:?}"),
    }
}

async fn describe(client: &mut DuplexClient, node: &NodeKeyV1, generation: u32) -> ResponseV1 {
    client
        .request(RequestV1::Describe(DescribeV1 {
            node: node.clone(),
            generation,
        }))
        .await
        .expect("describe round-trip")
}

#[tokio::test]
async fn describe_answers_the_resolved_node() {
    let (handle, connect, _scripts) = spawn_fixture();
    let mut client = connected(&connect).await;

    // Mint the handle the way a client does — resolve, then describe it:
    // role, name and bounds in one row, the facts the dump row carries.
    let (node, generation, rect) = resolve(
        &mut client,
        SelectorV1::Role {
            role: "Button".into(),
            name: Some(guiwright_proto::schema::TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        },
    )
    .await;
    let NodeKeyV1::AccessKit { tree, node: id } = &node else {
        panic!("accesskit fixture keys");
    };
    assert_eq!(*tree, [0u8; 16], "the fixture rides the root tree id");
    assert_eq!(*id, 2, "the fixture's Save button");

    match describe(&mut client, &node, generation).await {
        ResponseV1::Described(row) => {
            assert_eq!(row.node, node, "the row echoes the handle");
            assert_eq!(row.generation, generation, "still live");
            assert_eq!(row.role, "Button");
            assert_eq!(row.name.as_deref(), Some("Save"));
            assert_eq!(row.value, None, "a button carries no value");
            assert_eq!(row.bounds, rect, "the resolve rect, restated");
        }
        other => panic!("expected Described, got {other:?}"),
    }

    handle.shutdown();
}

#[tokio::test]
async fn describe_reads_the_value_an_input_carries() {
    let (handle, connect, _scripts) = spawn_fixture();
    let mut client = connected(&connect).await;

    let (node, generation, _) = resolve(
        &mut client,
        SelectorV1::Role {
            role: "TextInput".into(),
            name: Some(guiwright_proto::schema::TextMatchV1 {
                text: "Email".into(),
                exact: true,
                ignore_case: false,
            }),
        },
    )
    .await;

    match describe(&mut client, &node, generation).await {
        ResponseV1::Described(row) => {
            assert_eq!(row.role, "TextInput");
            assert_eq!(row.name.as_deref(), Some("Email"));
            assert_eq!(row.value.as_deref(), Some("ada@example.dev"));
        }
        other => panic!("expected Described, got {other:?}"),
    }

    handle.shutdown();
}

#[tokio::test]
async fn a_removed_handle_is_a_typed_error() {
    let (handle, connect, scripts) = spawn_fixture();
    let mut client = connected(&connect).await;

    let (node, generation, _) = resolve(
        &mut client,
        SelectorV1::Role {
            role: "Button".into(),
            name: Some(guiwright_proto::schema::TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        },
    )
    .await;

    // The mock's scripted removal: the fixture's Save button leaves the
    // tree, bumping its generation — the stale handle cannot describe.
    // The read pumps first (consistency, like `resolve`), so the very
    // describe round-trip delivers the queued diff before its guard runs.
    scripts
        .lock()
        .expect("script lock")
        .push(Fixture::buttons().remove_save_update());

    // The client layer turns a `ResponseV1::Error` into the typed
    // `GwpError::Rejected` — the stale handle reads as `StaleGeneration`.
    let err = client
        .request(RequestV1::Describe(DescribeV1 {
            node: node.clone(),
            generation,
        }))
        .await
        .expect_err("the stale handle cannot describe");
    let GwpError::Rejected {
        code: ErrorCode::StaleGeneration,
        ..
    } = err
    else {
        panic!("expected StaleGeneration, got {err:?}");
    };

    // A handle that never existed is `UnknownNode` — the same boundary
    // the Action arm enforces (the m32 dump-row rule's error shape).
    let ghost = NodeKeyV1::AccessKit {
        tree: [0u8; 16],
        node: 999,
    };
    let err = client
        .request(RequestV1::Describe(DescribeV1 {
            node: ghost,
            generation: 0,
        }))
        .await
        .expect_err("the ghost handle cannot describe");
    let GwpError::Rejected {
        code: ErrorCode::UnknownNode,
        ..
    } = err
    else {
        panic!("expected UnknownNode, got {err:?}");
    };

    handle.shutdown();
}

#[tokio::test]
async fn the_client_sugar_rides_the_trait() {
    let (handle, connect, _scripts) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The ClientConn default method — the same surface every shipped
    // client and downstream transport inherits (the hit_test pattern).
    let (node, generation, rect) = resolve(
        &mut client,
        SelectorV1::Role {
            role: "Label".into(),
            name: Some(guiwright_proto::schema::TextMatchV1 {
                text: "Welcome, Ada".into(),
                exact: true,
                ignore_case: false,
            }),
        },
    )
    .await;
    let row = client
        .describe(node.clone(), generation)
        .await
        .expect("the sugar round-trip");
    assert_eq!(row.node, node);
    assert_eq!(row.generation, generation);
    assert_eq!(row.role, "Label");
    assert_eq!(row.name.as_deref(), Some("Welcome, Ada"));
    assert_eq!(row.bounds, rect);

    handle.shutdown();
}

// --- The describe-all read (M39) ---------------------------------------------

#[tokio::test]
async fn describe_all_carries_the_set_facts_in_document_order() {
    let (handle, connect, _scripts) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The set read carrying facts: every current match of the selector,
    // document order, one round trip — the resolve-all walk's rows plus
    // the Described identity facts (the m37 record's standing gap).
    match client
        .request(RequestV1::DescribeAll(DescribeAllV1 {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: None,
            },
        }))
        .await
        .expect("describe-all round-trip")
    {
        ResponseV1::DescribedAll { rows } => {
            let names: Vec<Option<&str>> = rows.iter().map(|r| r.name.as_deref()).collect();
            assert_eq!(
                names,
                vec![
                    Some("Save"),
                    Some("Cancel"),
                    Some("Invisible"),
                    Some("Confirm")
                ],
                "document order, hidden nodes included (the set-read rule)"
            );
            assert!(rows.iter().all(|r| r.role == "Button"));
            assert_eq!(
                rows[0].bounds,
                Some([16.0, 16.0, 136.0, 48.0]),
                "the resolve rect, restated"
            );
            assert!(rows.iter().all(|r| r.generation == 0), "rows are live");
        }
        other => panic!("expected DescribedAll, got {other:?}"),
    }

    // An empty set answers with an empty list — the set read is total,
    // never an error (the m30 snapshot rule).
    match client
        .request(RequestV1::DescribeAll(DescribeAllV1 {
            selector: SelectorV1::TestId("ghost".into()),
        }))
        .await
        .expect("empty describe-all round-trip")
    {
        ResponseV1::DescribedAll { rows } => assert_eq!(rows.len(), 0),
        other => panic!("expected DescribedAll, got {other:?}"),
    }

    handle.shutdown();
}

#[tokio::test]
async fn the_describe_all_sugar_rides_the_trait() {
    let (handle, connect, _scripts) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The ClientConn default method — the same surface every shipped
    // client and downstream transport inherits (the describe pattern).
    let rows = client
        .describe_all(SelectorV1::Role {
            role: "TextInput".into(),
            name: None,
        })
        .await
        .expect("the sugar round-trip");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, "TextInput");
    assert_eq!(rows[0].name.as_deref(), Some("Email"));
    assert_eq!(rows[0].value.as_deref(), Some("ada@example.dev"));

    handle.shutdown();
}
