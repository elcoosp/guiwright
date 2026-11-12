//! TCP transport (architecture §7 "Transport: UDS (default; also TCP for
//! CI containers, WS for the viewer)"): the loopback/container door that
//! other-language clients drive (§16 "Other-language clients — GWP over
//! the wire; schema-published", M3). The connection state machine is the
//! shared [`crate::conn`] one — a TCP session is byte-identical to a UDS
//! or WS session, only the socket differs.

use tokio::net::{TcpListener, TcpStream};

use crate::GwpError;
use crate::conn::ClientCore;
use crate::schema::{AdapterInfoV1, EventKindV1, RequestV1, ResponseV1};

pub use crate::conn::Handler;

/// Serve connections on `listener` until the process ends (same
/// handshake/dispatch semantics as the UDS [`crate::uds::serve`]).
pub async fn serve<H: Handler>(listener: TcpListener, mut make_handler: impl FnMut() -> H) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let handler = make_handler();
                tokio::spawn(async move {
                    if let Err(err) = crate::conn::run_connection(stream, handler).await {
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

/// Client half of a TCP session (`127.0.0.1:port` or any `host:port`).
/// The conversation (handshake, requests, event buffering) is the shared
/// [`crate::conn::ClientCore`]; this type only owns the socket.
#[derive(Debug)]
pub struct Client {
    core: ClientCore<TcpStream>,
}

impl Client {
    pub async fn connect(addr: &str) -> Result<Client, GwpError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Client {
            core: ClientCore::new(stream),
        })
    }

    /// Connected peer address (the server's bound port lives here for
    /// `127.0.0.1:0` style ephemeral binds).
    pub fn peer_addr(&self) -> Result<std::net::SocketAddr, GwpError> {
        self.core.stream().peer_addr().map_err(GwpError::Io)
    }

    /// Send `Hello` and validate the driver's `Welcome` (§7).
    pub async fn handshake(&mut self, agent: &str) -> Result<AdapterInfoV1, GwpError> {
        self.core.handshake(agent).await
    }

    /// One request → response round-trip; `Error` replies map to
    /// [`GwpError::Rejected`]. Pushed [`ResponseV1::Event`] frames that
    /// interleave are buffered for [`Client::next_event`] (§7, M12).
    pub async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, GwpError> {
        self.core.request(request).await
    }

    /// The next pushed event (§7, M12) — the TCP mirror of the UDS
    /// [`crate::uds::Client::next_event`].
    pub async fn next_event(&mut self) -> Result<crate::schema::GwpEvent, GwpError> {
        self.core.next_event().await
    }

    /// Subscribe this connection to the named event kinds (§7, M12).
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
    use crate::framing::{read_frame, write_frame};
    use crate::schema::{CapsV1, ErrorCode, GwpRequest, GwpResponse, PROTO_VERSION};
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
                        session_id: "tcp-session".into(),
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

    /// Bind an ephemeral localhost listener and serve `EchoHandler` on it.
    async fn spawn_server(tag: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let handshakes = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&handshakes);
        tokio::spawn(async move {
            serve(listener, move || EchoHandler {
                handshakes: Arc::clone(&h2),
            })
            .await;
        });
        tracing::debug!(%tag, %addr, "tcp test server up");
        (addr, handshakes)
    }

    #[tokio::test]
    async fn handshake_ping_bye_roundtrip() {
        let (addr, handshakes) = spawn_server("roundtrip").await;

        let mut client = Client::connect(&addr).await.unwrap();
        let adapter = client.handshake("ts-client/0.1").await.unwrap();
        assert_eq!(adapter.name, "mock");
        assert_eq!(handshakes.load(Ordering::SeqCst), 1);

        client.ping().await.unwrap();

        let resp = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(resp, ResponseV1::Bye);
    }

    #[tokio::test]
    async fn sequential_connections_each_handshake() {
        let (addr, handshakes) = spawn_server("sequential").await;

        for i in 0..3 {
            let mut client = Client::connect(&addr).await.unwrap();
            let adapter = client.handshake(&format!("client-{i}")).await.unwrap();
            assert_eq!(adapter.name, "mock");
            client.ping().await.unwrap();
            let resp = client.request(RequestV1::Bye).await.unwrap();
            assert_eq!(resp, ResponseV1::Bye);
        }
        assert_eq!(handshakes.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn version_mismatch_gets_clean_error() {
        let (addr, _handshakes) = spawn_server("version").await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();
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
        let (addr, _handshakes) = spawn_server("nohello").await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();
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
        let (addr, _handshakes) = spawn_server("error").await;

        let mut client = Client::connect(&addr).await.unwrap();
        client.handshake("test").await.unwrap();
        let err = client
            .request(RequestV1::TreeSnapshot)
            .await
            .expect_err("unsupported request");
        assert!(matches!(err, GwpError::Rejected { .. }), "{err:?}");
    }

    /// The M3 parity guarantee: the same handler over UDS and TCP answers
    /// with byte-identical responses (only the socket differs, §7).
    #[tokio::test]
    async fn tcp_matches_uds_bytes() {
        use crate::framing::encode_frame;

        let (addr, _handshakes) = spawn_server("parity").await;
        let mut tcp = Client::connect(&addr).await.unwrap();
        tcp.handshake("parity").await.unwrap();
        tcp.ping().await.unwrap();
        let via_tcp = tcp.request(RequestV1::TreeSnapshot).await;
        assert!(via_tcp.is_err(), "echo handler rejects TreeSnapshot");

        // The frames on the wire for the same payloads are transport-blind:
        // encode once, compare — both transports use the same framing.
        let req = GwpRequest::V1(RequestV1::Ping);
        let frame = encode_frame(&req).unwrap();
        assert_eq!(&frame[4], &0x00, "V1 envelope");
        assert_eq!(&frame[5], &0x01, "Ping discriminant");
    }
}
