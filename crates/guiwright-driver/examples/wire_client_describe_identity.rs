//! The single-node identity describe read (m66): the m64 record's named
//! standing gap — the M36 `Describe` read keeps its identity-four
//! encoding and the description fact rides the identity *set* read only
//! — closed by [`ClientConn::describe_identity`], the appended
//! `DescribeIdentity` / `DescribedIdentity` domain. The M36 handle pair
//! rides in and the M64 identity row rides out, so one handle-addressed
//! round trip answers "what does this node say about itself" where the
//! set read walks.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_describe_identity
//! # prints the identity read answering a resolve-all handle — the
//! # description fact named beside the identity four — an undescribed
//! # node riding None (the name and the value never answer a
//! # description read), and the ghost handle's typed rejection
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{NodeKeyV1, SelectorV1, TextMatchV1};

/// One named button — a narrowed set whose head is that row alone.
fn named_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
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
            || MockAdapter::with_fixture(&Fixture::with_description()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");

        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-client-describe-identity-demo/0.1")
            .await
            .expect("handshake");

        // A resolve-all row mints the handle (the dump-row rule, M11);
        // the identity read answers it — the M64 row's identity four
        // plus the description fact, one round trip instead of the
        // whole-set walk.
        let rows = client
            .resolve_all(named_button("Install"))
            .await
            .expect("the set read enumerates");
        let row = rows.first().expect("the Install button resolves");
        let described = client
            .describe_identity(row.node.clone(), row.generation)
            .await
            .expect("the identity read answers the handle");
        println!(
            "describe_identity — {} {:?} value {:?} (generation {})",
            described.role,
            described.name.as_deref().unwrap_or_default(),
            described.value.as_deref(),
            described.generation
        );
        println!(
            "  description {:?} — the fact the M36 encoding left out",
            described.description.as_deref().unwrap_or_default()
        );

        // An undescribed node rides None (a total read, never an error —
        // the m38 rule: the name and the value never answer a
        // description read).
        let rows = client
            .resolve_all(named_button("Save"))
            .await
            .expect("the set read enumerates");
        let row = rows.first().expect("the Save button resolves");
        let described = client
            .describe_identity(row.node.clone(), row.generation)
            .await
            .expect("the identity read answers the handle");
        println!(
            "describe_identity — {} {:?} description {:?}",
            described.role,
            described.name.as_deref().unwrap_or_default(),
            described.description
        );

        // A handle the tree does not know rides the typed rejection —
        // the Action boundary's honesty, the same error an action on
        // the ghost row would meet.
        let ghost = NodeKeyV1::AccessKit {
            tree: [0xa5; 16],
            node: 4242,
        };
        let err = client
            .describe_identity(ghost, 0)
            .await
            .expect_err("the tree cannot describe a node it does not know");
        println!("describe_identity ghost handle — {err}");

        handle.shutdown();
    });
}
