//! The §7 state-change events end to end (the m20 record's remaining-gaps
//! list: *"`setChecked`-state change events (assert via subscription
//! instead of polling the tree) would ride the M12 event channel with a
//! new `EventKindV1`*): a client subscribes to `StateChange`, flips the
//! fixture's Dark-mode checkbox through the wire `Action` path's check
//! vocabulary, and the subscription answers — the node and the tri-state
//! it now carries — instead of a tree poll. A follow-up check that lands
//! without moving the fact publishes nothing (the honest contract's
//! other half).
//!
//! ```text
//! cargo run -p guiwright-driver --example state_change_event
//! # prints the resolved handle, the ActionDone frame, the StateChange
//! # event naming the node with `checked`, and the quiet no-move check
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{NodeId, Toggled, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{
    EventKindV1, EventV1, GwpEvent, RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1,
};
use guiwright_proto::uds::Client;

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async {
        // App side: the checkbox trio fixture, with the shared-script
        // channel the "app thread" pushes the widget mutation into (the
        // shape a real adapter's act() carries onto its tree).
        let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
        let scripts = Arc::clone(&shared);
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("state_change_demo.sock");
        let handle = spawn_session_server(path.clone(), move || {
            MockAdapter::with_fixture(&Fixture::with_checkboxes()).with_shared_scripts(scripts)
        })
        .expect("server binds");

        let mut client = Client::connect(&path).await.expect("client connects");
        client.handshake("guiwright-demo/0.1").await.unwrap();

        // Resolve the Dark-mode checkbox (NodeId 10, Toggled::False).
        let response = client
            .request(RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "CheckBox".into(),
                    name: Some(TextMatchV1 {
                        text: "Dark mode".into(),
                        exact: false,
                        ignore_case: false,
                    }),
                },
                timeout_ms: 0,
            })
            .await
            .expect("resolve");
        let ResponseV1::Resolved {
            node, generation, ..
        } = response
        else {
            panic!("expected a Resolved reply, got {response:?}");
        };
        println!("resolved: {node:?} @ generation {generation}");

        // Subscribe to the state-change kind, then flip the fact: the
        // widget mutation rides the action's pump (the same rendezvous
        // the §15.4 gate applies the action at), and the subscription
        // answers instead of a tree poll.
        client
            .subscribe(&[EventKindV1::StateChange])
            .await
            .expect("subscribe");
        let toggle = Fixture::with_checkboxes()
            .checkbox_toggle_update(NodeId(10), Toggled::True)
            .expect("Dark mode is a trio node");
        shared.lock().expect("scripts lock").push(toggle);
        let ack = client
            .request(RequestV1::Action {
                node,
                generation,
                action: SemActionV1::Check,
                force: false,
                timeout_ms: 5000,
            })
            .await
            .expect("the moving check lands");
        let ResponseV1::ActionDone { frame_seq, detail } = ack else {
            panic!("expected an ActionDone, got {ack:?}");
        };
        println!("check applied at frame {frame_seq} ({detail})");

        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("the state change arrives")
            .expect("event frame");
        let GwpEvent::V1(EventV1::StateChange { node, checked }) = event else {
            panic!("expected a state-change event, got {event:?}");
        };
        println!("state change: {node:?} now carries {checked:?}");

        // The follow-up check lands without moving the fact — nothing
        // publishes: a state-change event without a state change would
        // be a lie.
        client
            .request(RequestV1::Action {
                node,
                generation,
                action: SemActionV1::Check,
                force: false,
                timeout_ms: 5000,
            })
            .await
            .expect("the quiet check lands");
        let quiet = tokio::time::timeout(Duration::from_millis(400), client.next_event()).await;
        match quiet {
            Err(_) => println!("no-move check published nothing — the honest contract"),
            Ok(other) => panic!("expected quiet, got {other:?}"),
        }

        handle.shutdown();
    });
}
