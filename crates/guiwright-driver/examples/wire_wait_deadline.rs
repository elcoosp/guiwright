//! The spent-wait consumer (m44 — the m42 record's named conn-level
//! follow-up): a wait that expires leaves the SAME connection healthy.
//! [`ClientConn::wait_for`]'s deadline races the §7 event read with
//! `tokio::time::timeout` — the buffered client read half (m44) keeps
//! whatever a dropped read consumed, so the conversation continues over
//! the door that served the spent wait. No wire change (the read half is
//! byte-level state behind the same `ClientConn` vocabulary).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_wait_deadline
//! # prints the typed timeout (state, locator, last evaluation), then the
//! # follow-up round trips the same connection serves afterwards
//! ```
//!
//! The demo walks the debugging-session shape: wait for a node that
//! never appears (a mistyped locator, a banner that never renders), read
//! the typed timeout, then keep working on the same connection — ping,
//! a full set read, and a second wait that expires the same honest way.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{SelectorV1, TextMatchV1};
use guiwright_proto::uds::Client;
use guiwright_proto::wait::WaitState;

fn main() {
    let socket = std::env::temp_dir().join(format!(
        "guiwright-wait-deadline-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&socket);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // The buttons fixture: a "Save" button that never changes — the
        // "Ghost" button the wait below asks for never exists, so the
        // wait's deadline is the honest answer.
        let handle = spawn_session_server(socket.clone(), || {
            MockAdapter::with_fixture(&Fixture::buttons())
        })
        .expect("server binds");

        let mut client = Client::connect(&socket).await.expect("client connects");
        let adapter = client.handshake("wire-wait-deadline/0.1").await.unwrap();
        println!("attached to {} ({})", adapter.name, adapter.framework);

        let ghost = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Ghost".into(),
                exact: true,
                ignore_case: false,
            }),
        };

        // The wait expires: the typed timeout names the state, the
        // locator and the last evaluation (the failures-name-names
        // discipline at the client seam), and the subscription window is
        // dropped on return (a consumer that never drains its events
        // must not stay subscribed, §7/M12).
        let err = client
            .wait_for(
                ghost.clone(),
                WaitState::Visible,
                Duration::from_millis(150),
            )
            .await
            .expect_err("a button that never exists cannot become visible");
        match &err {
            GwpError::WaitTimeout {
                state,
                selector,
                timeout_ms,
                last,
            } => println!("wait {state} timed out after {timeout_ms} ms — {selector} ({last})"),
            other => panic!("expected the typed wait timeout, got {other:?}"),
        }

        // The spent wait left the connection healthy: the same door
        // answers a liveness probe and a full set read (the buffered
        // read half kept whatever the deadline's dropped read consumed).
        client.ping().await.expect("the connection still answers");
        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        let rows = client
            .describe_all(save)
            .await
            .expect("the set read rides the same door");
        println!(
            "after the spent wait: ping ok, describe_all answered {} row(s)",
            rows.len()
        );

        // A second wait starts clean and expires the same honest way —
        // no stale subscription, no stale events.
        let err = client
            .wait_for(ghost, WaitState::Visible, Duration::from_millis(150))
            .await
            .expect_err("the second wait expires the same way");
        assert!(matches!(err, GwpError::WaitTimeout { .. }), "{err:?}");
        println!("the second wait expired cleanly — the door keeps serving");

        let _ = handle; // the session outlives the demo conversation
    });
}
