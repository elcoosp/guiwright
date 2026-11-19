//! The count-assertion rows (m59): the kit's `expect(locator)
//! .to_have_count(...)` family (the M22 exact form + the M25 one-sided
//! comparators) over any door, composed client-side as
//! [`ClientConn::expect_count`] / [`ClientConn::expect_count_greater_than`]
//! / [`ClientConn::expect_count_less_than`] — the m54 content assertions'
//! wake-loop composition riding the M30 resolve-all round trip instead of
//! describe-all, woken by the §7 event channel the way the m42 wait wakes.
//! No wire change: the assertion rides existing domains byte-for-byte;
//! every [`ClientConn`] impl inherits the rows.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_count
//! # prints the whole-set and narrowed count reads over the fixture's
//! # four buttons, an assertion that stays pending until a tree update
//! # joins a fifth button (event-driven, no polling), the comparator
//! # forms, and the typed timeout naming the locator, the budget and
//! # the count the last evaluation found
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::TreeUpdate;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::SelectorV1;

/// Every `Button` the fixture carries — the set the count vocabulary
/// speaks about (strict mode never consulted, the m22 record's set read).
fn any_button() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: None,
    }
}

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the update onto the next
    // pump — the same shape the wire_wait conversations use): a fifth
    // button joins under the fixture's group.
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().add_button_update("Later"));
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
            .handshake("wire-client-count-demo/0.1")
            .await
            .expect("handshake");

        // The exact form (M22): the whole match set answers with the
        // found count — the fact the assertion speaks.
        let found = client
            .expect_count(any_button(), 4, Duration::from_secs(1))
            .await
            .expect("exact count");
        println!("expect_count → found {found} (the fixture's four buttons)");

        // The assertion is waitable: the fifth button joins after
        // attach, the first evaluation is pending (found 4), the pushed
        // TreeChanged wakes the re-read — event-driven, no polling
        // (§15.3 on both ends).
        let found = client
            .expect_count(any_button(), 5, Duration::from_secs(2))
            .await
            .expect("the set grows with the tree update");
        println!("waited       → found {found} (satisfied after the wake)");

        // The comparators (M25): the same loop, the one-sided forms.
        let found = client
            .expect_count_greater_than(any_button(), 4, Duration::from_secs(1))
            .await
            .expect("comparator greater-than");
        println!("greater_than → found {found} (> 4)");
        let found = client
            .expect_count_less_than(any_button(), 9, Duration::from_secs(1))
            .await
            .expect("comparator less-than");
        println!("less_than    → found {found} (< 9)");

        // The honesty: a spent budget answers the typed timeout — the
        // locator render, the budget and the last evaluation (the
        // demanded comparison and the count the last evaluation found),
        // the kit's "failures name names" discipline (§8.3) carried over
        // the wire.
        let err = client
            .expect_count(any_button(), 99, Duration::from_millis(250))
            .await
            .expect_err("the wrong count must not satisfy");
        println!("timeout      → {err}");

        handle.shutdown();
    });
}
