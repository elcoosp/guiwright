//! The node-describe wire domain (M36): the m32 record's standing gap —
//! "role/name would ride a node-describe domain that does not exist yet"
//! — closed as `Describe` → `Described` over GWP, with the typed
//! [`ClientConn::describe`] sugar. "What IS the node this handle points
//! at" is the debugging read the REPL's `hit` rows and viewer-style
//! clients speak: the handle is the dump-row form (a resolve row, a hit
//! row, a dump row — each is directly usable as the `Describe` node, and
//! as the `Action` node), the reply carries role/name/value/bounds, and
//! the generation guard is the Action boundary's — a stale handle reads
//! the typed `StaleGeneration` rejection, an unknown one `UnknownNode`.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_describe
//! # prints the Save button's identity facts from its resolve row, the
//! # value an input carries, and the stale-handle rejection
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{DescribeV1, NodeKeyV1, RequestV1, ResponseV1, SelectorV1};
use guiwright_proto::{ClientConn, DuplexClient};

fn named(role: &str, name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: Some(guiwright_proto::schema::TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

async fn resolve(client: &mut DuplexClient, selector: SelectorV1) -> (NodeKeyV1, u32) {
    match client
        .request(RequestV1::Resolve {
            selector,
            timeout_ms: 2000,
        })
        .await
        .expect("resolve round-trip")
    {
        ResponseV1::Resolved {
            node, generation, ..
        } => (node, generation),
        other => panic!("expected Resolved, got {other:?}"),
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
            .expect("client connects");
        client
            .handshake("wire-describe-demo/0.1")
            .await
            .expect("handshake");

        // Resolve mints the handle; describe reads what it points at —
        // the facts the dump row carries, one node instead of a walk.
        let (node, generation) = resolve(&mut client, named("Button", "Save")).await;
        let row = client
            .describe(node.clone(), generation)
            .await
            .expect("the read");
        println!(
            "resolved → {} {:?} (generation {}) bounds {:?}",
            row.role,
            row.name.as_deref().unwrap_or("(unnamed)"),
            row.generation,
            row.bounds
        );

        // The value an input carries rides the same read — the fact the
        // getByValue locators match (§8.1).
        let (email, generation) = resolve(&mut client, named("TextInput", "Email")).await;
        let row = client.describe(email, generation).await.expect("the read");
        println!(
            "resolved → {} {:?} value {:?}",
            row.role,
            row.name.as_deref().unwrap_or("(unnamed)"),
            row.value.as_deref().unwrap_or("(none)")
        );

        // The dump-row rule cuts both ways: a described handle is
        // directly usable as the Action node — hit the described node,
        // no second resolve.
        let (save, generation) = resolve(&mut client, named("Button", "Save")).await;
        let row = client
            .describe(save.clone(), generation)
            .await
            .expect("the read");
        let done = client
            .request(RequestV1::Action {
                node: save,
                generation: row.generation,
                action: guiwright_proto::schema::SemActionV1::Click,
                force: false,
                timeout_ms: 2000,
            })
            .await
            .expect("action on the described handle");
        if let ResponseV1::ActionDone { detail, frame_seq } = done {
            println!("describe row → action: {detail} (frame {frame_seq})");
        }

        // The Action boundary is the describe boundary: a handle minted
        // before its node left the tree reads the typed rejection.
        let scripts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let queued = scripts.clone();
        let (handle2, connect2) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(queued),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client2 = DuplexClient::connect(&connect2)
            .await
            .expect("client connects");
        client2
            .handshake("wire-describe-demo/0.1")
            .await
            .expect("handshake");
        let (stale, generation) = resolve(&mut client2, named("Button", "Save")).await;
        scripts
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().remove_save_update());
        let err = client2
            .request(RequestV1::Describe(DescribeV1 {
                node: stale,
                generation,
            }))
            .await
            .expect_err("the stale handle cannot describe");
        println!("stale handle → {err}");
        handle2.shutdown();

        handle.shutdown();
    });
}
