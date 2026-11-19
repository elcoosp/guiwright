//! The client-side wait consumer (m42): a GWP server whose app reveals a
//! "Saved!" banner a beat after attach + a Rust client that waits for it
//! with [`ClientConn::wait_for`] — the m21 record's named follow-up, no
//! wire change (subscribe, describe-all re-reads and the event channel,
//! byte-for-byte as M12/M39 blessed them).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_wait
//! # prints the satisfying row (a ready-to-use Action handle) and the
//! # cleanup ack
//! ```
//!
//! The wait covers the states a remote suite actually needs — attached /
//! visible / hidden / stable (the kit's `WaitState` vocabulary, the m21
//! record's "a clients-side follow-on, no protocol change"). The demo
//! walks two of them: `visible` for the banner that arrives late, and
//! `hidden` for the fixture's bounds-less "Invisible" button — satisfied
//! by absence, the Playwright rule over the wire rows.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{NodeKeyV1, SelectorV1, TextMatchV1};
use guiwright_proto::uds::Client;
use guiwright_proto::wait::WaitState;

fn main() {
    // App side: the buttons fixture + a shared script channel. "The app"
    // reveals a "Saved!" label (node 9) after a beat — the same update
    // shape a real toolkit emits when a widget joins the layout.
    let socket =
        std::env::temp_dir().join(format!("guiwright-wait-demo-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let scripts_for_app = Arc::clone(&scripts);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            let mut saved = Node::new(Role::Label);
            saved.set_label("Saved!");
            saved.set_bounds(accesskit::Rect {
                x0: 16.0,
                y0: 228.0,
                x1: 216.0,
                y1: 252.0,
            });
            let mut group = Node::new(Role::Group);
            group.set_children([
                NodeId(2),
                NodeId(3),
                NodeId(4),
                NodeId(5),
                NodeId(6),
                NodeId(7),
                NodeId(9),
            ]);
            scripts_for_app
                .lock()
                .expect("script lock")
                .push(TreeUpdate {
                    nodes: vec![(NodeId(1), group), (NodeId(9), saved)],
                    tree: None,
                    tree_id: accesskit::TreeId::ROOT,
                    focus: accesskit::NodeId(0),
                });
        });

        // The fixture carries the bounds-less "Invisible" button (node 6)
        // out of the box — the not-yet-visible node the `hidden` wait is
        // satisfied by the absence of.
        let handle = spawn_session_server(socket.clone(), move || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
        })
        .expect("server binds");

        let mut client = Client::connect(&socket).await.expect("client connects");
        let adapter = client.handshake("wire-wait-demo/0.1").await.unwrap();
        println!("attached to {} ({})", adapter.name, adapter.framework);

        let banner = SelectorV1::Label(TextMatchV1 {
            text: "Saved!".into(),
            exact: true,
            ignore_case: false,
        });
        let row = client
            .wait_for(banner, WaitState::Visible, Duration::from_secs(2))
            .await
            .expect("the banner arrives");
        let row = row.expect("a positive state answers its satisfying row");
        let node = match &row.node {
            NodeKeyV1::AccessKit { node, .. } => *node,
            other @ NodeKeyV1::Logical { .. } => {
                panic!("expected an AccessKit handle, got {other:?}")
            }
        };
        println!(
            "wait visible — satisfied by accesskit#{node} {:?} bounds={:?}",
            row.name.as_deref(),
            row.bounds
        );
        println!("(the row is a live Action handle — click()/fill() aim it directly)");

        // The hidden state composes the same walk: the fixture's
        // "Invisible" button is attached but bounds-less, so nothing
        // visible answers the selector — absence is the satisfying fact
        // (Playwright's satisfied-by-absence rule over the wire rows).
        let spinner = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Invisible".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        let gone = client
            .wait_for(spinner, WaitState::Hidden, Duration::from_secs(1))
            .await
            .expect("the hidden read");
        if gone.is_none() {
            println!("wait hidden — satisfied by absence (nothing visible answers the selector)");
        }

        let _ = handle; // the session outlives the demo conversation
    });
}
