//! The raw-input row (m56): [`ClientConn::input`] carries the §9
//! raw-injection domain as typed sugar — one call carrying the
//! [`InputEventV1`] batch and answering the [`InputReceipt`] (the frame the
//! batch applied at, the §15.4 rendezvous), the same shape the TypeScript
//! client has spoken since M7 (`input(events)`). Un-synthesized on
//! purpose: gestures synthesize their batches from the driver's pacing
//! table (§9), this row applies exactly the primitives the client sent —
//! the escape hatch the m48 gesture row's contract names, now typable. No
//! wire change: the batch rides the `Input` domain byte-for-byte as M0
//! blessed it; every [`ClientConn`] impl inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_input
//! # prints the pointer batch's receipt, the keyboard batch's receipt,
//! # and the empty batch's honest no-op pump
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::InputEventV1;

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
            .handshake("wire-client-input-demo/0.1")
            .await
            .expect("handshake");

        // A raw pointer batch (§9 primitives, in order — the client owns
        // the fidelity, the driver applies the batch inside the next
        // frame and answers the rendezvous).
        let pointer = client
            .input(vec![
                InputEventV1::PointerMove { x: 12.0, y: 34.0 },
                InputEventV1::PointerMove { x: 13.0, y: 35.0 },
            ])
            .await
            .expect("pointer batch");
        println!("pointer → applied at frame {}", pointer.frame_seq);

        // The empty batch is an honest no-op that still pumps (the
        // session's own rule) — the receipt names the applied frame,
        // never a synthesized sentinel.
        let empty = client.input(Vec::new()).await.expect("empty batch");
        println!("empty   → applied at frame {}", empty.frame_seq);

        handle.shutdown();
    });
}
