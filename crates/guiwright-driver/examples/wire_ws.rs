//! The WebSocket door end to end (architecture §7, M15): a GWP server
//! spawned over the RFC 6455 upgrade ([`GwpServerHandle::spawn_ws`]) and
//! a Rust client that connects with [`WsClient`] — the M3 TypeScript
//! client's `connectWs` peer — subscribes to tree changes, and receives
//! the pushed `TreeChanged` event when the "app" renames its Save button
//! mid-session through the mock's shared-script channel (§12's live
//! vocabulary, on the viewer's own transport).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_ws
//! # prints the subscription ack, the pushed event, and the renamed-node resolve
//! ```
//!
//! The same conversation works over UDS and TCP; the WS variant exists
//! because the viewer wants its live mode through browser-friendly
//! sockets without a byte of protocol difference.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::WsClient;
use guiwright_proto::schema::{
    EventKindV1, EventV1, GwpEvent, RequestV1, ResponseV1, SelectorV1, TextMatchV1,
};

fn main() {
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // App side: the buttons fixture over the WebSocket door, with the
        // shared-script channel the "app thread" pushes changes into.
        let scripts = Arc::clone(&shared);
        let handle = GwpServerHandle::spawn_ws(
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("ws server binds");
        let addr = match handle.endpoint() {
            guiwright_driver::server::Endpoint::Ws(addr) => *addr,
            other => panic!("expected a WS endpoint, got {other:?}"),
        };

        // Client side: upgrade, GWP handshake, subscribe.
        let mut client = WsClient::connect(&addr.to_string())
            .await
            .expect("client upgrades");
        let adapter = client.handshake("wire-ws-demo/0.1").await.unwrap();
        println!(
            "attached to {} ({}) over the upgraded socket",
            adapter.name, adapter.framework
        );

        client
            .subscribe(&[EventKindV1::TreeChanged])
            .await
            .expect("subscribed to TreeChanged");

        // App-side change mid-session: the rename lands on the session's
        // next pump and the subscriber sees it with no request in flight.
        let mut save = Node::new(Role::Button);
        save.set_label("Save (renamed)");
        shared.lock().unwrap().push(TreeUpdate {
            nodes: vec![(NodeId(2), save)],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(2),
        });

        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("event in time")
            .expect("event parses");
        match event {
            GwpEvent::V1(EventV1::TreeChanged { seq }) => {
                println!("pushed over ws: TreeChanged seq={seq}");
            }
            other => panic!("expected TreeChanged, got {other:?}"),
        }

        // The renamed node resolves through the same socket.
        let resolved = client
            .request(RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save (renamed)".into(),
                        exact: true,
                        ignore_case: false,
                    }),
                },
                timeout_ms: 2000,
            })
            .await
            .unwrap();
        assert!(
            matches!(resolved, ResponseV1::Resolved { .. }),
            "{resolved:?}"
        );
        println!("renamed Save resolved over ws");

        client.unsubscribe().await.expect("unsubscribed");
        let bye = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(bye, ResponseV1::Bye);
        println!("bye — the viewer live-mode door is open");

        handle.shutdown();
    });
}
