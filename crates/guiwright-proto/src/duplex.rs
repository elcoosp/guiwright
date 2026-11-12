//! In-process duplex transport (architecture §7, M16): the embedded door.
//!
//! The fourth GWP door serves sessions that never leave the process: the
//! server accepts `tokio::io::duplex` pipe halves through a queue instead
//! of sockets. Embedding hosts (the kit, downstream test harnesses,
//! in-process tooling) bring the remote-mode stack up with
//! [`crate::duplex::channel`] — or the driver's
//! `GwpServerHandle::spawn_inprocess` — and talk to it with [`Client`],
//! byte-identically to the UDS, TCP and WS conversations.
//!
//! The same module is the reference implementation for downstream
//! transports: an accept loop ([`serve`]) over a source of pre-connected
//! streams plus a client wrapper around the shared
//! [`crate::conn::ClientCore`]. TLS, a named-pipe bridge or an
//! in-memory mesh plug in the same way — the conversation never changes
//! (§7 "the transport owns only its `connect`").

use tokio::io::{DuplexStream, duplex};
use tokio::sync::mpsc;

use crate::GwpError;
use crate::conn::ClientCore;
use crate::schema::{EventKindV1, RequestV1, ResponseV1};

pub use crate::conn::{EVENT_QUEUE_CAPACITY, Handler, run_connection};

/// The per-connection pipe capacity (bytes). GWP frames are bounded by
/// the 64 MiB frame guard, but in-process traffic is command-and-event
/// shaped (the large payloads ride the blob side-channel in one direction
/// only); 64 KiB keeps a slow reader from buffering a whole tree dump
/// without starving the handshake.
pub const DUPLEX_BUFFER_BYTES: usize = 64 * 1024;

/// The pending-connection queue depth: how many [`Client::connect`] calls
/// may sit between the client half and the serve loop's accept. Matches
/// the command queue's depth (§5.1) — the embedded door is not a load
/// balancer.
const CONNECT_QUEUE_CAPACITY: usize = 64;

/// The accept half of the embedded door: the source of pre-connected
/// server pipe halves. Feed it to [`serve`] (or the driver's
/// [`Transport`](crate::duplex::serve) seam) to run GWP conversations.
#[derive(Debug)]
pub struct DuplexListener {
    pending: mpsc::Receiver<DuplexStream>,
}

/// The connect half of the embedded door: cloneable, handed to clients.
/// Dropping every clone (or the server) fails later connects honestly
/// instead of queueing into the void.
#[derive(Clone, Debug)]
pub struct DuplexConnect {
    queue: mpsc::Sender<DuplexStream>,
}

/// Build the two halves of the embedded door: the listener to serve on
/// and the connect handle to give clients.
pub fn channel() -> (DuplexListener, DuplexConnect) {
    let (queue, pending) = mpsc::channel(CONNECT_QUEUE_CAPACITY);
    (DuplexListener { pending }, DuplexConnect { queue })
}

/// Serve connections on the embedded door until the connect half is
/// dropped. Each connection: expect Hello first, reject mismatched
/// protocol versions cleanly, then dispatch requests to the handler —
/// the same [`run_connection`] state machine the UDS, TCP and WS doors
/// run.
pub async fn serve<H: Handler>(listener: DuplexListener, mut make_handler: impl FnMut() -> H) {
    let DuplexListener { mut pending } = listener;
    while let Some(stream) = pending.recv().await {
        let handler = make_handler();
        tokio::spawn(async move {
            if let Err(err) = run_connection(stream, handler).await {
                tracing::debug!(%err, "in-process connection closed");
            }
        });
    }
    tracing::debug!("in-process listener closed");
}

/// Client half of an embedded session. The conversation (handshake,
/// requests, event buffering) is the shared [`crate::conn::ClientCore`];
/// this type only owns the pipe.
#[derive(Debug)]
pub struct Client {
    core: ClientCore<DuplexStream>,
}

impl Client {
    /// Connect through the embedded door: open a fresh in-process pipe
    /// and hand its server half to the listener. Fails when the server
    /// (or every connect handle) is gone — never queues into the void.
    pub async fn connect(connect: &DuplexConnect) -> Result<Client, GwpError> {
        let (client_half, server_half) = duplex(DUPLEX_BUFFER_BYTES);
        connect.queue.send(server_half).await.map_err(|_| {
            GwpError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the in-process listener is gone (server dropped)",
            ))
        })?;
        Ok(Client {
            core: ClientCore::new(client_half),
        })
    }

    /// Send `Hello` and validate the driver's `Welcome`.
    pub async fn handshake(
        &mut self,
        agent: &str,
    ) -> Result<crate::schema::AdapterInfoV1, GwpError> {
        self.core.handshake(agent).await
    }

    pub async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, GwpError> {
        self.core.request(request).await
    }

    /// The next pushed event (§7, M12): from the buffer of events that
    /// interleaved earlier responses, or straight off the pipe. Blocks
    /// until the driver publishes one — pair it with the subscriber's
    /// own pacing, never on a hot path.
    pub async fn next_event(&mut self) -> Result<crate::schema::GwpEvent, GwpError> {
        self.core.next_event().await
    }

    /// Subscribe this connection to the named event kinds (§7, M12).
    /// Re-subscribing replaces the filter; the ack echoes the kinds the
    /// driver honored.
    pub async fn subscribe(&mut self, kinds: &[EventKindV1]) -> Result<(), GwpError> {
        self.core.subscribe(kinds).await
    }

    /// Drop this connection's event subscription (§7, M12).
    pub async fn unsubscribe(&mut self) -> Result<(), GwpError> {
        self.core.unsubscribe().await
    }

    pub async fn ping(&mut self) -> Result<(), GwpError> {
        self.core.ping().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AdapterInfoV1, CapsV1, ErrorCode, PROTO_VERSION};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoHandler {
        handshakes: Arc<AtomicUsize>,
    }

    impl Handler for EchoHandler {
        fn handle(
            &mut self,
            request: RequestV1,
        ) -> impl std::future::Future<Output = ResponseV1> + Send {
            let response = match request {
                RequestV1::Hello { .. } => {
                    self.handshakes.fetch_add(1, Ordering::SeqCst);
                    ResponseV1::Welcome {
                        proto_version: PROTO_VERSION,
                        session_id: "test-session".into(),
                        tree_seq: 1,
                        frame_seq: 2,
                        adapter: AdapterInfoV1 {
                            name: "mock".into(),
                            framework: "fixture".into(),
                            framework_version: "0".into(),
                            caps: CapsV1::default(),
                        },
                    }
                }
                RequestV1::Ping => ResponseV1::Pong,
                RequestV1::Bye => ResponseV1::Bye,
                _ => ResponseV1::Error {
                    code: ErrorCode::Unsupported,
                    message: "not implemented in echo handler".into(),
                },
            };
            async move { response }
        }
    }

    #[tokio::test]
    async fn handshake_ping_bye_roundtrip_inprocess() {
        let (listener, connect) = channel();
        let handshakes = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&handshakes);
        tokio::spawn(async move {
            serve(listener, move || EchoHandler {
                handshakes: Arc::clone(&h2),
            })
            .await;
        });

        let mut client = Client::connect(&connect).await.unwrap();
        let adapter = client.handshake("test-client/0.1").await.unwrap();
        assert_eq!(adapter.name, "mock");
        assert_eq!(handshakes.load(Ordering::SeqCst), 1);

        client.ping().await.unwrap();

        let resp = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(resp, ResponseV1::Bye);
    }

    #[tokio::test]
    async fn connect_after_listener_drop_fails_cleanly() {
        let (listener, connect) = channel();
        drop(listener);
        let err = Client::connect(&connect)
            .await
            .expect_err("listener gone = connect error");
        assert!(matches!(err, GwpError::Io(_)), "{err:?}");
        assert!(err.to_string().contains("listener is gone"));
    }

    #[tokio::test]
    async fn non_hello_first_request_rejected_inprocess() {
        let (listener, connect) = channel();
        tokio::spawn(async move {
            serve(listener, || EchoHandler {
                handshakes: Arc::new(AtomicUsize::new(0)),
            })
            .await;
        });

        let mut client = Client::connect(&connect).await.unwrap();
        // Skip the handshake: the connection loop must reject a non-Hello
        // first frame exactly like the socket doors do.
        let err = client
            .request(RequestV1::Ping)
            .await
            .expect_err("un-handshaked request");
        assert!(
            matches!(
                err,
                GwpError::Rejected {
                    code: ErrorCode::NotHandshaked,
                    ..
                }
            ),
            "{err:?}"
        );
    }
}
