//! The §7 event-channel consumer (M12): a GWP server whose app tree
//! changes on its own + a Rust client that subscribes and reads the
//! pushed events — the "a live dashboard watches a remote app" flow, no
//! polling anywhere (§15.3 discipline on both sides of the wire).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_events
//! # prints the subscription ack and every pushed event as it lands
//! ```
//!
//! The app side scripts tree diffs through the shared-script channel —
//! the mock adapter's stand-in for "a real app mutated its UI on its own
//! thread". The same conversation works over TCP
//! (`GwpServerHandle::spawn_tcp`) and from the TypeScript client
//! (`subscribe` / `nextEvent`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{EventKindV1, EventV1, GwpEvent, RequestV1};
use guiwright_proto::uds::Client;

fn main() {
    // App side: the buttons fixture + a shared script channel — updates
    // pushed here land on the session's next pump, exactly like app-side
    // UI changes riding a real adapter.
    let socket =
        std::env::temp_dir().join(format!("guiwright-events-demo-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let scripts_for_app = Arc::clone(&scripts);
        let app = std::thread::spawn(move || {
            // The app "mutates its UI" a few times, on its own cadence.
            for version in 1..=3u32 {
                std::thread::sleep(Duration::from_millis(60));
                let mut save = Node::new(Role::Button);
                save.set_label(format!("Save (v{version})"));
                save.set_bounds(accesskit::Rect {
                    x0: 16.0,
                    y0: 16.0,
                    x1: 136.0,
                    y1: 48.0,
                });
                scripts_for_app
                    .lock()
                    .expect("script lock")
                    .push(TreeUpdate {
                        nodes: vec![(NodeId(2), save)],
                        tree: None,
                        tree_id: accesskit::TreeId::ROOT,
                        focus: accesskit::NodeId(2),
                    });
            }
        });

        let handle = spawn_session_server(socket.clone(), move || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(Arc::clone(&scripts))
        })
        .expect("server binds");

        // Client side: handshake, subscribe, then read the stream. The
        // `next_event()` calls block until the driver publishes — no
        // polling loop, the event wakes the reader (§7 multiplexing).
        let mut client = Client::connect(&socket).await.expect("client connects");
        let adapter = client.handshake("wire-events-demo/0.1").await.unwrap();
        println!("attached to {} ({})", adapter.name, adapter.framework);

        client
            .subscribe(&[EventKindV1::TreeChanged])
            .await
            .expect("subscribe");
        println!("subscribed to tree_changed — waiting for the app to change");

        for _ in 0..3 {
            let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
                .await
                .expect("an event arrives per app change")
                .expect("event frame");
            let GwpEvent::V1(inner) = event;
            match inner {
                EventV1::TreeChanged { seq } => println!("event: tree_changed seq={seq}"),
                EventV1::FramePresented { seq, hash } => {
                    println!("event: frame_presented seq={seq} hash={hash:#x}")
                }
                EventV1::Log { record } => println!("event: log {record}"),
            }
        }

        // Off: the conversation returns to the plain request/response shape.
        client.unsubscribe().await.expect("unsubscribe");
        println!("unsubscribed");
        let _ = client.request(RequestV1::Bye).await;
        app.join().expect("the app thread finishes");
        handle.shutdown();
    });
}
