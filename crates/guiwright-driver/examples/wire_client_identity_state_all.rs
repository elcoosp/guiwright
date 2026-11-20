//! The identity read's state facts at set width (m70): the m68/m69
//! records' named standing gap — "no state facts at *set* width: the
//! `DescribeAllIdentity` rows stay identity-row shaped; a state-fact set
//! widening lands when a consumer asks — the same append-only move, its
//! own domain" — closed by [`ClientConn::describe_all_identity_state`],
//! the appended `DescribeAllIdentityState` / `DescribedAllIdentityState`
//! domain. The M39/m64 walk rides in and the M68 row rides out per
//! match, so one round trip answers "what does every node in this set
//! say about itself and its state" where the handle-width read answers
//! one handle at a time.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_identity_state_all
//! # prints the checkbox walk answering the tri-state per row in document
//! # order — the checked / unchecked / mixed vocabulary beside the
//! # identity four — then the disabled button's enabled: false and the
//! # empty set's honest empty list
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{SelectorV1, TextMatchV1};

/// One named row of one role — a narrowed set whose head is that row
/// alone (the m68 demo's shape, at set width).
fn named_row(role: &str, name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::with_state()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");

        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-client-identity-state-all-demo/0.1")
            .await
            .expect("handshake");

        // The whole checkbox match set rides one round trip, document
        // order — the m68 row per match (the identity four, the
        // description fact plus the state three).
        let rows = client
            .describe_all_identity_state(SelectorV1::Role {
                role: "CheckBox".into(),
                name: None,
            })
            .await
            .expect("the set read enumerates");
        println!(
            "describe_all_identity_state(CheckBox) — {} rows, document order:",
            rows.len()
        );
        for row in &rows {
            println!(
                "  {} {:?} focused {} enabled {} checked {:?}",
                row.role,
                row.name.as_deref().unwrap_or_default(),
                row.focused,
                row.enabled,
                row.checked
            );
        }

        // A narrowed walk answers one disabled row — the total read,
        // never an error (the m38 rule at set width).
        let rows = client
            .describe_all_identity_state(named_row("Button", "Confirm"))
            .await
            .expect("the set read enumerates");
        let row = rows.first().expect("the disabled button resolves");
        println!(
            "describe_all_identity_state(Confirm) — enabled {} checked {:?}",
            row.enabled, row.checked
        );

        // An empty set answers with an empty list — the set read is
        // total, never an error (the m30 rule).
        let rows = client
            .describe_all_identity_state(named_row("CheckBox", "No such toggle"))
            .await
            .expect("the set read answers");
        println!(
            "describe_all_identity_state(no match) — {} rows (the honest empty list)",
            rows.len()
        );

        handle.shutdown();
    });
}
