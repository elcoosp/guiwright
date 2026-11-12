//! End-to-end remote mode over WebSocket (architecture §7, M15): the
//! RFC 6455 door served by [`GwpServerHandle::spawn_ws`], driven with the
//! proto [`WsClient`]. The conversation is byte-identical to the UDS and
//! TCP sessions — handshake → resolve → act → clock, then the M12 event
//! leg — only the pipe differs.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::WsClient;
use guiwright_proto::schema::{
    ClockV1, EventKindV1, RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1,
};

fn ws_endpoint(handle: &GwpServerHandle) -> std::net::SocketAddr {
    match handle.endpoint() {
        guiwright_driver::server::Endpoint::Ws(addr) => *addr,
        other => panic!("expected WS endpoint, got {other:?}"),
    }
}

/// The M3 TS-client conversation (`connectWs` → handshake → snapshot →
/// resolve → act), now against a Rust WS server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_remote_session_over_ws() {
    let handle = GwpServerHandle::spawn_ws(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("ws server binds");
    let addr = ws_endpoint(&handle);
    assert_ne!(addr.port(), 0, "ephemeral bind reported");

    let mut client = WsClient::connect(&addr.to_string())
        .await
        .expect("client upgrades");
    let adapter = client.handshake("ts-client/0.1").await.unwrap();
    assert_eq!(adapter.name, "mock");
    assert!(adapter.caps.tree_update);
    client.ping().await.unwrap();

    // Resolve + act (the TS client's core two round-trips).
    let resolved = client
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
    let (node, generation) = match resolved {
        ResponseV1::Resolved {
            node, generation, ..
        } => (node, generation),
        other => panic!("expected Resolved, got {other:?}"),
    };
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

    // Clock vocabulary rides the same transport.
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

/// The M12 event leg over WS: subscribe, receive the driver's pushed
/// `FramePresented` while idle, unsubscribe back to silence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_flow_over_ws() {
    let handle = GwpServerHandle::spawn_ws(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("ws server binds");
    let addr = ws_endpoint(&handle);

    let mut client = WsClient::connect(&addr.to_string())
        .await
        .expect("client upgrades");
    client.handshake("ws-subscriber/0.1").await.unwrap();
    client
        .subscribe(&[EventKindV1::FramePresented])
        .await
        .expect("subscribe");

    // The mock adapter presents frames on its own cadence; the pushed
    // event must arrive with no request in flight.
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), client.next_event())
        .await
        .expect("event in time")
        .expect("event parses");
    match event {
        guiwright_proto::GwpEvent::V1(guiwright_proto::EventV1::FramePresented { .. }) => {}
        other => panic!("expected FramePresented, got {other:?}"),
    }

    client.unsubscribe().await.expect("unsubscribe");
    client.ping().await.expect("still alive after unsubscribe");

    handle.shutdown();
}
