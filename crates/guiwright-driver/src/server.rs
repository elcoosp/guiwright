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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{mpsc, oneshot};
use tracing_subscriber::layer::SubscriberExt;

use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, ClockV1, ErrorCode, EventKindV1, EventV1, GwpEvent, InspectV1, MockV1,
    NodeKeyV1, PROTO_VERSION, RequestV1, ResponseV1, ScreenshotV1, SelectorV1, SemActionV1,
    SnapshotV1, SubscribeV1, VideoV1, WindowInfoV1, WindowV1,
};
use guiwright_proto::tcp;
use guiwright_proto::uds::{self, Handler};
use guiwright_proto::ws;

use crate::adapter::{AdapterMeta, FrameworkAdapter, SemAction};
use crate::clock::FRAME_STEP;
use crate::events::{EventHub, Subscriber};
use crate::log_tail::{LOG_TAIL_CAPACITY, LogTail, LogTapLayer};
use crate::session::Session;
use guiwright_core::{CoreError, NodeKey, Selector};

/// A command on its way to the session thread (§5.1 rendezvous queue).
/// `window` is the sending connection's active window target (§16 v0.4,
/// M6): `None` = unscoped (the whole tree). Per-connection state lives on
/// the bridge — the shared session stays a single honest interpreter
/// (§11 "one driver session serves all connections").
pub struct Command {
    request: RequestV1,
    window: Option<String>,
    /// The sending connection's event identity + queue (§7, M12),
    /// attached only by `Subscribe` requests: the session thread
    /// registers it into the hub (On) or drops the subscription (Off).
    events: Option<(u64, mpsc::Sender<GwpEvent>)>,
    reply: oneshot::Sender<ResponseV1>,
}

/// Internal control messages outside the GWP surface.
enum Internal {
    Gwp(Command),
    /// Session thread exits after draining the queue.
    Shutdown,
}

/// Remote-mode spawn knobs (everything the stack needs beyond the
/// adapter). The default is the pre-log-tail shape: no seams, no tap —
/// a session spawned with it keeps the byte-exact conversation **and**
/// the process-wide `tracing` behavior.
#[derive(Default)]
pub struct ServerOptions {
    /// App-boot seams (§14): the hooks `Mock::Call`/`ResetState` may run
    /// over the wire.
    pub seams: crate::seams::SeamRegistry,
    /// The per-session `tracing` tap (the log-tail milestone): while
    /// enabled, the session thread's records (`DEBUG` and above) stream
    /// to `EventKindV1::Log` subscribers as `EventV1::Log` frames. Capture
    /// is a thread-local dispatcher on the session thread — other threads
    /// keep the process subscriber, and the tail is a bounded live tap
    /// (§15.6), never a replay buffer.
    pub log_tail: bool,
}

impl std::fmt::Debug for ServerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerOptions")
            .field("seams", &self.seams)
            .field("log_tail", &self.log_tail)
            .finish()
    }
}

/// Tokio-side bridge: implements the proto `Handler` by proxying to the
/// session thread. One instance per connection (the serve loop calls
/// `make_handler` per accepted socket); the active window target is
/// **per-connection** (§16 v0.4) — `Window::Use` on one connection never
/// re-aims another — and so is the event subscription (§7, M12).
pub struct GwpBridge {
    cmd_tx: mpsc::Sender<Internal>,
    /// This connection's active window target handle. Shared with the
    /// reply-processing future via `Arc<Mutex<..>>` (the `Handler` future
    /// cannot borrow `self`).
    window: Arc<std::sync::Mutex<Option<String>>>,
    /// This connection's stable event identity (the hub's key).
    conn_id: u64,
    /// This connection's event queue (§7, M12): the receiver half is
    /// handed to the connection loop once (`event_stream`); the sender
    /// travels through `Subscribe` commands into the session hub.
    event_tx: mpsc::Sender<GwpEvent>,
    event_rx: Option<mpsc::Receiver<GwpEvent>>,
}

impl GwpBridge {
    fn new(cmd_tx: mpsc::Sender<Internal>, conn_id: u64) -> Self {
        let (event_tx, event_rx) = mpsc::channel(guiwright_proto::conn::EVENT_QUEUE_CAPACITY);
        Self {
            cmd_tx,
            window: Arc::new(std::sync::Mutex::new(None)),
            conn_id,
            event_tx,
            event_rx: Some(event_rx),
        }
    }
}

impl Handler for GwpBridge {
    fn handle(&mut self, request: RequestV1) -> impl Future<Output = ResponseV1> + Send {
        let tx = self.cmd_tx.clone();
        let window = self.window.clone();
        let conn_id = self.conn_id;
        // `Subscribe` carries this connection's event queue to the session
        // hub (§7, M12); every other request is event-blind.
        let events = match &request {
            RequestV1::Subscribe(_) => Some((conn_id, self.event_tx.clone())),
            _ => None,
        };
        // `Window::Use` is the only request that mutates this connection's
        // target — after the session validates the handle (the reply is
        // `WindowUsed`).
        let use_target = match &request {
            RequestV1::Window(WindowV1::Use { handle }) => Some(handle.clone()),
            _ => None,
        };
        async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            // Stamp the active target onto every non-Window request: the
            // session scopes resolution to it (§16 v0.4).
            let stamped = if use_target.is_some() {
                None
            } else {
                window.lock().ok().and_then(|guard| guard.clone())
            };
            if tx
                .send(Internal::Gwp(Command {
                    request,
                    window: stamped,
                    events,
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
            let response = match reply_rx.await {
                Ok(resp) => resp,
                Err(_) => ResponseV1::Error {
                    code: ErrorCode::Internal,
                    message: "session dropped the request".into(),
                },
            };
            if let Some(handle) = use_target {
                if matches!(response, ResponseV1::WindowUsed { .. }) {
                    if let Ok(mut guard) = window.lock() {
                        *guard = Some(handle);
                    }
                }
            }
            response
        }
    }

    fn event_stream(&mut self) -> Option<mpsc::Receiver<GwpEvent>> {
        self.event_rx.take()
    }
}

/// Where a server handle listens (§7 transports).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// Unix domain socket path (default transport).
    Uds(PathBuf),
    /// TCP host:port (CI containers + other-language clients, M3).
    Tcp(std::net::SocketAddr),
    /// WebSocket host:port — the viewer live-mode door (M15), RFC 6455
    /// upgrade on plain TCP, then the byte-identical GWP conversation.
    Ws(std::net::SocketAddr),
}

/// Handle keeping the server + session thread alive.
pub struct GwpServerHandle {
    cmd_tx: mpsc::Sender<Internal>,
    serve_task: tokio::task::JoinHandle<()>,
    session_thread: std::thread::JoinHandle<()>,
    pub endpoint: Endpoint,
}

/// Per-server connection id counter: the hub keys subscribers by it
/// (stable across re-subscribes on one connection).
static CONN_IDS: AtomicU64 = AtomicU64::new(1);

fn next_conn_id() -> u64 {
    CONN_IDS.fetch_add(1, Ordering::Relaxed)
}

impl GwpServerHandle {
    pub fn bridge(&self) -> GwpBridge {
        GwpBridge::new(self.cmd_tx.clone(), next_conn_id())
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
    spawn_session_server_with(path, make_adapter, ServerOptions::default())
}

/// Like [`spawn_session_server`], with the full [`ServerOptions`] knobs —
/// the boot seams (§14) and the log tail (the log-tail milestone).
pub fn spawn_session_server_with<A, F>(
    path: PathBuf,
    make_adapter: F,
    options: ServerOptions,
) -> Result<GwpServerHandle, GwpError>
where
    A: FrameworkAdapter,
    F: FnOnce() -> A + Send + 'static,
{
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    GwpServerHandle::start(listener, Endpoint::Uds(path), make_adapter, options)
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
        spawn_session_server_with(
            path,
            make_adapter,
            ServerOptions {
                seams,
                ..ServerOptions::default()
            },
        )
    }

    /// Like [`spawn_session_server_with`], with the full [`ServerOptions`]
    /// (seams + the log tail) on the UDS door.
    pub fn spawn_with_options<A, F>(
        path: PathBuf,
        make_adapter: F,
        options: ServerOptions,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
    {
        spawn_session_server_with(path, make_adapter, options)
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
        GwpServerHandle::spawn_tcp_with(
            addr,
            make_adapter,
            ServerOptions {
                seams,
                ..ServerOptions::default()
            },
        )
    }

    /// Like [`spawn_tcp`], with the full [`ServerOptions`] knobs.
    pub fn spawn_tcp_with<A, F>(
        addr: std::net::SocketAddr,
        make_adapter: F,
        options: ServerOptions,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
    {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let bound = listener.local_addr()?;
        let listener = TcpListener::from_std(listener)?;
        GwpServerHandle::start(listener, Endpoint::Tcp(bound), make_adapter, options)
    }

    /// Spawn the remote-mode stack over **WebSocket** (§7's third door,
    /// M15): the RFC 6455 upgrade on plain TCP ([`guiwright_proto::ws`]),
    /// then the byte-identical GWP conversation on the same session
    /// thread. This is the server `connectWs` (the TypeScript client, M3)
    /// has been waiting for — pass port `0` for an ephemeral bind and
    /// read the bound address back from [`GwpServerHandle::endpoint`].
    pub fn spawn_ws<A, F>(
        addr: std::net::SocketAddr,
        make_adapter: F,
        seams: crate::seams::SeamRegistry,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
    {
        GwpServerHandle::spawn_ws_with(
            addr,
            make_adapter,
            ServerOptions {
                seams,
                ..ServerOptions::default()
            },
        )
    }

    /// Like [`spawn_ws`], with the full [`ServerOptions`] knobs.
    pub fn spawn_ws_with<A, F>(
        addr: std::net::SocketAddr,
        make_adapter: F,
        options: ServerOptions,
    ) -> Result<GwpServerHandle, GwpError>
    where
        A: FrameworkAdapter,
        F: FnOnce() -> A + Send + 'static,
    {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let bound = listener.local_addr()?;
        let listener = TcpListener::from_std(listener)?;
        GwpServerHandle::start(
            WsListener(listener),
            Endpoint::Ws(bound),
            make_adapter,
            options,
        )
    }

    /// Shared stack bring-up: session thread + accept loop bridged over
    /// one command queue (§5.1).
    fn start<A, F, L, S>(
        listener: L,
        endpoint: Endpoint,
        make_adapter: F,
        options: ServerOptions,
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
            .spawn(move || run_session_thread(cmd_rx, make_adapter(), options))
            .expect("spawn session thread");

        let bridge_tx = cmd_tx.clone();
        let serve_task = tokio::spawn(async move {
            listener
                .into_serve(move || GwpBridge::new(bridge_tx.clone(), next_conn_id()))
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

/// Newtype so the WebSocket door reuses the transport-blind
/// [`GwpServerHandle::start`]: each accepted TCP stream takes the RFC
/// 6455 upgrade ([`ws::accept`]) and then hands the session to the same
/// shared connection loop the UDS and TCP doors run.
struct WsListener(TcpListener);

impl IntoServe<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>> for WsListener {
    fn into_serve<H: Handler>(
        self,
        make_handler: impl FnMut() -> H + Send + 'static,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut make_handler = make_handler;
            loop {
                match self.0.accept().await {
                    Ok((stream, _addr)) => {
                        let handler = make_handler();
                        tokio::spawn(async move {
                            match ws::accept(stream).await {
                                Ok(ws) => {
                                    if let Err(err) =
                                        guiwright_proto::conn::run_connection(ws, handler).await
                                    {
                                        tracing::debug!(%err, "ws connection closed");
                                    }
                                }
                                Err(err) => {
                                    tracing::debug!(%err, "ws upgrade rejected");
                                }
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!(%err, "ws accept failed");
                        return;
                    }
                }
            }
        })
    }
}

/// The session thread event loop: drain the command queue, process each
/// request against the session, pump once per batch (frame-boundary
/// rendezvous, §15.4) — and, while anyone is subscribed (§7, M12), pump at
/// frame cadence so pushed events flow even with no request in flight.
///
/// The thread hosts its own current-thread runtime for exactly one reason:
/// the idle tick. Sessions without subscribers block on the command queue
/// like the pre-M12 thread (the tick branch never arms); with subscribers
/// the timer fires once per frame step — event-driven work, not a busy
/// poll.
fn run_session_thread<A: FrameworkAdapter>(
    cmd_rx: mpsc::Receiver<Internal>,
    adapter: A,
    options: ServerOptions,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("session thread runtime");
    // The per-session tracing tap (the log-tail milestone): a thread-local
    // dispatcher whose only layer feeds the tail. Other threads keep the
    // process subscriber; the guard lives exactly as long as the thread.
    let (tail, _tap_guard) = if options.log_tail {
        let (tail, tap) = LogTail::new(LOG_TAIL_CAPACITY);
        let guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(LogTapLayer::new(tap)),
        );
        (Some(tail), Some(guard))
    } else {
        (None, None)
    };
    runtime.block_on(session_loop(cmd_rx, adapter, options.seams, tail));
}

/// The async half of the session thread (see [`run_session_thread`]).
async fn session_loop<A: FrameworkAdapter>(
    mut cmd_rx: mpsc::Receiver<Internal>,
    adapter: A,
    seams: crate::seams::SeamRegistry,
    tail: Option<LogTail>,
) {
    let mut session = Session::new(adapter);
    // App-boot seams are available to the whole session (§14).
    session.register_seams(&seams);
    // Attach: settle the app so the initial tree is visible (§6.3
    // "app.wait_for_settle()").
    session.settle(std::time::Instant::now() + std::time::Duration::from_secs(5));

    // The session thread owns the event hub (§7, M12): subscriptions
    // arrive through the command queue, publications happen at the pump
    // sites — no locking, the same single-owner discipline as `Session`.
    let mut hub = EventHub::default();
    // What subscribers have already been told about: events are
    // "tree/frame is at seq N now" notifications, so coalescing a burst of
    // pumps into one event per batch is the design (seq numbers expose
    // any gap a slow client missed, §15.6).
    let mut published_tree = session.tree_seq();
    let mut published_frame = session.frame_seq();

    loop {
        // Idle tick: armed only while subscribers exist (§7, M12).
        let tick = async {
            if hub.is_empty() {
                std::future::pending::<()>().await
            } else {
                tokio::time::sleep(FRAME_STEP).await
            }
        };
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    None | Some(Internal::Shutdown) => break,
                    Some(Internal::Gwp(Command {
                        request,
                        window,
                        events,
                        reply,
                    })) => {
                        let response = handle_request(
                            &mut session,
                            request,
                            window,
                            &mut hub,
                            events,
                            tail.is_some(),
                        );
                        let _ = reply.send(response);
                        // One pump per batch: queued commands apply at the next
                        // frame boundary (§15.4).
                        session.pump();
                        publish_if_advanced(
                            &mut session,
                            &mut hub,
                            &mut published_tree,
                            &mut published_frame,
                            tail.as_ref(),
                        );
                    }
                }
            }
            _ = tick => {
                // Frame cadence with subscribers: the app advances, and
                // queued diffs/frames become events without any request.
                session.pump();
                publish_if_advanced(
                    &mut session,
                    &mut hub,
                    &mut published_tree,
                    &mut published_frame,
                    tail.as_ref(),
                );
            }
        }
    }
}

/// Publish the seq advancement since the last publish point (§7, M12):
/// one coalesced `TreeChanged` / `FramePresented` per batch, the honest
/// notification shape (a slow client misses intermediate seqs by design —
/// the seq numbers say exactly where it stands). A session with no
/// subscribers only advances the markers, never allocates events.
///
/// The log tail (the log-tail milestone) drains at the same points —
/// unconditionally, so records never sit waiting for a subscriber to be
/// replayed to — and its records ride the wire only while a subscriber
/// asked for [`EventKindV1::Log`].
fn publish_if_advanced<A: FrameworkAdapter>(
    session: &mut Session<A>,
    hub: &mut EventHub,
    published_tree: &mut u64,
    published_frame: &mut u64,
    tail: Option<&LogTail>,
) {
    let tree_seq = session.tree_seq();
    let frame_seq = session.frame_seq();
    // The tail is a live tap, not a replay buffer: drain first, publish
    // only to the kinds a subscriber named.
    let log_records = tail.map_or(Vec::new(), LogTail::drain);
    if hub.is_empty() {
        *published_tree = tree_seq;
        *published_frame = frame_seq;
        return;
    }
    let mut events = Vec::new();
    if tree_seq > *published_tree {
        events.push(GwpEvent::V1(EventV1::TreeChanged { seq: tree_seq }));
    }
    if frame_seq > *published_frame {
        if let Some(stamp) = session.last_presented() {
            events.push(GwpEvent::V1(EventV1::FramePresented {
                seq: stamp.seq,
                hash: stamp.hash,
            }));
        }
    }
    if !log_records.is_empty() && hub.wants(EventKindV1::Log) {
        events.extend(
            log_records
                .into_iter()
                .map(|record| GwpEvent::V1(EventV1::Log { record })),
        );
    }
    *published_tree = tree_seq;
    *published_frame = frame_seq;
    if !events.is_empty() {
        hub.publish(&events);
    }
}

/// Process a single GWP request against the session. `window` is the
/// sending connection's active target (§16 v0.4): resolution-only scoping —
/// node identities are already window-unique, actions/clock/mock/inspect/
/// video are session-wide by design. `events` is the sending connection's
/// event identity + queue, attached by `Subscribe` requests only (§7, M12).
/// `tail_enabled` is whether this session carries a log tail (the
/// log-tail milestone) — the `Log` kind's capability answer.
fn handle_request<A: FrameworkAdapter>(
    session: &mut Session<A>,
    request: RequestV1,
    window: Option<String>,
    hub: &mut EventHub,
    events: Option<(u64, mpsc::Sender<GwpEvent>)>,
    tail_enabled: bool,
) -> ResponseV1 {
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
        RequestV1::Subscribe(SubscribeV1::On { kinds }) => match events {
            Some((conn_id, tx)) => subscribe_on(hub, conn_id, tx, kinds, tail_enabled),
            None => ResponseV1::Error {
                code: ErrorCode::Internal,
                message: "subscribe request arrived without an event channel".into(),
            },
        },
        RequestV1::Subscribe(SubscribeV1::Off) => {
            if let Some((conn_id, _tx)) = events {
                hub.unsubscribe(conn_id);
                tracing::debug!(subscriber = conn_id, "event subscription dropped");
            }
            ResponseV1::Subscribed { kinds: Vec::new() }
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
            // Window scope (§16 v0.4): the connection's active target
            // narrows the resolution surface. Unknown/closed targets fail
            // before any waiting happens.
            let scope = match window.as_deref() {
                Some(handle) if !handle.is_empty() => match session.window_scope(handle) {
                    Ok(scope) => scope,
                    Err(err) => return error_response(err),
                },
                _ => None,
            };
            session.set_resolve_scope(scope);
            let result = resolve_result(session, selector, timeout_ms);
            session.clear_resolve_scope();
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
        RequestV1::Input { events } => {
            // Raw input over the wire (§9, M7): the connection's active
            // window target steers the coordinates (the M6 handoff —
            // "the gesture milestone will take the target window from
            // the connection's active handle"); an unscoped connection
            // targets the session's default window. The batch applies
            // inside the next frame (one rendezvous pump).
            match session.send_input(
                events.into_iter().map(Into::into).collect(),
                window.as_deref(),
            ) {
                Ok(frame_seq) => ResponseV1::InputQueued { frame_seq },
                Err(err) => error_response(err),
            }
        }
        RequestV1::Gesture(gesture) => {
            // Gesture synthesis (§9, M7): the session synthesizes the raw
            // batch from its adapter's pacing table and injects it through
            // the same per-window path as `Input`.
            match session.perform_gesture(gesture.into(), window.as_deref()) {
                Ok(outcome) => ResponseV1::GestureDone {
                    frame_seq: outcome.applied_at_frame,
                    detail: outcome.detail,
                },
                Err(err) => error_response(err),
            }
        }
        RequestV1::Clock(cmd) => clock_response(session, cmd),
        RequestV1::Mock(cmd) => mock_response(session, cmd),
        RequestV1::Inspect(cmd) => inspect_response(session, cmd),
        RequestV1::Video(cmd) => video_response(session, cmd),
        RequestV1::Window(cmd) => window_response(session, cmd),
        RequestV1::Screenshot(cmd) => screenshot_response(session, cmd, window.as_deref()),
        RequestV1::Snapshot(cmd) => tree_dump_response(session, cmd, window),
        RequestV1::Bye => ResponseV1::Bye,
    }
}

/// §7 render domain (§10 capture, M10): window shots render the frame
/// once; element shots resolve first (auto-wait within `timeout_ms`, the
/// same `Resolve` arm discipline, honoring the connection's window scope,
/// §16 v0.4) and crop by the resolved bounds.
fn screenshot_response<A: FrameworkAdapter>(
    session: &mut Session<A>,
    cmd: ScreenshotV1,
    window: Option<&str>,
) -> ResponseV1 {
    match cmd {
        ScreenshotV1::Window => match session.screenshot_window() {
            Ok(shot) => shot_response(shot),
            Err(err) => error_response(err),
        },
        ScreenshotV1::Element {
            selector,
            timeout_ms,
        } => {
            // Window scope (§16 v0.4): element shots aim at the
            // connection's active target, exactly like `Resolve`.
            let scope = match window {
                Some(handle) if !handle.is_empty() => match session.window_scope(handle) {
                    Ok(scope) => scope,
                    Err(err) => return error_response(err),
                },
                _ => None,
            };
            session.set_resolve_scope(scope);
            let result = resolve_result(session, selector, timeout_ms).and_then(|resolved| {
                let resolved = resolved
                    .ok_or_else(|| CoreError::other("resolve satisfied by absence".to_string()))?;
                session.screenshot_element(&resolved.key)
            });
            session.clear_resolve_scope();
            match result {
                Ok(shot) => shot_response(shot),
                Err(err) => error_response(err),
            }
        }
    }
}

/// The `Shot` reply shape (§7): session frame + pixel dimensions + PNG.
fn shot_response(shot: crate::session::WireShot) -> ResponseV1 {
    ResponseV1::Shot {
        frame_seq: shot.frame_seq,
        width: shot.width,
        height: shot.height,
        png: shot.png,
    }
}

/// §7 tree-dump domain (M11): the whole logical tree as typed rows —
/// session-wide by design (the resolve scope narrows locators, not
/// whole-tree reads).
fn tree_dump_response<A: FrameworkAdapter>(
    session: &mut Session<A>,
    cmd: SnapshotV1,
    window: Option<String>,
) -> ResponseV1 {
    match cmd {
        SnapshotV1::Full => {
            let dump = session.tree_dump();
            dump_response(dump)
        }
        SnapshotV1::Filtered {
            selector,
            timeout_ms,
        } => {
            // M13 (§7 tree domain): resolve (auto-wait, §8.2) under the
            // connection's window scope — the same resolve the `Resolve`
            // and `Screenshot::Element` arms run — then dump only the
            // match's subtree. A dead selector expires as `Timeout` like
            // every other auto-wait; a non-matching one is a strict-mode
            // violation, exactly like `Resolve`.
            let scope = match window.as_deref() {
                Some(handle) if !handle.is_empty() => match session.window_scope(handle) {
                    Ok(scope) => scope,
                    Err(err) => return error_response(err),
                },
                _ => None,
            };
            session.set_resolve_scope(scope);
            let resolved = resolve_result(session, selector, timeout_ms);
            session.clear_resolve_scope();
            match resolved {
                Ok(Some(resolved)) => match session.tree_dump_under(&resolved.key) {
                    Ok(dump) => dump_response(dump),
                    Err(err) => error_response(err),
                },
                Ok(None) => ResponseV1::Error {
                    code: ErrorCode::Internal,
                    message: "resolve succeeded without a node".into(),
                },
                Err(err) => error_response(err),
            }
        }
    }
}

/// The wire shape of a domain dump (the M11 row vocabulary).
fn dump_response(dump: crate::session::TreeDump) -> ResponseV1 {
    ResponseV1::TreeDump {
        tree_seq: dump.tree_seq,
        node_count: dump.node_count,
        focus: dump.focus.map(Into::into),
        nodes: dump
            .nodes
            .into_iter()
            .map(|node| guiwright_proto::schema::SnapshotNodeV1 {
                key: node.key.into(),
                role: node.role,
                name: node.name,
                value: node.value,
                bounds: node.bounds,
                children: node.children,
            })
            .collect(),
    }
}

/// The resolve/auto-wait core of the `Resolve` arm (§8.2 over the wire).
/// The caller has already applied the connection's window scope (§16 v0.4):
/// both the immediate resolve and the waiter surface honor it.
fn resolve_result<A: FrameworkAdapter>(
    session: &mut Session<A>,
    selector: SelectorV1,
    timeout_ms: u64,
) -> Result<Option<guiwright_core::Resolved>, CoreError> {
    let sel = Selector::from(selector);
    match session.resolve(&sel) {
        Ok(resolved) => Ok(Some(resolved)),
        Err(err) if matches!(err, CoreError::NoMatch { .. }) && timeout_ms > 0 => {
            // Auto-wait over the wire: attached within the timeout
            // (§8.2). Strict violations fail immediately.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
            session
                .wait_for(
                    &sel,
                    vec![guiwright_core::actionability::Check::Attached],
                    deadline,
                )
                .map(|wait| wait.into_node().ok())
        }
        Err(err) => Err(err),
    }
}

/// §7 `Window` domain (§16 v0.4 multi-window, M6): list targets, validate
/// `Use`. The bridge records the active target when it sees `WindowUsed` —
/// the session stays stateless about which connection asked.
fn window_response<A: FrameworkAdapter>(session: &mut Session<A>, cmd: WindowV1) -> ResponseV1 {
    match cmd {
        WindowV1::List => ResponseV1::Windows {
            windows: session
                .windows()
                .into_iter()
                .map(|w| WindowInfoV1 {
                    handle: w.handle,
                    title: w.title,
                    focused: w.focused,
                })
                .collect(),
        },
        WindowV1::Use { handle } => match session.window_scope(&handle) {
            Ok(_) => ResponseV1::WindowUsed { handle },
            Err(err) => error_response(err),
        },
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

/// §7 inspect domain (§13): remote codegen recording (M4) + the live
/// overlay/highlight session (M8). The session-side recorder is the
/// kit's `CodegenRecorder` (guiwright-inspect) fed by every applied
/// `Action` request; `StopRecording` replies with the generated kit code
/// and strict-mode warnings (§13). The overlay half composites through
/// the adapter's `show_overlay` hook; `Highlight` replies `Pong` — the
/// client renders the highlighted node's facts from its own resolution.
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
        InspectV1::EnableOverlay => match session.inspect_enable_overlay() {
            Ok(()) => ResponseV1::Pong,
            Err(err) => error_response(err),
        },
        InspectV1::DisableOverlay => match session.inspect_disable_overlay() {
            Ok(()) => ResponseV1::Pong,
            Err(err) => error_response(err),
        },
        InspectV1::Highlight { node } => {
            match session.inspect_highlight_key(node.map(Into::into)) {
                Ok(_) => ResponseV1::Pong,
                Err(err) => error_response(err),
            }
        }
    }
}

/// Register a subscription (§7, M12): dedupe the kind list, enforce the
/// all-or-nothing capability contract (`Log` rides the session's log tail
/// — a session spawned without it still answers `Unsupported`, never a
/// silently ignored filter), then hand the connection's queue to the hub.
fn subscribe_on(
    hub: &mut EventHub,
    conn_id: u64,
    tx: mpsc::Sender<GwpEvent>,
    kinds: Vec<EventKindV1>,
    tail_enabled: bool,
) -> ResponseV1 {
    let mut deduped: Vec<EventKindV1> = Vec::new();
    for kind in kinds {
        if !deduped.contains(&kind) {
            deduped.push(kind);
        }
    }
    if deduped.contains(&EventKindV1::Log) && !tail_enabled {
        return ResponseV1::Error {
            code: ErrorCode::Unsupported,
            message: "log events need the session's log tail — spawn the server with \
                      ServerOptions { log_tail: true }; supported kinds here: \
                      tree_changed, frame_presented"
                .into(),
        };
    }
    hub.subscribe(Subscriber::new(conn_id, deduped.clone(), tx));
    tracing::debug!(subscriber = conn_id, "event subscription registered");
    ResponseV1::Subscribed { kinds: deduped }
}

fn error_response(err: CoreError) -> ResponseV1 {
    let (code, message) = match &err {
        CoreError::NoMatch { .. } | CoreError::Ambiguous { .. } => {
            (ErrorCode::StrictModeViolation, err.to_string())
        }
        CoreError::Timeout { .. } => (ErrorCode::Timeout, err.to_string()),
        CoreError::StaleNode { .. } => (ErrorCode::StaleGeneration, err.to_string()),
        CoreError::Unsupported(what) => (ErrorCode::Unsupported, (*what).to_string()),
        // §14 (M8 cleanup): the paused-clock rejection rides the same
        // code — the capability exists, the state forbids the command.
        CoreError::ClockPaused(_) => (ErrorCode::Unsupported, err.to_string()),
        CoreError::UnknownWindow { .. } => (ErrorCode::UnknownWindow, err.to_string()),
        // §13 (M8): `Highlight` named a handle the tree does not know.
        CoreError::UnknownNode { .. } => (ErrorCode::UnknownNode, err.to_string()),
        CoreError::Other(msg) => (ErrorCode::Internal, msg.clone()),
        // #[non_exhaustive]: future variants degrade to `Internal` (§7
        // "old clients get clean errors").
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
        send_scoped(cmd_tx, request, None).await
    }

    async fn send_scoped(
        cmd_tx: &mpsc::Sender<Internal>,
        request: RequestV1,
        window: Option<String>,
    ) -> ResponseV1 {
        let (tx, rx) = oneshot::channel();
        cmd_tx
            .send(Internal::Gwp(Command {
                request,
                window,
                events: None,
                reply: tx,
            }))
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
                ServerOptions::default(),
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
                ServerOptions::default(),
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
                ServerOptions::default(),
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
                ServerOptions::default(),
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
            run_session_thread(
                cmd_rx,
                adapter,
                ServerOptions {
                    seams: app_seams(),
                    ..ServerOptions::default()
                },
            );
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
                ServerOptions::default(),
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
