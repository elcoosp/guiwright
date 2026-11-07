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
use std::time::Duration;

use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{mpsc, oneshot};

use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, ClockV1, ErrorCode, InspectV1, MockV1, NodeKeyV1, PROTO_VERSION,
    RequestV1, ResponseV1, SemActionV1, VideoV1,
};
use guiwright_proto::tcp;
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

/// Where a server handle listens (§7 transports).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// Unix domain socket path (default transport).
    Uds(PathBuf),
    /// TCP host:port (CI containers + other-language clients, M3).
    Tcp(std::net::SocketAddr),
}

/// Handle keeping the server + session thread alive.
pub struct GwpServerHandle {
    cmd_tx: mpsc::Sender<Internal>,
    serve_task: tokio::task::JoinHandle<()>,
    session_thread: std::thread::JoinHandle<()>,
    pub endpoint: Endpoint,
}

impl GwpServerHandle {
    pub fn bridge(&self) -> GwpBridge {
        GwpBridge {
            cmd_tx: self.cmd_tx.clone(),
        }
    }

    /// The bound endpoint (UDS path or TCP socket address). For an
    /// ephemeral `spawn_tcp("127.0.0.1:0")` this is how clients learn the
    /// port.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
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
        if let Endpoint::Uds(path) = &self.endpoint {
            let _ = std::fs::remove_file(path);
        }
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
    GwpServerHandle::spawn_with_seams(path, make_adapter, crate::seams::SeamRegistry::new())
}

impl GwpServerHandle {
    /// Like [`spawn_session_server`], plus the app's boot seam registry
    /// (§14): the hooks `Mock::Call`/`ResetState` may run over the wire.
    pub fn spawn_with_seams<A, F>(
        path: PathBuf,
        make_adapter: F,
        seams: crate::seams::SeamRegistry,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
    {
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        Self::start(listener, Endpoint::Uds(path), make_adapter, seams)
    }

    /// Spawn the remote-mode stack over **TCP** (§7 "TCP for CI
    /// containers" plus M3 other-language clients): same session thread,
    /// same rendezvous discipline, byte-identical GWP frames — only the
    /// socket differs. Pass port `0` for an ephemeral bind; read the
    /// bound address back from [`GwpServerHandle::endpoint`].
    pub fn spawn_tcp<A, F>(
        addr: std::net::SocketAddr,
        make_adapter: F,
        seams: crate::seams::SeamRegistry,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
    {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let bound = listener.local_addr()?;
        let listener = TcpListener::from_std(listener)?;
        Self::start(listener, Endpoint::Tcp(bound), make_adapter, seams)
    }

    /// Shared stack bring-up: session thread + accept loop bridged over
    /// one command queue (§5.1).
    fn start<A, F, L, S>(
        listener: L,
        endpoint: Endpoint,
        make_adapter: F,
        seams: crate::seams::SeamRegistry,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
        L: Send + 'static,
        S: Future<Output = ()> + Send + 'static,
        L: IntoServe<S>,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(64);
        let session_thread = std::thread::Builder::new()
            .name("guiwright-session".into())
            .spawn(move || run_session_thread(cmd_rx, make_adapter(), seams))
            .expect("spawn session thread");

        let bridge_tx = cmd_tx.clone();
        let serve_task = tokio::spawn(async move {
            listener
                .into_serve(move || GwpBridge {
                    cmd_tx: bridge_tx.clone(),
                })
                .await;
        });

        Ok(GwpServerHandle {
            cmd_tx,
            serve_task,
            session_thread,
            endpoint,
        })
    }
}

/// Bridges a transport listener into its `serve` future, so the shared
/// stack bring-up above is transport-blind.
trait IntoServe<S> {
    fn into_serve<H: Handler>(self, make_handler: impl FnMut() -> H + Send + 'static) -> S;
}

impl IntoServe<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>> for UnixListener {
    fn into_serve<H: Handler>(
        self,
        make_handler: impl FnMut() -> H + Send + 'static,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            uds::serve(self, make_handler).await;
        })
    }
}

impl IntoServe<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>> for TcpListener {
    fn into_serve<H: Handler>(
        self,
        make_handler: impl FnMut() -> H + Send + 'static,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            tcp::serve(self, make_handler).await;
        })
    }
}

/// The session thread event loop: drain the command queue, process each
/// request against the session, pump once per batch (frame-boundary
/// rendezvous, §15.4).
fn run_session_thread<A: FrameworkAdapter>(
    mut cmd_rx: mpsc::Receiver<Internal>,
    adapter: A,
    seams: crate::seams::SeamRegistry,
) {
    let mut session = Session::new(adapter);
    // App-boot seams are available to the whole session (§14).
    session.register_seams(&seams);
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
        RequestV1::Clock(cmd) => clock_response(session, cmd),
        RequestV1::Mock(cmd) => mock_response(session, cmd),
        RequestV1::Inspect(cmd) => inspect_response(session, cmd),
        RequestV1::Video(cmd) => video_response(session, cmd),
        RequestV1::Bye => ResponseV1::Bye,
    }
}

/// §7 video domain (§10): recording control. Wired to the session's
/// video service — frames ride the pump loop (§15.4), dedup + encoding
/// happen off the session thread. The client owns the artifact dir
/// (Playwright parity): `VideoOptsV1.dir` overrides the driver default.
fn video_response<A: FrameworkAdapter>(session: &mut Session<A>, cmd: VideoV1) -> ResponseV1 {
    match cmd {
        VideoV1::Start(opts) => {
            let result = match opts.dir.as_deref() {
                Some(dir) => session.video_start_in(std::path::Path::new(dir), opts.fps),
                None => session.video_start(opts.fps),
            };
            match result {
                Ok(()) => ResponseV1::Pong,
                Err(err) => error_response(err),
            }
        }
        VideoV1::Stop => match session.video_stop() {
            Ok(file) => ResponseV1::VideoFile {
                path: file.path.display().to_string(),
                frames: file.frames,
                dropped: file.dropped,
                duration_ms: file.duration_ms,
            },
            Err(err) => error_response(err),
        },
    }
}

/// §7 clock domain (§14 service): millisecond wire vocabulary → session
/// `Duration` surface.
fn clock_response<A: FrameworkAdapter>(session: &mut Session<A>, cmd: ClockV1) -> ResponseV1 {
    use guiwright_core::CoreError;
    let adapter_cmd = match cmd {
        ClockV1::InstallVirtual => crate::adapter::ClockCmd::InstallVirtual,
        ClockV1::FastForward { ms } => {
            crate::adapter::ClockCmd::FastForward(Duration::from_millis(ms))
        }
        ClockV1::RunFor { ms } => crate::adapter::ClockCmd::RunFor(Duration::from_millis(ms)),
        ClockV1::Pause => crate::adapter::ClockCmd::Pause,
        ClockV1::Resume => crate::adapter::ClockCmd::Resume,
        ClockV1::SetTime { ms } => crate::adapter::ClockCmd::SetTime(Duration::from_millis(ms)),
        ClockV1::State => crate::adapter::ClockCmd::Probe,
    };
    match session.clock(adapter_cmd) {
        Ok(report) => ResponseV1::ClockState {
            virtual_time_ms: report.state.virtual_time.map(|t| t.as_millis() as u64),
            paused: report.state.paused,
            steps: report.steps,
        },
        Err(CoreError::Other(msg)) if msg.contains("paused") => ResponseV1::Error {
            code: ErrorCode::Unsupported,
            message: msg,
        },
        Err(err) => error_response(err),
    }
}

/// §7 mock domain (§14 state seams).
fn mock_response<A: FrameworkAdapter>(session: &mut Session<A>, cmd: MockV1) -> ResponseV1 {
    match cmd {
        MockV1::InstallSeam { id, response } => {
            session.set_seam_response(id.clone(), response);
            ResponseV1::MockDone {
                id,
                detail: "seam override installed".into(),
            }
        }
        MockV1::Clear { id } => {
            session.clear_seam(&id);
            ResponseV1::MockDone {
                id,
                detail: "seam override cleared".into(),
            }
        }
        MockV1::ResetState { scope } => match session.reset_state(&scope) {
            Ok(detail) => ResponseV1::MockDone { id: scope, detail },
            Err(err) => error_response(err),
        },
        MockV1::Call { id, scope } => match session.run_seam(&id, &scope) {
            Ok(detail) => ResponseV1::MockDone { id, detail },
            Err(err) => error_response(err),
        },
    }
}

/// §7 inspect domain (§13): remote codegen recording (M4). The
/// session-side recorder is the kit's `CodegenRecorder` (guiwright-inspect)
/// fed by every applied `Action` request; `StopRecording` replies with the
/// generated kit code + strict-mode warnings (§13). The live highlight
/// handles land with the interactive overlay session.
fn inspect_response<A: FrameworkAdapter>(session: &mut Session<A>, cmd: InspectV1) -> ResponseV1 {
    match cmd {
        InspectV1::StartRecording => match session.codegen_start() {
            Ok(()) => ResponseV1::Pong,
            Err(err) => error_response(err),
        },
        InspectV1::StopRecording => match session.codegen_stop() {
            Ok((code, warnings)) => ResponseV1::Codegen { code, warnings },
            Err(err) => error_response(err),
        },
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
            run_session_thread(
                cmd_rx,
                MockAdapter::with_fixture(&Fixture::buttons()),
                crate::seams::SeamRegistry::new(),
            );
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
            run_session_thread(
                cmd_rx,
                MockAdapter::with_fixture(&Fixture::buttons()),
                crate::seams::SeamRegistry::new(),
            );
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
            run_session_thread(
                cmd_rx,
                MockAdapter::with_fixture(&Fixture::buttons()),
                crate::seams::SeamRegistry::new(),
            );
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

    // --- M2: clock & mock domains over the session thread ----------------

    #[tokio::test]
    async fn clock_domain_steps_over_the_session() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(8);
        let handle = std::thread::spawn(move || {
            run_session_thread(
                cmd_rx,
                MockAdapter::with_fixture(&Fixture::buttons()),
                crate::seams::SeamRegistry::new(),
            );
        });

        let resp = send(&cmd_tx, RequestV1::Clock(ClockV1::InstallVirtual)).await;
        match resp {
            ResponseV1::ClockState {
                virtual_time_ms, ..
            } => assert_eq!(virtual_time_ms, Some(0)),
            other => panic!("expected ClockState, got {other:?}"),
        }

        // 50 ms fast-forward = 3 frame steps (16.67 ms each).
        let resp = send(&cmd_tx, RequestV1::Clock(ClockV1::FastForward { ms: 50 })).await;
        match resp {
            ResponseV1::ClockState {
                virtual_time_ms,
                steps,
                ..
            } => {
                assert_eq!(virtual_time_ms, Some(50));
                assert_eq!(steps, 3);
            }
            other => panic!("expected ClockState, got {other:?}"),
        }

        let resp = send(&cmd_tx, RequestV1::Clock(ClockV1::State)).await;
        match resp {
            ResponseV1::ClockState { steps, .. } => assert_eq!(steps, 0, "state probe"),
            other => panic!("expected ClockState, got {other:?}"),
        }

        drop(cmd_tx);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn mock_domain_runs_seams_and_overrides() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(8);
        let handle = std::thread::spawn(move || {
            let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
            adapter = adapter.with_state(AppDb { rows: 7 });
            run_session_thread(cmd_rx, adapter, app_seams());
        });

        // Canned override wins (§7 InstallSeam).
        let resp = send(
            &cmd_tx,
            RequestV1::Mock(MockV1::InstallSeam {
                id: "reset_state".into(),
                response: Some("canned-ok".into()),
            }),
        )
        .await;
        match resp {
            ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "seam override installed"),
            other => panic!("expected MockDone, got {other:?}"),
        }

        let resp = send(
            &cmd_tx,
            RequestV1::Mock(MockV1::ResetState {
                scope: "seed-42".into(),
            }),
        )
        .await;
        match resp {
            ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "canned-ok"),
            other => panic!("expected MockDone, got {other:?}"),
        }

        // Clear → the real hook runs inside the runner context (§14).
        let resp = send(
            &cmd_tx,
            RequestV1::Mock(MockV1::Clear {
                id: "reset_state".into(),
            }),
        )
        .await;
        match resp {
            ResponseV1::MockDone { .. } => {}
            other => panic!("expected MockDone, got {other:?}"),
        }
        let resp = send(
            &cmd_tx,
            RequestV1::Mock(MockV1::ResetState {
                scope: "seed-42".into(),
            }),
        )
        .await;
        match resp {
            ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "db reset to seed-42"),
            other => panic!("expected MockDone, got {other:?}"),
        }

        // Direct Call on a second seam.
        let resp = send(
            &cmd_tx,
            RequestV1::Mock(MockV1::Call {
                id: "seed_users".into(),
                scope: "smoke".into(),
            }),
        )
        .await;
        match resp {
            ResponseV1::MockDone { detail, .. } => assert_eq!(detail, "3 users seeded"),
            other => panic!("expected MockDone, got {other:?}"),
        }

        drop(cmd_tx);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn mock_unknown_seam_reports_clean_error() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Internal>(8);
        let handle = std::thread::spawn(move || {
            run_session_thread(
                cmd_rx,
                MockAdapter::with_fixture(&Fixture::buttons()),
                crate::seams::SeamRegistry::new(),
            );
        });
        let resp = send(
            &cmd_tx,
            RequestV1::Mock(MockV1::Call {
                id: "missing".into(),
                scope: String::new(),
            }),
        )
        .await;
        match resp {
            ResponseV1::Error { message, .. } => {
                assert!(message.contains("no seam registered"), "{message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
        drop(cmd_tx);
        handle.join().unwrap();
    }

    #[derive(Debug, Default)]
    struct AppDb {
        rows: usize,
    }

    fn app_seams() -> crate::seams::SeamRegistry {
        let mut reg = crate::seams::SeamRegistry::new();
        reg.register("reset_state", |ctx, scope| {
            let db = ctx
                .downcast_mut::<AppDb>()
                .ok_or("wrong runner context type")?;
            db.rows = 0;
            Ok(format!("db reset to {scope}"))
        });
        reg.register("seed_users", |ctx, _scope| {
            let db = ctx
                .downcast_mut::<AppDb>()
                .ok_or("wrong runner context type")?;
            db.rows = 3;
            Ok("3 users seeded".into())
        });
        reg
    }
}
