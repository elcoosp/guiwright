//! UDS transport: server loop + client (architecture §7 "Transport: UDS
//! (default; also TCP for CI containers, WS for the viewer)").
//!
//! The connection state machine (handshake gate, dispatch, event flush)
//! lives in [`crate::conn`] and is shared byte-for-byte with the TCP
//! transport; this module owns the accept loop and the client socket.

use tokio::net::{UnixListener, UnixStream};

use crate::GwpError;
use crate::framing::{read_frame, write_frame};
use crate::schema::{GwpRequest, GwpResponse, PROTO_VERSION, RequestV1, ResponseV1};

pub use crate::conn::{EVENT_QUEUE_CAPACITY, Handler, run_connection};

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
                GwpResponse::V1(_) => Err(GwpError::Handshake("expected Welcome")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AdapterInfoV1, CapsV1, ErrorCode};
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
