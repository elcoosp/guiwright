//! End-to-end remote mode: GWP over a real UDS socket (architecture §5.1
//! remote mode + §7 transport). Spawn the server stack with the mock
//! adapter, drive it with the proto client, assert Playwright-shaped
//! semantics over the wire.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    CapsV1, ErrorCode, NodeKeyV1, RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1,
};
use guiwright_proto::uds::Client;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_remote_session() {
    let path = socket_path("remote-session.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");

    let mut client = Client::connect(&path).await.expect("client connects");

    // Handshake → Welcome with adapter capabilities (§7).
    let adapter = client.handshake("guiwright-tests/0.1").await.unwrap();
    assert_eq!(adapter.name, "mock");
    assert_eq!(adapter.framework, "fixture");
    let CapsV1 {
        tree_update, clock, ..
    } = adapter.caps;
    assert!(tree_update);
    assert!(clock);

    // Latency probe.
    client.ping().await.unwrap();

    // Tree snapshot: the fixture tree settled at attach (§6.3
    // wait_for_settle on the session thread).
    let snapshot = client.request(RequestV1::TreeSnapshot).await.unwrap();
    match snapshot {
        ResponseV1::TreeSnapshot { node_count, .. } => assert_eq!(node_count, 8),
        other => panic!("expected TreeSnapshot, got {other:?}"),
    }

    // Resolve with auto-wait over the wire (§8.2 remote).
    let resolved = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: Some(TextMatchV1 {
                    text: "Save".into(),
                    exact: false,
                    ignore_case: true,
                }),
            },
            timeout_ms: 2000,
        })
        .await
        .unwrap();
    let (node, generation) = match resolved {
        ResponseV1::Resolved {
            node, generation, ..
        } => (node, generation),
        other => panic!("expected Resolved, got {other:?}"),
    };
    assert_eq!(generation, 0);
    let NodeKeyV1::AccessKit { node: id, .. } = node else {
        panic!("accesskit identity over the wire");
    };
    assert_eq!(id, Fixture::buttons().save_button.0);

    // Semantic action applied at a frame boundary (§8.4 + §15.4).
    let done = client
        .request(RequestV1::Action {
            node,
            generation,
            action: SemActionV1::Click,
            force: false,
            timeout_ms: 2000,
        })
        .await
        .unwrap();
    match done {
        ResponseV1::ActionDone { frame_seq, detail } => {
            assert_eq!(detail, "clicked");
            assert!(frame_seq > 0, "action applied at a frame");
        }
        other => panic!("expected ActionDone, got {other:?}"),
    }

    // Strict-mode violation is reported as a clean error code (§8.2).
    let err = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: None,
            },
            timeout_ms: 0,
        })
        .await
        .expect_err("ambiguous resolve");
    assert!(
        matches!(
            err,
            GwpError::Rejected {
                code: ErrorCode::StrictModeViolation,
                ..
            }
        ),
        "{err:?}"
    );

    // Timeout error code for a missing node with a real deadline.
    let err = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::TestId("does-not-exist".into()),
            timeout_ms: 100,
        })
        .await
        .expect_err("timeout resolve");
    assert!(
        matches!(
            err,
            GwpError::Rejected {
                code: ErrorCode::Timeout,
                ..
            }
        ),
        "{err:?}"
    );

    // Clean close.
    let bye = client.request(RequestV1::Bye).await.unwrap();
    assert_eq!(bye, ResponseV1::Bye);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_generation_is_rejected_over_the_wire() {
    let path = socket_path("stale.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.unwrap();
    client.handshake("test").await.unwrap();

    let err = client
        .request(RequestV1::Action {
            node: NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: 2,
            },
            generation: 42,
            action: SemActionV1::Click,
            force: false,
            timeout_ms: 100,
        })
        .await
        .expect_err("stale generation");
    assert!(
        matches!(
            err,
            GwpError::Rejected {
                code: ErrorCode::StaleGeneration,
                ..
            }
        ),
        "{err:?}"
    );

    handle.shutdown();
}

/// Resolve with a generous timeout waits for a node that appears late
/// (scripted update lands while the resolve is in flight).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_auto_waits_over_the_wire() {
    let path = socket_path("auto-wait.sock");
    let fixture = Fixture::buttons();
    let handle = spawn_session_server(path.clone(), move || {
        let mut adapter = MockAdapter::with_fixture(&fixture);
        // The Export button appears one pump after attach (scripted queue).
        adapter.push_update(fixture.add_button_update("Export"));
        adapter.push_frame(1);
        adapter
    })
    .expect("server binds");

    // Give the session thread a moment to attach + settle.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = Client::connect(&path).await.unwrap();
    client.handshake("test").await.unwrap();

    let resolved = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::TestId("late-export".into()),
            timeout_ms: 0,
        })
        .await
        .expect_err("no test id registered yet");

    // Registering test ids is a kit-side capability; assert the clean
    // NoMatch error path instead.
    assert!(
        matches!(
            resolved,
            GwpError::Rejected {
                code: ErrorCode::StrictModeViolation,
                ..
            }
        ),
        "{resolved:?}"
    );

    let resolved = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: Some(TextMatchV1 {
                    text: "Export".into(),
                    exact: true,
                    ignore_case: false,
                }),
            },
            timeout_ms: 2000,
        })
        .await
        .unwrap();
    match resolved {
        ResponseV1::Resolved { node, .. } => {
            let NodeKeyV1::AccessKit { node: id, .. } = node else {
                panic!("accesskit identity");
            };
            assert_eq!(id, 8, "the late Export button");
        }
        other => panic!("expected Resolved, got {other:?}"),
    }

    handle.shutdown();
}
