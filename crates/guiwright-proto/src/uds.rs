//! UDS transport: server loop + client (architecture §7 "Transport: UDS
//! (default; also TCP for CI containers, WS for the viewer)").
//!
//! The server multiplexes one connection per client; each request is a
//! round-trip. Pushed events land on a shared queue the server can flush
//! between requests (bounded — slow clients degrade, never block the UI
//! thread, §15.6).

use std::future::Future;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::GwpError;
use crate::framing::{read_frame, write_frame};
use crate::schema::{
    ErrorCode, EventV1, GwpEvent, GwpRequest, GwpResponse, PROTO_VERSION, RequestV1, ResponseV1,
};

/// A request handler: application logic behind the transport.
pub trait Handler: Send + 'static {
    fn handle(&mut self, request: RequestV1) -> impl Future<Output = ResponseV1> + Send;
}

/// Bounded outbound event queue (§15.6 backpressure: 64 messages default).
pub const EVENT_QUEUE_CAPACITY: usize = 64;

/// Serve connections on `listener` until the process ends. Each connection:
/// expect Hello first, reject mismatched protocol versions cleanly, then
/// dispatch requests to the handler.
pub async fn serve<H: Handler>(listener: UnixListener, mut make_handler: impl FnMut() -> H) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let handler = make_handler();
                tokio::spawn(async move {
                    if let Err(err) = run_connection(stream, handler).await {
                        tracing::debug!(%err, "connection closed");
                    }
                });
            }
            Err(err) => {
                tracing::warn!(%err, "accept failed");
                return;
            }
        }
    }
}

async fn run_connection<H: Handler>(
    mut stream: UnixStream,
    mut handler: H,
) -> Result<(), GwpError> {
    // Handshake gate: first frame must be Hello with a known version (§7).
    let first: GwpRequest = read_frame(&mut stream).await?;
    let GwpRequest::V1(RequestV1::Hello {
        proto_version,
        agent,
    }) = first
    else {
        write_frame(
            &mut stream,
            &GwpResponse::V1(ResponseV1::Error {
                code: ErrorCode::NotHandshaked,
                message: "first request must be Hello".into(),
            }),
        )
        .await?;
        return Err(GwpError::Handshake("first request was not Hello"));
    };
    if proto_version != PROTO_VERSION {
        write_frame(
            &mut stream,
            &GwpResponse::V1(ResponseV1::Error {
                code: ErrorCode::UnsupportedVersion,
                message: format!(
                    "client protocol {proto_version}, driver protocol {PROTO_VERSION}"
                ),
            }),
        )
        .await?;
        return Err(GwpError::Handshake("version mismatch"));
    }
    tracing::debug!(agent, "client handshaked");
    write_frame(
        &mut stream,
        &handshake_response(
            &handler
                .handle(RequestV1::Hello {
                    proto_version,
                    agent,
                })
                .await,
        ),
    )
    .await?;

    // Event pump: events queued by the app side are flushed between
    // requests (bounded queue, §15.6).
    let (_event_tx, mut event_rx) = mpsc::channel::<GwpEvent>(EVENT_QUEUE_CAPACITY);
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                if let Some(event) = event {
                    write_frame(&mut stream, &event).await?;
                }
            }
            req = read_frame::<GwpRequest, _>(&mut stream) => {
                let req = req?;
                // GwpRequest has a single (versioned) variant, so this is
                // irrefutable; the version check happened at handshake.
                let GwpRequest::V1(inner) = req;
                if matches!(inner, RequestV1::Bye) {
                    write_frame(&mut stream, &GwpResponse::V1(ResponseV1::Bye)).await?;
                    return Ok(());
                }
                let resp = handler.handle(inner).await;
                write_frame(&mut stream, &GwpResponse::V1(resp)).await?;
                // Opportunistically flush queued events after each response.
                while let Ok(event) = event_rx.try_recv() {
                    write_frame(&mut stream, &event).await?;
                }
            }
        }
    }
}

/// Extract a `Welcome` from a handler response, or synthesize an error.
fn handshake_response(resp: &ResponseV1) -> GwpResponse {
    match resp {
        ResponseV1::Welcome { .. } => GwpResponse::V1(resp.clone()),
        other => GwpResponse::V1(ResponseV1::Error {
            code: ErrorCode::Internal,
            message: format!("driver handler did not answer Hello with Welcome: {other:?}"),
        }),
    }
}

/// Client half of a UDS session.
#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
}

impl Client {
    pub async fn connect(path: &std::path::Path) -> Result<Client, GwpError> {
        let stream = UnixStream::connect(path).await?;
        Ok(Client { stream })
    }

    /// Send `Hello` and validate the driver's `Welcome`.
    pub async fn handshake(
        &mut self,
        agent: &str,
    ) -> Result<crate::schema::AdapterInfoV1, GwpError> {
        write_frame(
            &mut self.stream,
            &GwpRequest::V1(RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: agent.to_string(),
            }),
        )
        .await?;
        let resp: GwpResponse = read_frame(&mut self.stream).await?;
        let GwpResponse::V1(ResponseV1::Welcome {
            proto_version,
            adapter,
            ..
        }) = resp
        else {
            return match resp {
                GwpResponse::V1(ResponseV1::Error { code, message }) => {
                    Err(GwpError::Rejected { code, message })
                }
                _ => Err(GwpError::Handshake("expected Welcome")),
            };
        };
        if proto_version != PROTO_VERSION {
            return Err(GwpError::Handshake("driver protocol mismatch"));
        }
        Ok(adapter)
    }

    pub async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, GwpError> {
        write_frame(&mut self.stream, &GwpRequest::V1(request)).await?;
        // Responses only, until one arrives; events are tolerated and
        // dropped for M0 (subscribers are a later milestone).
        // Every branch below returns; the loop is collapsed (clippy
        // `never_loop`) until event frames become a real milestone.
        let resp: GwpResponse = read_frame(&mut self.stream).await?;
        let GwpResponse::V1(inner) = resp;
        match inner {
            ResponseV1::Error { code, message } => Err(GwpError::Rejected { code, message }),
            other => Ok(other),
        }
    }

    pub async fn ping(&mut self) -> Result<(), GwpError> {
        match self.request(RequestV1::Ping).await? {
            ResponseV1::Pong => Ok(()),
            other => Err(GwpError::Protocol(format!("expected Pong, got {other:?}"))),
        }
    }
}

/// Client-side helper to build events for tests/servers.
pub fn event(seq: u64) -> GwpEvent {
    GwpEvent::V1(EventV1::FramePresented { seq, hash: seq })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AdapterInfoV1, CapsV1};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoHandler {
        handshakes: Arc<AtomicUsize>,
    }

    impl Handler for EchoHandler {
        async fn handle(&mut self, request: RequestV1) -> ResponseV1 {
            match request {
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
            }
        }
    }

    fn temp_socket_path(tag: &str) -> std::path::PathBuf {
        // TempDir drops at end of scope; keep it alive for the test by
        // leaking (tests are short-lived).
        let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
        dir.path().join(tag)
    }

    #[tokio::test]
    async fn handshake_ping_bye_roundtrip() {
        let path = temp_socket_path("handshake.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let handshakes = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&handshakes);
        tokio::spawn(async move {
            serve(listener, move || EchoHandler {
                handshakes: Arc::clone(&h2),
            })
            .await;
        });

        let mut client = Client::connect(&path).await.unwrap();
        let adapter = client.handshake("test-client/0.1").await.unwrap();
        assert_eq!(adapter.name, "mock");
        assert_eq!(handshakes.load(Ordering::SeqCst), 1);

        client.ping().await.unwrap();

        let resp = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(resp, ResponseV1::Bye);
    }

    #[tokio::test]
    async fn version_mismatch_gets_clean_error() {
        let path = temp_socket_path("version.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let handshakes = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&handshakes);
        tokio::spawn(async move {
            serve(listener, move || EchoHandler {
                handshakes: Arc::clone(&h2),
            })
            .await;
        });

        let mut stream = UnixStream::connect(&path).await.unwrap();
        write_frame(
            &mut stream,
            &GwpRequest::V1(RequestV1::Hello {
                proto_version: 99,
                agent: "old-client".into(),
            }),
        )
        .await
        .unwrap();
        let resp: GwpResponse = read_frame(&mut stream).await.unwrap();
        let GwpResponse::V1(ResponseV1::Error { code, message }) = resp else {
            panic!("expected error, got {resp:?}");
        };
        assert_eq!(code, ErrorCode::UnsupportedVersion);
        assert!(message.contains("99"));
    }

    #[tokio::test]
    async fn non_hello_first_request_rejected() {
        let path = temp_socket_path("nohello.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let handshakes = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&handshakes);
        tokio::spawn(async move {
            serve(listener, move || EchoHandler {
                handshakes: Arc::clone(&h2),
            })
            .await;
        });

        let mut stream = UnixStream::connect(&path).await.unwrap();
        write_frame(&mut stream, &GwpRequest::V1(RequestV1::Ping))
            .await
            .unwrap();
        let resp: GwpResponse = read_frame(&mut stream).await.unwrap();
        let GwpResponse::V1(ResponseV1::Error { code, .. }) = resp else {
            panic!("expected error, got {resp:?}");
        };
        assert_eq!(code, ErrorCode::NotHandshaked);
    }

    #[tokio::test]
    async fn error_response_maps_to_rejected() {
        let path = temp_socket_path("error.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let handshakes = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&handshakes);
        tokio::spawn(async move {
            serve(listener, move || EchoHandler {
                handshakes: Arc::clone(&h2),
            })
            .await;
        });

        let mut client = Client::connect(&path).await.unwrap();
        client.handshake("test").await.unwrap();
        let err = client
            .request(RequestV1::TreeSnapshot)
            .await
            .expect_err("unsupported request");
        assert!(matches!(err, GwpError::Rejected { .. }), "{err:?}");
    }
}
