//! The resolve-all wire domain (M30 — the m22/m26/m29 records' named
//! protocol half): the §8.2 set read — `locator.count()` / `all()`'s
//! `resolve_all` walk — as `ResolveAll` → `ResolvedAll` over GWP, with
//! the typed [`ClientConn::resolve_all`] sugar. The four-button fixture
//! is exactly the ambiguity strict mode rejects single resolves with,
//! which is the set read's data; the two-window fixture shows the
//! connection's window scope narrowing the walk. The same conversation
//! the TypeScript client speaks (`resolveAll()`,
//! `clients/typescript/scripts/e2e.ts`).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_bulk_read
//! # prints the enumeration in document order, the window-scoped set,
//! # and an action on an enumerated row
//! ```

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{NodeKeyV1, RequestV1, ResponseV1, SelectorV1, SemActionV1};
use guiwright_proto::{ClientConn, DuplexClient};

fn role(name: Option<&str>) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: name.map(|n| guiwright_proto::schema::TextMatchV1 {
            text: n.into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

fn tree_of(match_: &guiwright_proto::schema::ResolvedMatchV1) -> String {
    match &match_.node {
        NodeKeyV1::AccessKit { tree, node } => {
            let hex: String = tree.iter().map(|b| format!("{b:02x}")).collect();
            format!("{}#{node}", &hex[..8])
        }
        other => format!("{other:?}"),
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // --- The unscoped set read: strict mode never consulted ----------
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-bulk-read-demo/0.1")
            .await
            .expect("handshake");

        let matches = client.resolve_all(role(None)).await.expect("the set read");
        println!(
            "resolve-all role Button — {} matches, document order:",
            matches.len()
        );
        for (index, m) in matches.iter().enumerate() {
            println!(
                "  [{index}] {} generation {} rect {:?}",
                tree_of(m),
                m.generation,
                m.rect.map(|[x0, y0, x1, y1]| (x0, y0, x1, y1))
            );
        }
        // An empty set is a valid answer, never an error (the count()
        // contract over the wire).
        let empty = client
            .resolve_all(role(Some("Ghost")))
            .await
            .expect("the empty set read");
        println!(
            "resolve-all role Button \"Ghost\" — {} matches (total, no error)",
            empty.len()
        );

        // The dump-row rule: an enumerated row is directly usable as an
        // Action handle — no second resolve.
        let save = &matches[0];
        let done = client
            .request(RequestV1::Action {
                node: save.node.clone(),
                generation: save.generation,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 2000,
            })
            .await
            .expect("action on the enumerated row");
        if let ResponseV1::ActionDone { frame_seq, detail } = done {
            println!("action on row [0]: {detail} (frame {frame_seq})");
        }
        handle.shutdown();

        // --- The window-scoped set read (§16 v0.4) -----------------------
        let fx = MultiWindowFixture::default();
        let w2 = fx.w2.0.to_string();
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_multi_window(&fx),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-bulk-read-demo/0.1")
            .await
            .expect("handshake");

        client.window_use(&w2).await.expect("Preview aimed");
        let scoped = client
            .resolve_all(role(None))
            .await
            .expect("scoped set read");
        println!(
            "resolve-all inside Preview — {} match(es) (the connection's scope rides the walk)",
            scoped.len()
        );
        handle.shutdown();
    });
}
