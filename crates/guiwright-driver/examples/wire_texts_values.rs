//! The bulk-content reads over the wire (m46 — the m41 record's
//! Rust-client half): [`ClientConn::all_text_contents`] and
//! [`ClientConn::all_values`] derive the kit's m29/m31 source orders over
//! the M39 describe-all round trip — one read per set, no wire change
//! (the rows ride `DescribeAll` byte-for-byte as M39 blessed them). The
//! same vocabulary the TypeScript client carries as `allTextContents()` /
//! `allValues()` (m41) and the REPL as `texts` / `values` (m40), now on
//! the Rust seam: one round trip, every [`ClientConn`] impl inherits it.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_texts_values
//! # prints the button set's texts (label first, hidden included), the
//! # input's text vs. value (the flipped source orders), and the empty
//! # set's total answer
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::SelectorV1;
use guiwright_proto::{ClientConn, DuplexClient};

fn role(role: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: None,
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-texts-values-demo/0.1")
            .await
            .expect("handshake");

        // The text read (m29 order): label first, document order, the
        // hidden "Invisible" button included — the rows get_by_text
        // matches, derived from the describe-all walk client-side.
        let texts = client
            .all_text_contents(role("Button"))
            .await
            .expect("the button set's texts");
        println!("buttons — texts: {texts:?}");

        // The two source orders on one row: the Email input answers its
        // label to the text read and its value to the form dump — the
        // m31 rule that the reads can never silently agree on the wrong
        // datum.
        let input_text = client
            .all_text_contents(role("TextInput"))
            .await
            .expect("the input's text");
        let input_value = client
            .all_values(role("TextInput"))
            .await
            .expect("the input's value");
        println!("input — text {input_text:?} vs. value {input_value:?} (the m29/m31 orders)");

        // The read is total: an empty set answers an empty list, never
        // an error (the m30 snapshot rule over the m39 domain).
        let empty = client
            .all_values(SelectorV1::TestId("ghost".into()))
            .await
            .expect("the empty set read");
        println!("empty set → {} values (total, never an error)", empty.len());

        handle.shutdown();
    });
}
