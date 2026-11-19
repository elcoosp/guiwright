//! The client seam's bye row over the wire (m58): [`ClientConn::bye`]
//! carries the §7 lifecycle close as typed seam sugar — one call answering
//! `Ok(())` on the wire's `Bye` arm, the same round trip the TypeScript
//! client's `close()` has made since its M3 landing before its own
//! transport teardown. **No wire change**: the sugar rides the `Bye`
//! domain byte-for-byte as M0 blessed it; every [`ClientConn`] impl
//! inherits the row. Driven here over the in-process door — the door's
//! transport close stays the door's business (drop / your client's own
//! teardown), so the tests can pin the honest wire semantics: the farewell
//! ends the *conversation* (`run_connection` returns right after answering
//! `Bye`), it does not tear the *session* down.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-bye/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts")
}

/// The farewell lands as `Ok(())`: one round trip, the wire's own answer —
/// no carrier, no sentinel, the `Bye` arm verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_farewell_answers_ok() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    client.bye().await.expect("the farewell answers");

    handle.shutdown();
}

/// The farewell ends the conversation, not the session (the wire's own
/// contract): a read on the farewelled connection no longer lands — the
/// door's transport closed with the bye, exactly why the TS `close()`
/// awaits its bye before its own `transport.close()` — while a fresh
/// connection to the same server still reads the same tree. The arm is
/// also no one-shot latch on the server: it simply answered and ended
/// *this* conversation, so the session under it never moved.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_farewell_ends_the_conversation_not_the_session() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    client.bye().await.expect("the farewell answers");

    let spent = client.tree_snapshot().await;
    assert!(
        spent.is_err(),
        "the conversation ended with the farewell, the read cannot land: {spent:?}"
    );

    let mut successor = connected(&connect).await;
    let summary = successor
        .tree_snapshot()
        .await
        .expect("the successor connection still lands");
    assert_eq!(
        summary.node_count, 8,
        "the same session kept serving, untouched by the farewell"
    );

    handle.shutdown();
}
