//! The client-side connection seam (architecture §7, M18): every shipped
//! client — UDS, TCP, WebSocket and the in-process duplex door —
//! implements one [`ClientConn`] trait, so a consumer written against the
//! trait (the viewer's live feed, a downstream recorder) runs over any
//! door unchanged. The server-side mirror is [`guiwright_proto::Handler`]:
//! one seam per conversation half.
//!
//! The contract is the shared conversation vocabulary itself: handshake,
//! request, `next_event`, subscribe/unsubscribe, ping. The connect call
//! stays transport-owned (a path, an address, a duplex handle) — the §7
//! "the transport owns only its `connect`" rule.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use guiwright_proto::DuplexClient;
use guiwright_proto::Handler;
use guiwright_proto::TcpClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::conn::run_connection;
use guiwright_proto::duplex;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, ErrorCode, EventKindV1, PROTO_VERSION, RequestV1, ResponseV1,
};
use guiwright_proto::uds::Client as UdsSocketClient;
use guiwright_proto::ws::WsClient;

/// The echo conversation every door serves: Hello → Welcome, Ping → Pong,
/// Subscribe → Subscribed (echoing the kinds), Bye → Bye.
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
                    session_id: "seam-test".into(),
                    tree_seq: 1,
                    frame_seq: 0,
                    adapter: AdapterInfoV1 {
                        name: "mock".into(),
                        framework: "fixture".into(),
                        framework_version: "0".into(),
                        caps: CapsV1::default(),
                    },
                }
            }
            RequestV1::Ping => ResponseV1::Pong,
            RequestV1::Subscribe(guiwright_proto::schema::SubscribeV1::On { kinds }) => {
                ResponseV1::Subscribed { kinds }
            }
            RequestV1::Subscribe(guiwright_proto::schema::SubscribeV1::Off) => {
                ResponseV1::Subscribed { kinds: Vec::new() }
            }
            RequestV1::Bye => ResponseV1::Bye,
            _ => ResponseV1::Error {
                code: ErrorCode::Unsupported,
                message: "not implemented in the seam echo handler".into(),
            },
        };
        async move { response }
    }
}

/// One full conversation written against the trait, not a concrete
/// client: the exact shape the viewer's live feed takes.
async fn exercise<C: ClientConn>(client: &mut C) {
    let adapter = client.handshake("seam-test/0.1").await.expect("handshake");
    assert_eq!(adapter.name, "mock");

    client.ping().await.expect("ping");

    let acked = match client
        .request(RequestV1::Subscribe(
            guiwright_proto::schema::SubscribeV1::On {
                kinds: vec![EventKindV1::TreeChanged],
            },
        ))
        .await
        .expect("subscribe")
    {
        ResponseV1::Subscribed { kinds } => kinds,
        other => panic!("expected Subscribed, got {other:?}"),
    };
    assert_eq!(acked, vec![EventKindV1::TreeChanged]);

    client.unsubscribe().await.expect("unsubscribe");

    let bye = client.request(RequestV1::Bye).await.expect("bye");
    assert_eq!(bye, ResponseV1::Bye);
}

fn temp_socket_path(tag: &str) -> std::path::PathBuf {
    // TempDir drops at end of scope; keep it alive for the test by
    // leaking (tests are short-lived).
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

#[tokio::test]
async fn the_seam_serves_the_uds_door() {
    let path = temp_socket_path("seam-uds.sock");
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).expect("bind");
    let handshakes = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&handshakes);
    tokio::spawn(async move {
        guiwright_proto::uds::serve(listener, move || EchoHandler {
            handshakes: Arc::clone(&h2),
        })
        .await;
    });

    let mut client = UdsSocketClient::connect(&path).await.expect("connect");
    exercise(&mut client).await;
    assert_eq!(handshakes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_seam_serves_the_tcp_door() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let handshakes = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&handshakes);
    tokio::spawn(async move {
        guiwright_proto::tcp::serve(listener, move || EchoHandler {
            handshakes: Arc::clone(&h2),
        })
        .await;
    });

    let mut client = TcpClient::connect(&addr).await.expect("connect");
    exercise(&mut client).await;
    assert_eq!(handshakes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_seam_serves_the_embedded_door() {
    let (listener, connect) = duplex::channel();
    let handshakes = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&handshakes);
    tokio::spawn(async move {
        guiwright_proto::duplex::serve(listener, move || EchoHandler {
            handshakes: Arc::clone(&h2),
        })
        .await;
    });

    let mut client = DuplexClient::connect(&connect).await.expect("connect");
    exercise(&mut client).await;
    assert_eq!(handshakes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_seam_serves_the_ws_door() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handshakes = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&handshakes);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                return;
            };
            let handshakes = Arc::clone(&h2);
            tokio::spawn(async move {
                // The upgrade + the same shared connection loop every
                // other door runs.
                if let Ok(ws) = guiwright_proto::ws::accept(stream).await {
                    let _ = run_connection(ws, EchoHandler { handshakes }).await;
                }
            });
        }
    });

    let mut client = WsClient::connect(addr.to_string().as_str())
        .await
        .expect("connect");
    exercise(&mut client).await;
    assert_eq!(handshakes.load(Ordering::SeqCst), 1);
}
