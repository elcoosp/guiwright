//! The identity read's state facts at handle width (m68): the m67
//! record's named standing gap — "no state flags on the identity row:
//! the checked/focused/enabled facts ride the dump rows and the kit
//! assertion vocabulary" — closed by [`ClientConn::describe_identity_state`],
//! the appended `DescribeIdentityState` / `DescribedIdentityState`
//! domain. The M36 handle pair rides in and the M68 row rides out (the
//! M64 identity row's fields plus the state three), so one
//! handle-addressed round trip answers "what does this node say about
//! itself and its state".
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_identity_state
//! # prints the state read answering resolve-all handles — the checked /
//! # unchecked / mixed tri-state beside the identity four, the disabled
//! # button's enabled: false, an unfocused node's focused: false — and
//! # the ghost handle's typed rejection
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{NodeKeyV1, SelectorV1, TextMatchV1};

/// One named row of one role — a narrowed set whose head is that row
/// alone (the m66 demo's `named_button`, generalized over the role so
/// the checkbox trio resolves the same way).
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
            .handshake("wire-client-identity-state-demo/0.1")
            .await
            .expect("handshake");

        // A resolve-all row mints the handle (the dump-row rule, M11);
        // the state read answers it — the M64 identity row's fields plus
        // the state three, one round trip instead of one question per
        // assertion.
        for (role, name) in [
            ("CheckBox", "Notifications"),
            ("CheckBox", "Dark mode"),
            ("CheckBox", "Partially installed"),
        ] {
            let rows = client
                .resolve_all(named_row(role, name))
                .await
                .expect("the set read enumerates");
            let row = rows.first().expect("the row resolves");
            let described = client
                .describe_identity_state(row.node.clone(), row.generation)
                .await
                .expect("the state read answers the handle");
            println!(
                "describe_identity_state — {} {:?} checked {:?} (generation {})",
                described.role,
                described.name.as_deref().unwrap_or_default(),
                described.checked,
                described.generation
            );
        }

        // The disabled node answers `enabled: false` and a node that
        // exposes no checked state answers NotCheckable — the total
        // read, never an error (the m38 rule at state width).
        let rows = client
            .resolve_all(named_row("Button", "Confirm"))
            .await
            .expect("the set read enumerates");
        let row = rows.first().expect("the disabled button resolves");
        let described = client
            .describe_identity_state(row.node.clone(), row.generation)
            .await
            .expect("the state read answers the handle");
        println!(
            "describe_identity_state — {} {:?} enabled {} checked {:?}",
            described.role,
            described.name.as_deref().unwrap_or_default(),
            described.enabled,
            described.checked
        );

        // An unfocused node answers false (a total fact, never an
        // error — the state facts are always named).
        let rows = client
            .resolve_all(named_row("Button", "Save"))
            .await
            .expect("the set read enumerates");
        let row = rows.first().expect("the Save button resolves");
        let described = client
            .describe_identity_state(row.node.clone(), row.generation)
            .await
            .expect("the state read answers the handle");
        println!(
            "describe_identity_state — {} {:?} focused {}",
            described.role,
            described.name.as_deref().unwrap_or_default(),
            described.focused
        );

        // A handle the tree does not know rides the typed rejection —
        // the Action boundary's honesty, the same error an action on
        // the ghost row would meet.
        let ghost = NodeKeyV1::AccessKit {
            tree: [0xa5; 16],
            node: 4242,
        };
        let err = client
            .describe_identity_state(ghost, 0)
            .await
            .expect_err("the tree cannot describe a node it does not know");
        println!("describe_identity_state ghost handle — {err}");

        handle.shutdown();
    });
}
