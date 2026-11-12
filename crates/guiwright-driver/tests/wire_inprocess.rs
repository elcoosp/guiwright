//! End-to-end remote mode over the in-process door (architecture §7, M16):
//! the embedded transport served by [`GwpServerHandle::spawn_inprocess`],
//! driven with the proto [`DuplexClient`] — no OS socket involved. The
//! conversation is byte-identical to the UDS, TCP and WS sessions —
//! handshake → resolve → act → clock, then the M12 event leg — only the
//! pipe differs. The same milestone opens the public [`Transport`] seam:
//! a downstream-style wrapper transport is spawned and its per-connection
//! hook is observed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::{BoxServeFuture, GwpServerHandle, ServerOptions, Transport};
use guiwright_proto::DuplexClient;
use guiwright_proto::duplex::DuplexListener;
use guiwright_proto::schema::{
    ClockV1, EventKindV1, EventV1, GwpEvent, RequestV1, ResponseV1, SelectorV1, SemActionV1,
    TextMatchV1,
};

/// A downstream-style transport wrapper (the M16 seam contract): counts
/// the connections the serve loop opens by hooking `make_handler`, then
/// delegates the accept loop to the inner [`DuplexListener`]. Lives in
/// this test file on purpose — the trait must be implementable outside
/// the driver crate.
struct CountingTransport {
    inner: DuplexListener,
    connections: Arc<AtomicUsize>,
}

impl Transport for CountingTransport {
    fn into_serve<H: guiwright_proto::Handler>(
        self,
        mut make_handler: impl FnMut() -> H + Send + 'static,
    ) -> BoxServeFuture {
        let connections = Arc::clone(&self.connections);
        self.inner.into_serve(move || {
            connections.fetch_add(1, Ordering::Relaxed);
            make_handler()
        })
    }
}

fn resolved_node(response: ResponseV1, tag: &str) -> (guiwright_proto::schema::NodeKeyV1, u32) {
    match response {
        ResponseV1::Resolved {
            node, generation, ..
        } => (node, generation),
        other => panic!("expected Resolved ({tag}), got {other:?}"),
    }
}

/// The full remote-session conversation over the embedded door: no
/// socket, no filesystem — the server lives in the test process.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_remote_session_inprocess() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    assert!(matches!(
        handle.endpoint(),
        guiwright_driver::server::Endpoint::InProcess
    ));

    let mut client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    let adapter = client.handshake("embedded-client/0.1").await.unwrap();
    assert_eq!(adapter.name, "mock");
    assert!(adapter.caps.tree_update);
    client.ping().await.unwrap();

    // Resolve + act — the same two round-trips every door serves.
    let (node, generation) = resolved_node(
        client
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
            .unwrap(),
        "Save button",
    );
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

    // Clock vocabulary rides the embedded door unchanged.
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

/// The M12 event leg over the embedded door: subscribe on one connection,
/// receive `FramePresented` while idle with no request in flight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_flow_inprocess() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    client.handshake("embedded-subscriber/0.1").await.unwrap();
    client
        .subscribe(&[EventKindV1::FramePresented])
        .await
        .expect("subscribe");

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), client.next_event())
        .await
        .expect("event arrives with no request in flight")
        .expect("event frame");
    assert!(matches!(
        event,
        GwpEvent::V1(EventV1::FramePresented { .. })
    ));

    client.unsubscribe().await.expect("unsubscribe");
    handle.shutdown();
}

/// Two connections on one embedded server: each aimed conversation stays
/// independent (the §11 "one session serves all connections" discipline),
/// and the transport seam's per-connection hook observed both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_transport_serves_two_connections() {
    let (listener, connect) = guiwright_proto::duplex::channel();
    let connections = Arc::new(AtomicUsize::new(0));
    let handle = GwpServerHandle::spawn_transport_with(
        CountingTransport {
            inner: listener,
            connections: Arc::clone(&connections),
        },
        || MockAdapter::with_fixture(&Fixture::buttons()),
        ServerOptions::default(),
    )
    .expect("custom-transport server starts");
    assert!(matches!(
        handle.endpoint(),
        guiwright_driver::server::Endpoint::InProcess
    ));

    let mut first = DuplexClient::connect(&connect).await.expect("first client");
    first.handshake("first/0.1").await.unwrap();
    let mut second = DuplexClient::connect(&connect)
        .await
        .expect("second client");
    second.handshake("second/0.1").await.unwrap();

    // Each connection resolves independently; the shared session answers
    // both (§11).
    for (client, tag) in [(&mut first, "first"), (&mut second, "second")] {
        let response = client
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
        let (node, generation) = resolved_node(response, tag);
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
        assert!(
            matches!(done, ResponseV1::ActionDone { .. }),
            "expected ActionDone ({tag}), got {done:?}"
        );
    }

    assert_eq!(
        connections.load(Ordering::Relaxed),
        2,
        "the transport seam counted both connections"
    );

    handle.shutdown();
}

/// The embedded door's connect handle reports an honest error once the
/// server is gone: `connect` fails instead of hanging or silently
/// queueing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_after_shutdown_fails_cleanly() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    handle.shutdown();
    let err = DuplexClient::connect(&connect)
        .await
        .expect_err("connect after shutdown is an error, not a hang");
    assert!(
        matches!(err, guiwright_proto::GwpError::Io(_)),
        "the listener-gone error rides the Io family, got {err:?}"
    );
}
