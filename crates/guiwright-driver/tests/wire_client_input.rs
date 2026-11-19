//! The client seam's raw-input row over the wire (m56): [`ClientConn::input`]
//! carries the §9 raw-injection domain as typed seam sugar — one call
//! carrying the [`InputEventV1`] batch and answering the [`InputReceipt`]
//! carrier (the frame the batch queued against — the §15.4 rendezvous), the
//! same shape the TypeScript client has spoken since M7 (`input(events)`)
//! and the un-synthesized escape hatch the m48 gesture row's contract
//! names ("the raw `RequestV1::Input` batch stays exactly where M7 left
//! it" — now typable, still un-synthesized). **No wire change**: the sugar
//! rides the `Input` domain byte-for-byte as M0/M7 blessed it; every
//! [`ClientConn`] impl inherits the row. Driven here over the in-process
//! door — the same reality `input_gestures.rs` pins at the raw-request
//! level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::InputEventV1;

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-input/0.1")
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

/// A pointer batch queues against a frame rendezvous: the receipt names the
/// frame the batch applied at (§15.4 — clients can await exactly this
/// frame), and the next batch queues against a later frame (the session
/// pumps per batch, so the seq moves forward).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pointer_batch_queues_against_a_frame() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let first = client
        .input(vec![InputEventV1::PointerMove { x: 12.0, y: 34.0 }])
        .await
        .expect("the first batch");
    assert!(first.frame_seq >= 1, "the receipt names the applied frame");

    let second = client
        .input(vec![
            InputEventV1::PointerMove { x: 13.0, y: 35.0 },
            InputEventV1::PointerMove { x: 14.0, y: 36.0 },
        ])
        .await
        .expect("the second batch");
    assert!(
        second.frame_seq > first.frame_seq,
        "each batch applies at its own rendezvous: {} then {}",
        first.frame_seq,
        second.frame_seq
    );

    handle.shutdown();
}

/// The empty batch is an honest no-op that still pumps (the session's own
/// rule): the receipt still names the frame the pump applied at — the
/// sugar surfaces the wire's answer verbatim, never synthesizing a
/// sentinel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_batch_is_an_honest_no_op_that_still_pumps() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let receipt = client.input(Vec::new()).await.expect("the empty batch");
    assert!(
        receipt.frame_seq >= 1,
        "the pump still applied: {receipt:?}"
    );

    handle.shutdown();
}
