//! The description-assertion rows (m64): the kit's
//! `expect(locator).to_have_accessible_description(...)` vocabulary (M38)
//! over any door, composed client-side as
//! [`ClientConn::expect_description`] — the m62 identity rows' third
//! fact, riding the appended `DescribeAllIdentity` /
//! `DescribedAllIdentity` domain (the M39 encodings stay byte-pinned;
//! the widening is its own domain reusing the `DescribeAll` payload).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_description
//! # prints the description assertion answering the fixture's Install
//! # button (the fact the identity four ride beside), an assertion that
//! # stays pending until a scripted restatement lands (event-driven, no
//! # polling), and the typed timeout naming `have description` and the
//! # description the last evaluation saw
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

/// The fixture's Install button — the described row (the m38 extension
/// band): `"Installs the latest build"` is the fact the description
/// assertion speaks, and neither the name nor the value answers for it.
fn install_button() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: "Install".into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

/// A fuzzy button locator — the set the restatement conversation rides
/// (the head row stays the Save button across the scripted update).
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

fn fuzzy(pattern: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: pattern.into(),
        exact: false,
        ignore_case: true,
    }
}

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the update onto the next
    // pump): the Save button gains its description — the pending
    // assertion below wakes on the tree event (descriptions arrive with
    // the node's restatement, the m38 rule).
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::with_description().describe_save_update());
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || {
                MockAdapter::with_fixture(&Fixture::with_description()).with_shared_scripts(scripts)
            },
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");

        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-client-description-demo/0.1")
            .await
            .expect("handshake");

        // Satisfied on entry: the description answers the matcher — the
        // satisfying row is the full identity row (the m64 row is a
        // dump-row handle with the fact beside it), directly usable as
        // an Action handle.
        let row = client
            .expect_description(install_button(), fuzzy("Installs"), Duration::from_secs(2))
            .await
            .expect("the description answers on entry");
        println!(
            "expect_description — {:?} (generation {})",
            row.description.as_deref().unwrap_or_default(),
            row.generation
        );
        println!(
            "  the identity four ride beside it: {:?} {:?}",
            row.role,
            row.name.as_deref().unwrap_or_default()
        );

        // Pending until the scripted restatement lands: the assertion
        // rides the seam's wake loop — tree event, re-read, answered.
        let row = client
            .expect_description(
                fuzzy_button("Sav"),
                fuzzy("Saves every change"),
                Duration::from_secs(2),
            )
            .await
            .expect("the restatement wakes the re-read");
        println!(
            "expect_description after the restatement — {:?} (generation {})",
            row.description.as_deref().unwrap_or_default(),
            row.generation
        );

        // A spent budget prints the typed timeout — `have description`,
        // the locator, the description the last evaluation saw.
        let err = client
            .expect_description(install_button(), fuzzy("Ghost"), Duration::from_millis(200))
            .await
            .expect_err("the Install button describes the install, not Ghost");
        println!("expect_description mismatch — {err}");

        handle.shutdown();
    });
}
