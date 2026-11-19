//! The identity-assertion rows (m62): the kit's `expect(locator)
//! .to_have_role(...)` / `to_have_accessible_name(...)` vocabulary (M38)
//! over any door, composed client-side as [`ClientConn::expect_role`] /
//! [`ClientConn::expect_name`] — the m54 content assertions' wake-loop
//! composition evaluating the identity facts the M39 describe-all rows
//! already carry. No wire change: the rows ride existing domains
//! byte-for-byte; every [`ClientConn`] impl inherits them.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_identity
//! # prints the role and label-only name assertions answering their row
//! # over the fixture's buttons, an assertion that stays pending until a
//! # scripted rename lands (event-driven, no polling), and the typed
//! # timeout naming `have role` and the role the last evaluation saw
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::TreeUpdate;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{SelectorV1, TextMatchV1};

/// Every `Button` the fixture carries — the set whose head row the
/// identity assertions pin (the m54 set-head rule, document order).
fn any_button() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: None,
    }
}

/// The fixture's Email input — the label-only name read below (the role
/// locator narrows the set to the row alone).
fn email_input() -> SelectorV1 {
    SelectorV1::Role {
        role: "TextInput".into(),
        name: Some(TextMatchV1 {
            text: "Email".into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

/// A fuzzy button locator — the set the rename conversation rides (the
/// head row stays the Save button across the rename).
fn fuzzy_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the update onto the next
    // pump): the Save button renames — the pending name assertion below
    // wakes on the tree event.
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().rename_save_update());
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");

        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-client-identity-demo/0.1")
            .await
            .expect("handshake");

        // Satisfied on entry: the role compares exactly (the AccessKit
        // debug name the describe rows print), the label-only name rides
        // the REPL's matcher posture here — the m38 vocabulary's
        // narrower read, the value never answering.
        let row = client
            .expect_role(any_button(), "Button".into(), Duration::from_secs(2))
            .await
            .expect("the head row's role answers");
        println!("expect_role — {} (generation {})", row.role, row.generation);

        let row = client
            .expect_name(
                email_input(),
                TextMatchV1 {
                    text: "Email".into(),
                    exact: false,
                    ignore_case: true,
                },
                Duration::from_secs(2),
            )
            .await
            .expect("the label-only name answers");
        println!(
            "expect_name — {:?} (generation {})",
            row.name.as_deref().unwrap_or_default(),
            row.generation
        );

        // Pending until the scripted rename lands: the assertion rides
        // the seam's wake loop — tree event, re-read, answered.
        let row = client
            .expect_name(
                fuzzy_button("Sav"),
                TextMatchV1 {
                    text: "Save All".into(),
                    exact: false,
                    ignore_case: true,
                },
                Duration::from_secs(2),
            )
            .await
            .expect("the rename wakes the re-read");
        println!(
            "expect_name after the rename — {:?} (generation {})",
            row.name.as_deref().unwrap_or_default(),
            row.generation
        );

        // A spent budget prints the typed timeout — `have role`, the
        // locator, the role the last evaluation saw.
        let err = client
            .expect_role(any_button(), "TextInput".into(), Duration::from_millis(200))
            .await
            .expect_err("the head row is a Button, not a TextInput");
        println!("expect_role mismatch — {err}");

        handle.shutdown();
    });
}
