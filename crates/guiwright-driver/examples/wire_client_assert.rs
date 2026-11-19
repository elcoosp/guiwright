//! The content-assertion rows (m54): the kit's `expect(locator)
//! .to_have_text(...)` / `to_have_value(...)` half of the §8 check
//! vocabulary, composed client-side as [`ClientConn::expect_text`] /
//! [`ClientConn::expect_value`] — the m46 bulk reads' source orders (label
//! first vs value first) as waitable assertions over the m39 describe-all
//! round trip, woken by the §7 event channel the way the m42 wait wakes.
//! No wire change: the assertions ride existing domains byte-for-byte;
//! every [`ClientConn`] impl inherits the rows.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_assert
//! # prints the label-first and value-first reads over the fixture's
//! # Email input, an assertion that stays pending until a tree update
//! # changes the value (event-driven, no polling), and the typed
//! # timeout naming the locator, the budget and what the last
//! # evaluation found
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{SelectorV1, TextMatchV1};

/// The fixture's Email input (`getByRole(TextInput, "Email")`): its
/// accessible name "Email" and its value "ada@example.dev" are the m29/m31
/// order pins — the two reads can never silently agree on the wrong datum.
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

/// A fuzzy (substring, case-insensitive) expectation — the Playwright
/// default the kit's text locators speak.
fn want(text: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: text.into(),
        exact: false,
        ignore_case: true,
    }
}

/// The Email input's value restated (node 5): the update shape a real
/// toolkit emits when a form field changes while the test watches.
fn email_value_update(value: &str) -> TreeUpdate {
    let mut input = Node::new(Role::TextInput);
    input.set_label("Email");
    input.set_value(value);
    input.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 128.0,
        x1: 256.0,
        y1: 156.0,
    });
    TreeUpdate {
        nodes: vec![(NodeId(5), input)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    }
}

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the update onto the next
    // pump — the same shape the wire_wait conversations use).
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(email_value_update("ada@corp.example"));
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || {
                MockAdapter::with_fixture(&Fixture::buttons())
                    .with_shared_scripts(Arc::clone(&scripts))
            },
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-client-assert-demo/0.1")
            .await
            .expect("handshake");

        // The label-first read (m29 order): the Email input's accessible
        // name answers `expect_text` — its value never satisfies it while
        // the label rides.
        let row = client
            .expect_text(email_input(), want("Email"), Duration::from_secs(1))
            .await
            .expect("label-first read");
        println!(
            "expect_text  → {} (name {:?}, value {:?})",
            row.role,
            row.name.as_deref(),
            row.value.as_deref()
        );

        // The value-first read (m31 order): the form dump's flip — the
        // value answers `expect_value`, the label never satisfies it.
        let row = client
            .expect_value(
                email_input(),
                want("ada@example.dev"),
                Duration::from_secs(1),
            )
            .await
            .expect("value-first read");
        println!(
            "expect_value → {} (name {:?}, value {:?})",
            row.role,
            row.name.as_deref(),
            row.value.as_deref()
        );

        // The assertion is waitable: the value changes after attach, the
        // first evaluation is pending, the pushed TreeChanged wakes the
        // re-read — event-driven, no polling (§15.3 on both ends).
        let row = client
            .expect_value(
                email_input(),
                want("ada@corp.example"),
                Duration::from_secs(2),
            )
            .await
            .expect("the value arrives with the tree update");
        println!(
            "waited       → value now {:?} (satisfied after the wake)",
            row.value.as_deref()
        );

        // The honesty: a spent budget answers the typed timeout — the
        // locator render, the budget and the last evaluation (expected
        // and found), the kit's "failures name names" discipline (§8.3)
        // carried over the wire.
        let err = client
            .expect_value(
                email_input(),
                want("ada@elsewhere.example"),
                Duration::from_millis(250),
            )
            .await
            .expect_err("the wrong value must not satisfy");
        println!("timeout      → {err}");

        handle.shutdown();
    });
}
