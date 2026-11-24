//! The app-fed log tail (the m14 record's named knob — docs/m14-log-tail.md
//! §5: *"an app-side `ServerOptions.log_tap(tap)` knob is a later append if
//! a consumer shows up"*): the app builds a [`LogTap`], spawns the session
//! server with `ServerOptions::log_tap`, keeps a clone of the tap, and
//! feeds its own records onto the same `EventV1::Log` stream the session
//! thread's `tracing` records ride. The knob implies the tail — no
//! separate `log_tail` bool to keep honest.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_app_logs
//! # prints the subscription ack, the session's own registration record
//! # and the app's fed records as they land, one logs.ndjson-shape JSON
//! # line per event (§12's vocabulary across the wire)
//! ```
//!
//! The producer end is cloneable (`LogTap`'s documented contract, now in
//! the compiler): the app keeps clones for feeding while the original's
//! paired tail serves the session. A clone carries no tail — a clone
//! passed to a second spawn refuses with the typed `GwpError::Protocol`
//! (one tap serves one session), never a silent tail-less run.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::{LOG_TAIL_CAPACITY, LogTap, ServerOptions, spawn_session_server_with};
use guiwright_proto::schema::{EventKindV1, EventV1, GwpEvent, RequestV1, SubscribeV1};
use guiwright_proto::uds::Client;

fn main() {
    let socket = std::env::temp_dir().join(format!(
        "guiwright-app-logs-demo-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&socket);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // App side: build the tap, keep a clone for feeding, spawn the
        // session with the original (its paired tail leaves at spawn).
        let tap = LogTap::new(LOG_TAIL_CAPACITY);
        let feeder = tap.clone();
        let handle = spawn_session_server_with(
            socket.clone(),
            || MockAdapter::with_fixture(&Fixture::buttons()),
            ServerOptions {
                log_tap: Some(tap),
                ..ServerOptions::default()
            },
        )
        .expect("the tap's tail serves the spawn");

        let mut client = Client::connect(&socket).await.expect("client connects");
        let adapter = client.handshake("wire-app-logs-demo/0.1").await.unwrap();
        println!("attached to {} ({})", adapter.name, adapter.framework);

        client
            .subscribe(&[EventKindV1::Log])
            .await
            .expect("the app-fed tap implies the tail");
        println!("subscribed to log — the session and the app both feed below");

        // The subscription lifecycle emits the session's own record first.
        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("the registration record arrives")
            .expect("event frame");
        print_record("subscribe", &event);

        // App side: feed the app's own records — pre-formatted
        // logs.ndjson-shape JSON lines (§12's vocabulary), lossy by
        // contract (a full tail drops and counts, never blocks).
        feeder.send(
            serde_json::json!({
                "ts_ns": 0u64,
                "level": "INFO",
                "target": "my_app::lifecycle",
                "pid": std::process::id(),
                "message": "the app booted"
            })
            .to_string(),
        );
        feeder.send(
            serde_json::json!({
                "ts_ns": 1u64,
                "level": "INFO",
                "target": "my_app::lifecycle",
                "pid": std::process::id(),
                "message": "the app's own milestone"
            })
            .to_string(),
        );

        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
                .await
                .expect("the app's records arrive")
                .expect("event frame");
            print_record("app-fed", &event);
        }

        // The session's thread keeps feeding too: a re-subscribe
        // registers again, one record beside the app's.
        let _ = client
            .request(RequestV1::Subscribe(SubscribeV1::On {
                kinds: vec![EventKindV1::Log],
            }))
            .await;
        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("the re-subscription record arrives")
            .expect("event frame");
        print_record("re-subscribe", &event);

        handle.shutdown();
    });
}

/// One record as the wire carries it: one `logs.ndjson`-shape JSON line.
fn print_record(tag: &str, event: &GwpEvent) {
    let GwpEvent::V1(EventV1::Log { record }) = event else {
        panic!("expected a log event, got {event:?}");
    };
    let parsed: serde_json::Value = serde_json::from_str(record).expect("one JSON line");
    println!(
        "{tag}: {} {} — {}",
        parsed["level"].as_str().unwrap_or("?"),
        parsed["target"].as_str().unwrap_or("?"),
        parsed["message"].as_str().unwrap_or("?"),
    );
}
