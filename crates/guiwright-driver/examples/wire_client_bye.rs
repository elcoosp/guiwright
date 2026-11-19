//! The bye row (m58): [`ClientConn::bye`] carries the §7 lifecycle close as
//! typed sugar — one call answering `Ok(())` on the wire's `Bye` arm, the
//! same round trip the TypeScript client's `close()` has made since its M3
//! landing before its own transport teardown. The wire's own contract: the
//! driver answers the farewell and the conversation ends — a further
//! request on this connection cannot land — while the session under it
//! keeps serving fresh connections untouched. No wire change: the farewell
//! rides the `Bye` domain byte-for-byte as M0 blessed it; every
//! [`ClientConn`] impl inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_bye
//! # prints the farewell's answer, the spent connection's honest error,
//! # and the successor connection's summary read (the session kept serving)
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;

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
            .handshake("wire-client-bye-demo/0.1")
            .await
            .expect("handshake");

        // The farewell: the driver answers before the door closes, so
        // `Ok(())` is the honest last-word — the driver received the
        // goodbye.
        client.bye().await.expect("the farewell answers");
        println!("bye     → answered, the conversation is over");

        // A further request on the farewelled connection cannot land: the
        // transport closed with the bye (the door's own business).
        let spent = client.tree_snapshot().await;
        assert!(spent.is_err(), "the conversation ended with the farewell");
        println!("post-bye read → the door closed with the farewell");

        // The session under the connection kept serving: a fresh
        // connection reads the same tree.
        let mut successor = DuplexClient::connect(&connect)
            .await
            .expect("the successor client connects");
        successor
            .handshake("wire-client-bye-demo/0.1")
            .await
            .expect("handshake");
        let summary = successor
            .tree_snapshot()
            .await
            .expect("the successor's summary read");
        assert_eq!(summary.node_count, 8, "the same session, untouched");
        println!(
            "successor → tree_seq {} at {} nodes (the session kept serving)",
            summary.tree_seq, summary.node_count
        );

        handle.shutdown();
    });
}
