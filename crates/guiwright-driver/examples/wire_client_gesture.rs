//! The gesture seam row (m48): [`ClientConn::gesture`] carries the §9
//! input vocabulary as typed sugar — one call answering the
//! [`GestureReceipt`] (the frame the synthesized batch applied at plus
//! the synthesis summary), the same shape the TypeScript client has
//! spoken since M7 (`gesture()`) and the REPL since M7 (`mouse` / `key` /
//! `text`). No wire change: the synthesis rides the `Gesture` domain
//! byte-for-byte as M7 blessed it; every [`ClientConn`] impl inherits
//! the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_gesture
//! # prints the click's receipt (frame + detail), the wheel's receipt,
//! # and the typed rejection for the zero-count client bug
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{ErrorCode, GestureV1, PointerButtonV1};
use guiwright_proto::{DuplexClient, GwpError};

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
            .handshake("wire-client-gesture-demo/0.1")
            .await
            .expect("handshake");

        // A click is synthesized at the Save button's center — the driver
        // builds the move→down→up batch from its pacing table and applies
        // it inside the next frame; the receipt names that frame.
        let receipt = client
            .gesture(GestureV1::Click {
                x: 76.0,
                y: 32.0,
                button: PointerButtonV1::Primary,
                count: 1,
            })
            .await
            .expect("click applied");
        println!(
            "click (76, 32)   → {detail} (frame {frame})",
            detail = receipt.detail,
            frame = receipt.frame_seq
        );

        // Wheel: logical-px deltas, the same one-call shape.
        let receipt = client
            .gesture(GestureV1::Wheel {
                dx: 0.0,
                dy: -120.0,
            })
            .await
            .expect("wheel applied");
        println!(
            "wheel (0, -120)  → {detail} (frame {frame})",
            detail = receipt.detail,
            frame = receipt.frame_seq
        );

        // The boundary's honesty: a zero-count click synthesizes nothing
        // — the session rejects the empty batch as a client bug, and the
        // seam answers the wire's typed error naming the cause.
        match client
            .gesture(GestureV1::Click {
                x: 76.0,
                y: 32.0,
                button: PointerButtonV1::Primary,
                count: 0,
            })
            .await
        {
            Err(GwpError::Rejected { code, message }) => {
                println!("count 0 → {code:?}: {message}");
                assert_eq!(code, ErrorCode::Internal);
            }
            other => panic!("expected the typed rejection, got {other:?}"),
        }

        handle.shutdown();
    });
}
