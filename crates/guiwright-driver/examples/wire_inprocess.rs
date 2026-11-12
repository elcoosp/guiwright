//! The in-process door end to end (architecture §7, M16): the remote
//! stack spawned with [`GwpServerHandle::spawn_inprocess`] — no socket,
//! no filesystem — and a Rust client connected through the returned
//! handle with [`DuplexClient`]. The "app" renames its Save button
//! mid-session through the mock's shared-script channel and the
//! subscriber receives the pushed `TreeChanged` event: §12's live
//! vocabulary, embedded.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_inprocess
//! # prints the subscription ack, the pushed event, and the renamed-node resolve
//! ```
//!
//! This is the door embedding hosts want (the kit, downstream harnesses,
//! in-process tooling): the whole remote-mode stack lives inside the
//! process, byte-identical to the UDS, TCP and WS conversations. Custom
//! listeners (TLS, named pipes, an in-memory mesh) plug in through the
//! public [`guiwright_driver::Transport`] seam with
//! [`GwpServerHandle::spawn_transport_with`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
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
        // App side: the buttons fixture over the embedded door, with the
        // shared-script channel the "app thread" pushes changes into.
        let scripts = Arc::clone(&shared);
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        assert!(
            matches!(
                handle.endpoint(),
                guiwright_driver::server::Endpoint::InProcess
            ),
            "the embedded door reports no socket address"
        );

        // Client side: pipe open, GWP handshake, subscribe.
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        let adapter = client.handshake("wire-inprocess-demo/0.1").await.unwrap();
        println!(
            "attached to {} ({}) over the in-process pipe",
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
                println!("pushed over the pipe: TreeChanged seq={seq}");
            }
            other => panic!("expected TreeChanged, got {other:?}"),
        }

        // The renamed node resolves through the same pipe.
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
        println!("renamed Save resolved over the pipe");

        client.unsubscribe().await.expect("unsubscribed");
        let bye = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(bye, ResponseV1::Bye);
        println!("bye — the embedded door is open");

        handle.shutdown();
    });
}
