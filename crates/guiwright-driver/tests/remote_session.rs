//! End-to-end remote mode: GWP over a real UDS socket (architecture §5.1
//! remote mode + §7 transport). Spawn the server stack with the mock
//! adapter, drive it with the proto client, assert Playwright-shaped
//! semantics over the wire.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_driver::spawn_session_server;
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    CapsV1, ClockV1, ErrorCode, MockV1, NodeKeyV1, RequestV1, ResponseV1, SelectorV1, SemActionV1,
    TextMatchV1,
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

// --- M2: clock & mock domains over a real socket (§7, §14) ------------------

/// The spike's harness: a remote session with app-boot seams, driven end
/// to end. This is the same surface a GPUI remote session will use (§17
/// "remote-mode GPUI spike"): the GPUI adapter is M3, but the *remote*
/// contract — handshake → resolve → act → clock → mock — is fully
/// exercised here against the mock adapter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clock_and_mock_domains_over_the_wire() {
    #[derive(Debug, Default)]
    struct SpikeState {
        fixture: Option<String>,
        rows: usize,
    }

    let mut seams = guiwright_driver::SeamRegistry::new();
    seams.register("reset_state", |ctx, scope| {
        let state = ctx
            .downcast_mut::<SpikeState>()
            .ok_or("wrong runner context type")?;
        state.rows = 0;
        state.fixture = Some(scope.to_string());
        Ok(format!("state reset to {scope}"))
    });
    seams.register("seed_users", |ctx, _scope| {
        let state = ctx
            .downcast_mut::<SpikeState>()
            .ok_or("wrong runner context type")?;
        state.rows = 3;
        Ok("3 users seeded".into())
    });

    let path = socket_path("remote-clock-mock.sock");
    let handle = GwpServerHandle::spawn_with_seams(
        path.clone(),
        || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_state(SpikeState {
                rows: 9,
                fixture: None,
            })
        },
        seams,
    )
    .expect("server binds");

    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("m2-spike/0.1").await.unwrap();

    // Clock: install + stepped fast-forward (§14), reported in ms.
    let state = client
        .request(RequestV1::Clock(ClockV1::InstallVirtual))
        .await
        .unwrap();
    match state {
        ResponseV1::ClockState {
            virtual_time_ms, ..
        } => {
            assert_eq!(virtual_time_ms, Some(0));
        }
        other => panic!("expected ClockState, got {other:?}"),
    }
    let state = client
        .request(RequestV1::Clock(ClockV1::FastForward { ms: 50 }))
        .await
        .unwrap();
    match state {
        ResponseV1::ClockState {
            virtual_time_ms,
            steps,
            ..
        } => {
            assert_eq!(virtual_time_ms, Some(50));
            assert_eq!(steps, 3, "50 ms = 3 frame-sized steps");
        }
        other => panic!("expected ClockState, got {other:?}"),
    }

    // Mock: canned override wins, then clear runs the real hook in the
    // runner context (§14 state seams over the wire).
    client
        .request(RequestV1::Mock(MockV1::InstallSeam {
            id: "reset_state".into(),
            response: Some("canned".into()),
        }))
        .await
        .unwrap();
    let done = client
        .request(RequestV1::Mock(MockV1::ResetState {
            scope: "seed-42".into(),
        }))
        .await
        .unwrap();
    match done {
        ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "canned"),
        other => panic!("expected MockDone, got {other:?}"),
    }

    client
        .request(RequestV1::Mock(MockV1::Clear {
            id: "reset_state".into(),
        }))
        .await
        .unwrap();
    let done = client
        .request(RequestV1::Mock(MockV1::ResetState {
            scope: "seed-42".into(),
        }))
        .await
        .unwrap();
    match done {
        ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "state reset to seed-42"),
        other => panic!("expected MockDone, got {other:?}"),
    }

    // A second seam, called directly.
    let done = client
        .request(RequestV1::Mock(MockV1::Call {
            id: "seed_users".into(),
            scope: "smoke".into(),
        }))
        .await
        .unwrap();
    match done {
        ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "3 users seeded"),
        other => panic!("expected MockDone, got {other:?}"),
    }

    handle.shutdown();
}

// --- M3: the same remote session over TCP (§7 "TCP for CI containers") ------

/// Byte-identical remote session, TCP transport: the M3 other-language
/// clients (clients/typescript) drive exactly this conversation. The
/// handshake → resolve → act → clock sequence below is the TS e2e's Rust
/// twin; `spawn_tcp` on an ephemeral port reports the bound address on the
/// handle endpoint.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_remote_session_over_tcp() {
    use guiwright_proto::tcp::Client as TcpClient;

    let handle = GwpServerHandle::spawn_tcp(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("tcp server binds");
    let addr = match handle.endpoint() {
        guiwright_driver::server::Endpoint::Tcp(addr) => *addr,
        other => panic!("expected TCP endpoint, got {other:?}"),
    };
    assert_ne!(addr.port(), 0, "ephemeral bind reported");

    let mut client = TcpClient::connect(&addr.to_string())
        .await
        .expect("client connects");
    let adapter = client.handshake("ts-client/0.1").await.unwrap();
    assert_eq!(adapter.name, "mock");
    assert!(adapter.caps.tree_update);
    client.ping().await.unwrap();

    // Resolve + act (the TS client's core two round-trips).
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
        ResponseV1::ActionDone { detail, .. } => assert_eq!(detail, "clicked"),
        other => panic!("expected ActionDone, got {other:?}"),
    }

    // Clock vocabulary rides the same transport.
    let state = client
        .request(RequestV1::Clock(ClockV1::FastForward { ms: 32 }))
        .await
        .unwrap();
    match state {
        ResponseV1::ClockState {
            virtual_time_ms, ..
        } => assert_eq!(virtual_time_ms, Some(32)),
        other => panic!("expected ClockState, got {other:?}"),
    }

    let bye = client.request(RequestV1::Bye).await.unwrap();
    assert_eq!(bye, ResponseV1::Bye);

    handle.shutdown();
}

/// §7 `Video` domain over the wire (§10, M3): Start → drive (pumps
/// present frames) → Stop returns a finalized artifact path. The session
/// records into `GUIWRIGHT_VIDEO_DIR` (test override) because the reply's
/// path is how clients learn where the file landed; the dir rides the
/// Start options (Playwright-style client-owned artifact dir).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn video_domain_over_the_wire() {
    let dir = tempfile::tempdir().expect("tmpdir");

    let handle = GwpServerHandle::spawn_tcp(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
                24,
                12,
                vec![64; 24 * 12 * 4],
            )
        },
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("tcp server binds");
    let addr = match handle.endpoint() {
        guiwright_driver::server::Endpoint::Tcp(addr) => *addr,
        other => panic!("expected TCP endpoint, got {other:?}"),
    };

    let mut client = guiwright_proto::tcp::Client::connect(&addr.to_string())
        .await
        .expect("client connects");
    client.handshake("video-client/0.1").await.unwrap();

    // Start: Pong (§7 video domain acknowledges control commands). The
    // client names the artifact dir (Playwright-style ownership).
    let resp = client
        .request(RequestV1::Video(guiwright_proto::schema::VideoV1::Start(
            guiwright_proto::schema::VideoOptsV1 {
                fps: 25,
                dir: Some(dir.path().display().to_string()),
            },
        )))
        .await
        .unwrap();
    assert_eq!(resp, ResponseV1::Pong);

    // Drive the session: each request batch pumps the adapter once and
    // the presented frames feed the recorder (hash-0 frames; the first
    // lands, the static span dedups).
    let _ = client
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

    // Stop: the finalized artifact summary rides the reply.
    let resp = client
        .request(RequestV1::Video(guiwright_proto::schema::VideoV1::Stop))
        .await
        .unwrap();
    let (path, frames) = match resp {
        ResponseV1::VideoFile {
            path,
            frames,
            dropped: _,
            duration_ms: _,
        } => (path, frames),
        other => panic!("expected VideoFile, got {other:?}"),
    };
    assert!(frames >= 1, "at least the first presented frame recorded");
    assert!(
        std::path::Path::new(&path).exists(),
        "artifact exists at {path}"
    );

    // Stop without a running recording is a clean error (session rules).
    let err = client
        .request(RequestV1::Video(guiwright_proto::schema::VideoV1::Stop))
        .await
        .expect_err("no recording running");
    assert!(
        matches!(err, GwpError::Rejected { .. }),
        "clean rejected error: {err:?}"
    );

    handle.shutdown();
}
