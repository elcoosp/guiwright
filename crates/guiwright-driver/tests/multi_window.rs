//! Multi-window integration tests (architecture §16 v0.4 "Multi-window /
//! tabs — targets per window", M6): session-level window targeting and the
//! full remote-mode story — two connections over one UDS socket, each aimed
//! at a different window target, sharing one driver session (§11).

use std::time::Duration;

use guiwright_core::selector::Selector;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_core::{CoreError, NodeKey};
use guiwright_driver::Session;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    ErrorCode, NodeKeyV1, RequestV1, ResponseV1, SelectorV1, TextMatchV1, WindowV1,
};
use guiwright_proto::uds::Client;

fn save_selector() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: "Save".into(),
            exact: false,
            ignore_case: false,
        }),
    }
}

fn target_handle(fx: &MultiWindowFixture, w1: bool) -> String {
    if w1 {
        fx.w1.0.to_string()
    } else {
        fx.w2.0.to_string()
    }
}

// ---------------------------------------------------------------------------
// Session level (embedded surface, no wire)
// ---------------------------------------------------------------------------

#[test]
fn session_reports_adapter_window_targets() {
    let fx = MultiWindowFixture::new();
    let mut session = Session::new(MockAdapter::with_multi_window(&fx));
    session.settle(std::time::Instant::now() + Duration::from_secs(5));

    let windows = session.windows();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].handle, fx.w1.0.to_string());
    assert_eq!(windows[0].title, "Editor");
    assert!(windows[0].focused, "the scripted focus is reported");
    assert_eq!(windows[1].handle, fx.w2.0.to_string());
    assert!(!windows[1].focused);

    // Single-window persona: the default `main` target.
    let mut single = Session::new(MockAdapter::with_fixture(
        &guiwright_core::test_util::Fixture::buttons(),
    ));
    single.settle(std::time::Instant::now() + Duration::from_secs(5));
    let windows = single.windows();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].handle, "main");
}

#[test]
fn session_scoped_resolve_and_wait_honor_the_scope() {
    let fx = MultiWindowFixture::new();
    let mut session = Session::new(MockAdapter::with_multi_window(&fx));
    session.settle(std::time::Instant::now() + Duration::from_secs(5));

    let selector = Selector::role_named(accesskit::Role::Button, "Save");

    // Unscoped: strict mode sees both windows.
    let err = session.resolve(&selector).expect_err("two matches");
    assert!(matches!(err, CoreError::Ambiguous { .. }), "{err}");

    // Scoped to W1: exactly W1's Save, reached through the same
    // resolve/wait surface the auto-wait uses.
    let scope = session
        .window_scope(&fx.w1.0.to_string())
        .expect("w1 is a live target");
    session.set_resolve_scope(scope);
    let resolved = session.resolve(&selector).expect("scoped to w1");
    session.clear_resolve_scope();
    assert_eq!(resolved.key, fx.w1_save_key());

    // The waiter surface honors the scope too (auto-wait over the wire is
    // scoped exactly the same way).
    let scope = session
        .window_scope(&fx.w2.0.to_string())
        .expect("w2 is a live target");
    session.set_resolve_scope(scope);
    let wait = session
        .wait_for(
            &selector,
            vec![guiwright_core::actionability::Check::Attached],
            std::time::Instant::now() + Duration::from_secs(2),
        )
        .expect("scoped wait finds w2's save");
    session.clear_resolve_scope();
    let resolved = wait.into_node().expect("attached");
    assert_eq!(resolved.key, fx.w2_save_key());

    // After the scope is dropped the session is back to the whole tree.
    let err = session.resolve(&selector).expect_err("unscoped again");
    assert!(matches!(err, CoreError::Ambiguous { .. }), "{err}");
}

#[test]
fn session_window_scope_rejects_unknown_handles() {
    let fx = MultiWindowFixture::new();
    let session = Session::new(MockAdapter::with_multi_window(&fx));
    let err = session
        .window_scope("00000000-0000-0000-0000-00000000dead")
        .expect_err("no such window");
    assert!(matches!(err, CoreError::UnknownWindow { .. }), "{err}");
}

// ---------------------------------------------------------------------------
// Remote mode: two connections, one session, two window targets (§11)
// ---------------------------------------------------------------------------

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

async fn connect(path: &std::path::Path) -> Client {
    let mut client = Client::connect(path).await.expect("client connects");
    client
        .handshake("guiwright-tests/0.1")
        .await
        .expect("hello");
    client
}

fn expect_save_key(resp: ResponseV1, fx: &MultiWindowFixture, w1: bool) {
    let want = if w1 {
        fx.w1_save_key()
    } else {
        fx.w2_save_key()
    };
    let want = NodeKeyV1::from(want);
    match resp {
        ResponseV1::Resolved { node, .. } => assert_eq!(node, want, "scoped to the right window"),
        other => panic!("expected Resolved, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_connections_target_different_windows() {
    let fx = MultiWindowFixture::new();
    let scripted = fx.clone();
    let path = socket_path("multi-window.sock");
    let server = spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted)
    })
    .expect("server binds");

    let mut conn_a = connect(&path).await;
    let mut conn_b = connect(&path).await;

    // Window::List → both targets with titles and focus metadata.
    let listed = conn_a
        .request(RequestV1::Window(WindowV1::List))
        .await
        .expect("list");
    match listed {
        ResponseV1::Windows { windows } => {
            assert_eq!(windows.len(), 2);
            assert_eq!(windows[0].handle, fx.w1.0.to_string());
            assert_eq!(windows[0].title, "Editor");
            assert!(windows[0].focused);
            assert_eq!(windows[1].handle, fx.w2.0.to_string());
            assert_eq!(windows[1].title, "Preview");
        }
        other => panic!("expected Windows, got {other:?}"),
    }

    // Each connection aims at a different target — per-connection state
    // (§16 v0.4): neither Use re-aims the other connection.
    for (conn, w1) in [(&mut conn_a, true), (&mut conn_b, false)] {
        let resp = conn
            .request(RequestV1::Window(WindowV1::Use {
                handle: target_handle(&fx, w1),
            }))
            .await
            .expect("use");
        match resp {
            ResponseV1::WindowUsed { handle } => assert_eq!(handle, target_handle(&fx, w1)),
            other => panic!("expected WindowUsed, got {other:?}"),
        }
    }

    // The same-named "Save" resolves to a different node per connection —
    // the Playwright "targets per window" semantics, one session behind it.
    let sel = save_selector();
    let a = conn_a
        .request(RequestV1::Resolve {
            selector: sel.clone(),
            timeout_ms: 1_000,
        })
        .await
        .expect("conn a resolves");
    expect_save_key(a, &fx, true);
    let b = conn_b
        .request(RequestV1::Resolve {
            selector: sel,
            timeout_ms: 1_000,
        })
        .await
        .expect("conn b resolves");
    expect_save_key(b, &fx, false);

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unscoped_connection_keeps_strict_mode_across_windows() {
    let fx = MultiWindowFixture::new();
    let scripted = fx.clone();
    let path = socket_path("multi-window-strict.sock");
    let server = spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted)
    })
    .expect("server binds");

    let mut conn = connect(&path).await;
    // No Window::Use: the connection sees the WHOLE tree, so a same-named
    // locator stays a strict-mode violation — scoping is opt-in.
    let err = conn
        .request(RequestV1::Resolve {
            selector: save_selector(),
            timeout_ms: 0,
        })
        .await
        .expect_err("strict violation across windows");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::StrictModeViolation, "{message}");
            assert!(message.contains("2 nodes"), "{message}");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_window_use_fails_clean_and_applies_nothing() {
    let fx = MultiWindowFixture::new();
    let scripted = fx.clone();
    let path = socket_path("multi-window-unknown.sock");
    let server = spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted)
    })
    .expect("server binds");

    let mut conn = connect(&path).await;
    let err = conn
        .request(RequestV1::Window(WindowV1::Use {
            handle: "00000000-0000-0000-0000-00000000dead".into(),
        }))
        .await
        .expect_err("unknown window");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::UnknownWindow, "{message}");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    // A failed Use must not half-apply: the connection is still unscoped,
    // so the strict violation across both windows is intact.
    let err = conn
        .request(RequestV1::Resolve {
            selector: save_selector(),
            timeout_ms: 0,
        })
        .await
        .expect_err("still unscoped");
    match err {
        GwpError::Rejected { code, .. } => assert_eq!(code, ErrorCode::StrictModeViolation),
        other => panic!("expected Rejected, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_resolution_survives_window_local_updates() {
    let fx = MultiWindowFixture::new();
    let path = socket_path("multi-window-incremental.sock");
    // Shared script channel: the "app" pushes a window-local diff while
    // the session is live (the same pump stream every adapter uses).
    let scripts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let scripted = fx.clone();
    let channel = scripts.clone();
    let server = spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted).with_shared_scripts(channel)
    })
    .expect("server binds");

    let mut conn = connect(&path).await;
    conn.request(RequestV1::Window(WindowV1::Use {
        handle: fx.w1.0.to_string(),
    }))
    .await
    .expect("use w1");

    // The update lands AFTER the Use — the scoped auto-wait below may only
    // succeed through a W1-scoped waiter.
    scripts
        .lock()
        .expect("scripts")
        .push(fx.w1_add_button_update("Export"));

    // Auto-wait over the wire INSIDE the window: the Export button appears
    // in W1's subtree and the scoped resolve attaches to it.
    let resp = conn
        .request(RequestV1::Resolve {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: Some(TextMatchV1 {
                    text: "Export".into(),
                    exact: false,
                    ignore_case: false,
                }),
            },
            timeout_ms: 2_000,
        })
        .await
        .expect("resolve answers");
    match resp {
        ResponseV1::Resolved { node, .. } => match node {
            NodeKeyV1::AccessKit { tree, .. } => {
                assert_eq!(tree, *fx.w1.0.as_bytes(), "landed in the editor window");
            }
            other => panic!("expected AccessKit key, got {other:?}"),
        },
        other => panic!("expected Resolved, got {other:?}"),
    }

    server.shutdown();
}

/// The session-level scope must never leak between requests: a scoped
/// resolve leaves the tree service unscoped afterwards (the server clears
/// it in the same rendezvous step).
#[test]
fn resolve_scope_is_transient() {
    let fx = MultiWindowFixture::new();
    let mut session = Session::new(MockAdapter::with_multi_window(&fx));
    session.settle(std::time::Instant::now() + Duration::from_secs(5));
    let scope = session.window_scope(&fx.w1.0.to_string()).expect("w1");
    session.set_resolve_scope(scope);
    assert!(session.tree().scope().is_some(), "scope applied");
    session.clear_resolve_scope();
    assert!(session.tree().scope().is_none(), "scope cleared");
}

/// NodeKey stays the window discriminator end-to-end: two same-local-id
/// buttons in different windows carry different identities, so `Action`
/// requests are already window-unique without extra scoping (§16 v0.4
/// "targets per window" — the target only narrows resolution).
#[test]
fn window_identity_is_the_node_key_discriminator() {
    let fx = MultiWindowFixture::new();
    let w1: NodeKey = fx.w1_save_key();
    let w2: NodeKey = fx.w2_save_key();
    assert_ne!(w1, w2);
}
