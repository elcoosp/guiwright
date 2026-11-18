//! The hit-test wire domain (M32): the §8.3 "receives events" mechanism —
//! `TreeService::hit_test` — as `HitTest` → `HitTest` over GWP, with the
//! typed [`ClientConn::hit_test`] sugar. "What lives at this point" is
//! the debugging read the REPL's `hit` command and viewer-style clients
//! speak: the point is physical px in the tree's bounds space (the same
//! space the resolve rects and dump rows report, so a client can hit
//! inside a rect it just resolved), a hit answers one row directly
//! usable as an `Action` handle, a miss answers `None` — the read is
//! total, never an error. The two-window fixture shows the connection's
//! window scope narrowing the answer.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_hit_test
//! # prints the hit under the Save button's center, the empty-space
//! # miss, an action on the hit row, and the scoped reads
//! ```

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{NodeKeyV1, RequestV1, ResponseV1, SemActionV1};
use guiwright_proto::{ClientConn, DuplexClient};

fn describe(node: &NodeKeyV1) -> String {
    match node {
        NodeKeyV1::AccessKit { node, .. } => format!("accesskit#{node}"),
        other => format!("{other:?}"),
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // --- The unscoped hit-test read: the click-what-you-see loop -----
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-hit-test-demo/0.1")
            .await
            .expect("handshake");

        let hit = client.hit_test(76.0, 32.0).await.expect("the read");
        match &hit {
            Some(row) => println!(
                "hit (76, 32) → {} (generation {}) rect {:?}",
                describe(&row.node),
                row.generation,
                row.rect
            ),
            None => println!("hit (76, 32) → nothing"),
        }
        // A miss is a valid answer, never an error (the read is total).
        let miss = client.hit_test(400.0, 220.0).await.expect("the read");
        println!(
            "hit (400, 220) → {} (empty space answers None)",
            match miss {
                Some(_) => "a node",
                None => "nothing",
            }
        );
        // The dump-row rule: a hit is directly usable as an Action handle
        // — hit, then act, no second resolve.
        let row = hit.expect("the save button sits under the point");
        let done = client
            .request(RequestV1::Action {
                node: row.node.clone(),
                generation: row.generation,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 2000,
            })
            .await
            .expect("action on the hit row");
        if let ResponseV1::ActionDone { frame_seq, detail } = done {
            println!("action on the hit: {detail} (frame {frame_seq})");
        }
        handle.shutdown();

        // --- The window-scoped hit-test read (§16 v0.4) ------------------
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
            .handshake("wire-hit-test-demo/0.1")
            .await
            .expect("handshake");

        client.window_use(&w2).await.expect("Preview aimed");
        // Editor's Save sits at (76, 32) — outside the aimed window, so
        // the scoped read answers None instead of a cross-window hit.
        let outside = client.hit_test(76.0, 32.0).await.expect("the read");
        println!(
            "hit (76, 32) inside Preview → {} (the connection's scope rides the read)",
            match outside {
                Some(_) => "a node",
                None => "nothing",
            }
        );
        let inside = client
            .hit_test(72.0, 432.0)
            .await
            .expect("the read")
            .expect("Preview's own Save sits under the point");
        println!(
            "hit (72, 432) inside Preview → {} (generation {})",
            describe(&inside.node),
            inside.generation
        );
        handle.shutdown();
    });
}
