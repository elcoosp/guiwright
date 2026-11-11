//! The event channel's log half (the log-tail milestone): a GWP server
//! spawned with `ServerOptions::log_tail` — the per-session `tracing` tap
//! — and a Rust client that subscribes to the `Log` kind and reads the
//! session thread's records as they land, one `logs.ndjson`-shape JSON
//! line per event (§12's vocabulary across the wire).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_logs
//! # prints the subscription ack and every log record as it lands
//! ```
//!
//! The records here are the driver's own — the subscription lifecycle
//! (`event subscription registered` / `dropped`) is emitted on the session
//! thread, so the flow is deterministic without scripting an app. The same
//! conversation works over TCP (`GwpServerHandle::spawn_tcp_with`) and
//! from the TypeScript client (`subscribe(["log"])` / `nextEvent()`); a
//! session spawned without the tap answers the `Log` subscription with a
//! typed `Unsupported` — the honest capability answer.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::{ServerOptions, spawn_session_server_with};
use guiwright_proto::schema::{EventKindV1, EventV1, GwpEvent, RequestV1, SubscribeV1};
use guiwright_proto::uds::Client;

fn main() {
    let socket =
        std::env::temp_dir().join(format!("guiwright-logs-demo-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // App side: the buttons fixture, spawned WITH the log tail. The
        // tap is a thread-local dispatcher on the session thread — other
        // threads keep the process subscriber, and the tail is a bounded
        // live tap (§15.6): records emitted before a subscription are
        // discarded, never replayed.
        let handle = spawn_session_server_with(
            socket.clone(),
            || MockAdapter::with_fixture(&Fixture::buttons()),
            ServerOptions {
                log_tail: true,
                ..ServerOptions::default()
            },
        )
        .expect("server binds");

        let mut client = Client::connect(&socket).await.expect("client connects");
        let adapter = client.handshake("wire-logs-demo/0.1").await.unwrap();
        println!("attached to {} ({})", adapter.name, adapter.framework);

        client
            .subscribe(&[EventKindV1::Log])
            .await
            .expect("the tail session serves the Log kind");
        println!("subscribed to log — the session tap's records stream below");

        // The subscription itself is logged on the session thread: the
        // first record on the stream is its own registration.
        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("the registration record arrives")
            .expect("event frame");
        let GwpEvent::V1(EventV1::Log { record }) = event else {
            panic!("expected a log event, got {event:?}");
        };
        // One JSON line per record (the logs.ndjson vocabulary):
        // ts_ns, level, target, pid, message + structured fields.
        let parsed: serde_json::Value =
            serde_json::from_str(&record).expect("one JSON line per record");
        println!(
            "subscribe: {} {} — {}",
            parsed["level"].as_str().unwrap_or("?"),
            parsed["target"].as_str().unwrap_or("?"),
            parsed["message"].as_str().unwrap_or("?"),
        );

        // The lifecycle keeps logging while the subscription is on:
        // a re-subscribe registers again, one record.
        let _ = client
            .request(RequestV1::Subscribe(SubscribeV1::On {
                kinds: vec![EventKindV1::Log],
            }))
            .await;
        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("a record arrives per registration")
            .expect("event frame");
        let GwpEvent::V1(EventV1::Log { record }) = event else {
            panic!("expected a log event, got {event:?}");
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&record).expect("one JSON line per record");
        println!(
            "re-subscribe: {} {} — {}",
            parsed["level"].as_str().unwrap_or("?"),
            parsed["target"].as_str().unwrap_or("?"),
            parsed["message"].as_str().unwrap_or("?"),
        );

        // Unsubscribe: the drop is logged on the session thread AFTER the
        // subscription is gone, so the live tap discards it — no
        // subscriber, no replay (§15.6: the tail is diagnostics, not
        // evidence). Nothing more arrives on this connection.
        client.unsubscribe().await.expect("unsubscribe");
        let next = tokio::time::timeout(Duration::from_millis(400), client.next_event()).await;
        assert!(next.is_err(), "no records after `Off`: {next:?}");
        println!("unsubscribed — no further records (the live tap never replays)");

        let _ = client.request(RequestV1::Bye).await;
        handle.shutdown();
    });
}
