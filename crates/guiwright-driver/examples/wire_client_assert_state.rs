//! The state-assertion rows (m73): the kit's `expect(locator)
//! .to_be_checked(...)` / `to_be_enabled(...)` / `to_be_focused(...)`
//! half of the §8 check vocabulary, composed client-side as
//! [`ClientConn::expect_checked`] / [`ClientConn::expect_enabled`] /
//! [`ClientConn::expect_focused`] — the m68 state facts (the tri-state
//! word, the enabled boolean, the focus) as waitable assertions over the
//! m70 set-width state read, woken by the §7 event channel the way the
//! m54 content assertions wake. No wire change: the assertions ride
//! existing domains byte-for-byte; every [`ClientConn`] impl inherits
//! the rows.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_assert_state
//! # prints the tri-state asserted on entry, an assertion that stays
//! # pending until a tree update flips the checkbox (event-driven, no
//! # polling), the focus arriving with its update, and the typed
//! # timeout naming the locator, the budget and what the last
//! # evaluation found
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{NodeId, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{CheckedStateV1, SelectorV1, TextMatchV1};

/// One of the fixture's checkboxes (`getByRole(CheckBox, <name>)`): the
/// m20 trio — Notifications checked, Dark mode unchecked, Partially
/// installed mixed — the tri-state pins.
fn checkbox(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "CheckBox".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the updates onto the next
    // pump — the same shape the wire_wait conversations use): Dark mode
    // flips checked, and the focus moves to Notifications.
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        let mut lock = scripts_for_push.lock().expect("script lock");
        if let Some(update) =
            Fixture::with_checkboxes().checkbox_toggle_update(NodeId(10), accesskit::Toggled::True)
        {
            lock.push(update);
        }
        lock.push(TreeUpdate {
            nodes: vec![],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: NodeId(9),
        });
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || {
                MockAdapter::with_fixture(&Fixture::with_checkboxes())
                    .with_shared_scripts(Arc::clone(&scripts))
            },
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-client-assert-state-demo/0.1")
            .await
            .expect("handshake");

        // The tri-state word demanded exactly (m68 vocabulary, waitable):
        // Notifications is checked from the start, the row rides back as
        // the dump-row handle.
        let row = client
            .expect_checked(
                checkbox("Notifications"),
                CheckedStateV1::Checked,
                Duration::from_secs(1),
            )
            .await
            .expect("checked on entry");
        println!(
            "expect_checked → {} {:?} checked {:?}",
            row.role,
            row.name.as_deref(),
            row.checked
        );

        // The assertion is waitable: Dark mode flips checked after
        // attach, the wake fires on the pushed TreeChanged — event-driven,
        // no polling (the m54 loop's edges at state width).
        let row = client
            .expect_checked(
                checkbox("Dark mode"),
                CheckedStateV1::Checked,
                Duration::from_secs(2),
            )
            .await
            .expect("the flip arrives");
        println!(
            "expect_checked (waited) → {} {:?} checked {:?}",
            row.role,
            row.name.as_deref(),
            row.checked
        );

        // The focus arrives with its update: the node IS the tree's
        // focus right now (the core Check::Focused read's shape).
        let row = client
            .expect_focused(checkbox("Notifications"), Duration::from_secs(2))
            .await
            .expect("the focus arrives");
        println!(
            "expect_focused → {} {:?} focused {}",
            row.role,
            row.name.as_deref(),
            row.focused
        );

        // The typed timeout: Partially installed is mixed, so a Checked
        // demand stays pending — a spent budget names `be checked`, the
        // locator and the word the last evaluation found (failures name
        // names, §8.3).
        let err = client
            .expect_checked(
                checkbox("Partially installed"),
                CheckedStateV1::Checked,
                Duration::from_millis(300),
            )
            .await
            .expect_err("mixed never satisfies checked");
        println!("expect_checked (mixed) → {err}");

        handle.shutdown();
    });
}
