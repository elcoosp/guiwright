//! GWP-over-UDS server (architecture §5.1 remote mode + §7).
//!
//! The tokio-side bridge never touches the adapter: it forwards requests to
//! the session thread over a channel; the session applies each command
//! **inside its next frame** and replies through a oneshot — the exact
//! rendezvous discipline of §5.1 ("commands are queued to the UI thread,
//! applied inside the next frame, then replied").
//!
//! `Resolve` auto-waits server-side (attached within `timeout_ms`, §8.2
//! over the wire); `Action` carries the node handle + generation obtained
//! from a prior `Resolve` and is applied after a generation check (§8.2
//! ABA guard).

use std::future::Future;
use std::path::PathBuf;

use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};

use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, ErrorCode, NodeKeyV1, PROTO_VERSION, RequestV1, ResponseV1, SemActionV1,
};
use guiwright_proto::uds::{self, Handler};

use crate::adapter::{AdapterMeta, FrameworkAdapter, SemAction};
use crate::session::Session;
use guiwright_core::{CoreError, NodeKey, Selector};

/// A command on its way to the session thread (§5.1 rendezvous queue).
pub struct Command {
    request: RequestV1,
    reply: oneshot::Sender<ResponseV1>,
}

/// Internal control messages outside the GWP surface.
enum Internal {
    Gwp(Command),
    /// Session thread exits after draining the queue.
    Shutdown,
}

/// Tokio-side bridge: implements the proto `Handler` by proxying to the
/// session thread.
#[derive(Clone)]
pub struct GwpBridge {
    cmd_tx: mpsc::Sender<Internal>,
}

impl Handler for GwpBridge {
    fn handle(&mut self, request: RequestV1) -> impl Future<Output = ResponseV1> + Send {
        let tx = self.cmd_tx.clone();
        async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            if tx
                .send(Internal::Gwp(Command {
                    request,
                    reply: reply_tx,
                }))
                .await
                .is_err()
            {
                return ResponseV1::Error {
                    code: ErrorCode::Internal,
                    message: "session thread is gone".into(),
                };
            }
            match reply_rx.await {
                Ok(resp) => resp,
                Err(_) => ResponseV1::Error {
                    code: ErrorCode::Internal,
                    message: "session dropped the request".into(),
                },
            }
        }
    }
}

/// Handle keeping the server + session thread alive.
pub struct GwpServerHandle {
    cmd_tx: mpsc::Sender<Internal>,
    serve_task: tokio::task::JoinHandle<()>,
    session_thread: std::thread::JoinHandle<()>,
    pub path: PathBuf,
}

impl GwpServerHandle {
    pub fn bridge(&self) -> GwpBridge {
        GwpBridge {
            cmd_tx: self.cmd_tx.clone(),
        }
    }

    /// Graceful shutdown: signal the session thread, stop the accept loop,
    /// drop the queue, join everything.
    pub fn shutdown(self) {
        // 1. Tell the session thread to exit after draining.
        let _ = self.cmd_tx.try_send(Internal::Shutdown);
        // 2. Stop accepting connections (bridge clones die with the task).
        self.serve_task.abort();
        // 3. Drop our sender so in-flight work can finish.
        drop(self.cmd_tx);
        // 4. Join the session thread (bounded by the queue drain).
        let _ = self.session_thread.join();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Spawn the remote-mode stack for an adapter: a session thread owning the
/// adapter + a tokio UDS server bridging GWP requests (§5.1, §7).
pub fn spawn_session_server<A, F>(
    path: PathBuf,
    make_adapter: F,
) -> Result<GwpServerHandle, GwpError>
where
    A: FrameworkAdapter,
    F: FnOnce() -> A + Send + 'static,
{
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;

    let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(64);
    let session_thread = std::thread::Builder::new()
        .name("guiwright-session".into())
        .spawn(move || run_session_thread(cmd_rx, make_adapter()))
        .expect("spawn session thread");

    let bridge_tx = cmd_tx.clone();
    let serve_task = tokio::spawn(async move {
        uds::serve(listener, move || GwpBridge {
            cmd_tx: bridge_tx.clone(),
        })
        .await;
    });

    Ok(GwpServerHandle {
        cmd_tx,
        serve_task,
        session_thread,
        path,
    })
}

/// The session thread event loop: drain the command queue, process each
/// request against the session, pump once per batch (frame-boundary
/// rendezvous, §15.4).
fn run_session_thread<A: FrameworkAdapter>(mut cmd_rx: mpsc::Receiver<Internal>, adapter: A) {
    let mut session = Session::new(adapter);
    // Attach: settle the app so the initial tree is visible (§6.3
    // "app.wait_for_settle()").
    session.settle(std::time::Instant::now() + std::time::Duration::from_secs(5));

    while let Some(cmd) = cmd_rx.blocking_recv() {
        match cmd {
            Internal::Shutdown => break,
            Internal::Gwp(Command { request, reply }) => {
                let response = handle_request(&mut session, request);
                let _ = reply.send(response);
                // One pump per batch: queued commands apply at the next
                // frame boundary (§15.4).
                session.pump();
            }
        }
    }
}

/// Process a single GWP request against the session.
fn handle_request<A: FrameworkAdapter>(session: &mut Session<A>, request: RequestV1) -> ResponseV1 {
    match request {
        RequestV1::Hello { .. } => {
            let meta = session.adapter().meta();
            ResponseV1::Welcome {
                proto_version: PROTO_VERSION,
                session_id: format!("{}-{}", meta.name, std::process::id()),
                tree_seq: session.tree_seq(),
                frame_seq: session.frame_seq(),
                adapter: adapter_info(meta),
            }
        }
        RequestV1::Ping => ResponseV1::Pong,
        RequestV1::TreeSnapshot => ResponseV1::TreeSnapshot {
            tree_seq: session.tree_seq(),
            node_count: session.tree().node_count(),
        },
        RequestV1::Resolve {
            selector,
            timeout_ms,
        } => {
            let sel = Selector::from(selector);
            let result = match session.resolve(&sel) {
                Ok(resolved) => Ok(Some(resolved)),
                Err(err) if matches!(err, CoreError::NoMatch { .. }) && timeout_ms > 0 => {
                    // Auto-wait over the wire: attached within the timeout
                    // (§8.2). Strict violations fail immediately.
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
                    session
                        .wait_for(
                            &sel,
                            vec![guiwright_core::actionability::Check::Attached],
                            deadline,
                        )
                        .map(|wait| wait.into_node().ok())
                }
                Err(err) => Err(err),
            };
            match result {
                Ok(Some(resolved)) => ResponseV1::Resolved {
                    node: NodeKeyV1::from(resolved.key),
                    generation: resolved.generation,
                    rect: resolved.bounds.map(rect_to_array),
                },
                Ok(None) => ResponseV1::Error {
                    code: ErrorCode::Internal,
                    message: "resolve satisfied by absence".into(),
                },
                Err(err) => error_response(err),
            }
        }
        RequestV1::Action {
            node,
            generation,
            action,
            force,
            timeout_ms,
        } => {
            let key: NodeKey = node.into();
            let sem = convert_action(&action);
            // Generation guard at the boundary (§8.2): the client's handle
            // must still be live; a recycled id cannot slip through.
            let resolved = guiwright_core::Resolved {
                key,
                generation,
                bounds: None,
            };
            if let Err(err) = session.tree().check_generation(&resolved).and_then(|()| {
                session
                    .tree()
                    .node(&resolved.key)
                    .map(|_| ())
                    .ok_or_else(|| CoreError::NoMatch {
                        selector: resolved.key.describe(),
                        last: Box::new(session.tree().summary()),
                    })
            }) {
                return error_response(err);
            }
            match session.apply_action(resolved.key, sem, force, timeout_ms) {
                Ok(outcome) => ResponseV1::ActionDone {
                    frame_seq: outcome.applied_at_frame,
                    detail: outcome.detail,
                },
                Err(err) => error_response(err),
            }
        }
        RequestV1::Input { .. } => ResponseV1::Error {
            code: ErrorCode::Unsupported,
            message: "raw input over GWP lands with the gesture synthesis milestone (§9)".into(),
        },
        RequestV1::Bye => ResponseV1::Bye,
    }
}

fn error_response(err: CoreError) -> ResponseV1 {
    let (code, message) = match &err {
        CoreError::NoMatch { .. } | CoreError::Ambiguous { .. } => {
            (ErrorCode::StrictModeViolation, err.to_string())
        }
        CoreError::Timeout { .. } => (ErrorCode::Timeout, err.to_string()),
        CoreError::StaleNode { .. } => (ErrorCode::StaleGeneration, err.to_string()),
        CoreError::Unsupported(what) => (ErrorCode::Unsupported, (*what).to_string()),
        CoreError::Other(msg) => (ErrorCode::Internal, msg.clone()),
        _ => (ErrorCode::Internal, err.to_string()),
    };
    ResponseV1::Error { code, message }
}

fn convert_action(action: &SemActionV1) -> SemAction {
    match action {
        SemActionV1::Click => SemAction::Click,
        SemActionV1::DoubleClick => SemAction::DoubleClick,
        SemActionV1::SetValue { text } => SemAction::SetValue { text: text.clone() },
        SemActionV1::Focus => SemAction::Focus,
        SemActionV1::Blur => SemAction::Blur,
        SemActionV1::Hover => SemAction::Hover,
        SemActionV1::Press { key, modifiers } => SemAction::Press {
            key: key.clone(),
            modifiers: modifiers.clone(),
        },
        SemActionV1::Scroll { dx, dy } => SemAction::Scroll { dx: *dx, dy: *dy },
        SemActionV1::Check => SemAction::Check,
        SemActionV1::Uncheck => SemAction::Uncheck,
    }
}

fn adapter_info(meta: &AdapterMeta) -> AdapterInfoV1 {
    AdapterInfoV1 {
        name: meta.name.to_string(),
        framework: meta.framework.to_string(),
        framework_version: meta.framework_version.clone(),
        caps: CapsV1 {
            tree_update: meta.caps.tree_update,
            logical_snapshot: meta.caps.logical_snapshot,
            input: meta.caps.input,
            last_frame: meta.caps.last_frame,
            render_frame: meta.caps.render_frame,
            clock: meta.caps.clock,
        },
    }
}

fn rect_to_array(rect: accesskit::Rect) -> [f64; 4] {
    [rect.x0, rect.y0, rect.x1, rect.y1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAdapter;
    use guiwright_core::test_util::Fixture;
    use guiwright_proto::schema::SelectorV1;
    use tokio::sync::oneshot;

    async fn send(cmd_tx: &mpsc::Sender<Internal>, request: RequestV1) -> ResponseV1 {
        let (tx, rx) = oneshot::channel();
        cmd_tx
            .send(Internal::Gwp(Command { request, reply: tx }))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    #[tokio::test]
    async fn session_thread_answers_handshake_and_snapshot() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(8);
        let handle = std::thread::spawn(move || {
            run_session_thread(cmd_rx, MockAdapter::with_fixture(&Fixture::buttons()));
        });

        let resp = send(
            &cmd_tx,
            RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: "test".into(),
            },
        )
        .await;
        match resp {
            ResponseV1::Welcome {
                adapter, tree_seq, ..
            } => {
                assert_eq!(adapter.name, "mock");
                assert!(adapter.caps.tree_update);
                assert!(tree_seq >= 1, "settle integrated the initial tree");
            }
            other => panic!("expected Welcome, got {other:?}"),
        }

        let resp = send(&cmd_tx, RequestV1::TreeSnapshot).await;
        match resp {
            ResponseV1::TreeSnapshot { node_count, .. } => assert_eq!(node_count, 8),
            other => panic!("expected snapshot, got {other:?}"),
        }

        drop(cmd_tx);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn resolve_and_action_roundtrip() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(8);
        let handle = std::thread::spawn(move || {
            run_session_thread(cmd_rx, MockAdapter::with_fixture(&Fixture::buttons()));
        });

        // Resolve the Save button (fuzzy role+name).
        let resp = send(
            &cmd_tx,
            RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(guiwright_proto::schema::TextMatchV1 {
                        text: "Save".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                timeout_ms: 1000,
            },
        )
        .await;
        let (node, generation) = match resp {
            ResponseV1::Resolved {
                node, generation, ..
            } => (node, generation),
            other => panic!("expected Resolved, got {other:?}"),
        };
        assert!(generation == 0);

        // Act on it.
        let resp = send(
            &cmd_tx,
            RequestV1::Action {
                node,
                generation,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 1000,
            },
        )
        .await;
        match resp {
            ResponseV1::ActionDone { detail, .. } => assert_eq!(detail, "clicked"),
            other => panic!("expected ActionDone, got {other:?}"),
        }

        drop(cmd_tx);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn stale_generation_is_rejected() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(8);
        let handle = std::thread::spawn(move || {
            run_session_thread(cmd_rx, MockAdapter::with_fixture(&Fixture::buttons()));
        });

        let resp = send(
            &cmd_tx,
            RequestV1::Action {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 2,
                },
                generation: 99,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 100,
            },
        )
        .await;
        match resp {
            ResponseV1::Error { code, .. } => assert_eq!(code, ErrorCode::StaleGeneration),
            other => panic!("expected Error, got {other:?}"),
        }

        drop(cmd_tx);
        handle.join().unwrap();
    }
}
