//! The gesture wire domain (M7): the §9 input vocabulary —
//! `Gesture::Click | Drag | Wheel | Key` — over GWP, the commands the
//! REPL's `mouse`/`key` family and the TypeScript client's `mouse` /
//! `keyboard` helpers speak. The driver synthesizes the pointer path
//! under the adapter's pacing table (the §9 ratchet: interpolated moves
//! between the endpoints, one rendezvous pump applying the batch), so
//! hover/drag state machines see a real pointer. The reply carries the
//! frame the gesture applied at plus a human-readable synthesis summary.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_gesture
//! # prints the synthesis detail of a click, a key, a wheel and a drag
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::schema::{GestureV1, PointerButtonV1, RequestV1, ResponseV1};

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
            .handshake("wire-gesture-demo/0.1")
            .await
            .expect("handshake");

        // A click is synthesized at the Save button's center — the same
        // coordinates `get`'s rect and the hit-test read report.
        let done = client
            .request(RequestV1::Gesture(GestureV1::Click {
                x: 76.0,
                y: 32.0,
                button: PointerButtonV1::Primary,
                count: 1,
            }))
            .await
            .expect("click applied");
        if let ResponseV1::GestureDone { frame_seq, detail } = done {
            println!("click (76, 32)   → {detail} (frame {frame_seq})");
        }

        // `count` follows Playwright's clickCount: 2 = double-click
        // (down/up repeated at the same point).
        let done = client
            .request(RequestV1::Gesture(GestureV1::Click {
                x: 76.0,
                y: 32.0,
                button: PointerButtonV1::Primary,
                count: 2,
            }))
            .await
            .expect("double click applied");
        if let ResponseV1::GestureDone { frame_seq, detail } = done {
            println!("dblclick (76, 32) → {detail} (frame {frame_seq})");
        }

        // Keyboard: key names are the §8.4 `Press` vocabulary.
        let done = client
            .request(RequestV1::Gesture(GestureV1::Key {
                key: "Enter".into(),
                modifiers: vec![],
            }))
            .await
            .expect("key applied");
        if let ResponseV1::GestureDone { frame_seq, detail } = done {
            println!("key Enter        → {detail} (frame {frame_seq})");
        }

        // Wheel: logical-px deltas.
        let done = client
            .request(RequestV1::Gesture(GestureV1::Wheel {
                dx: 0.0,
                dy: -120.0,
            }))
            .await
            .expect("wheel applied");
        if let ResponseV1::GestureDone { frame_seq, detail } = done {
            println!("wheel (0, -120)  → {detail} (frame {frame_seq})");
        }

        // The drag: the driver interpolates the moves (§9 ratchet pacing)
        // between the two endpoints — the locator `drag_to` shape's raw
        // coordinate form (M33 rides the same synthesis).
        let done = client
            .request(RequestV1::Gesture(GestureV1::Drag {
                from_x: 76.0,
                from_y: 32.0,
                to_x: 76.0,
                to_y: 72.0,
                button: PointerButtonV1::Primary,
            }))
            .await
            .expect("drag applied");
        if let ResponseV1::GestureDone { frame_seq, detail } = done {
            println!("drag → (76, 72)  → {detail} (frame {frame_seq})");
        }
        handle.shutdown();
    });
}
