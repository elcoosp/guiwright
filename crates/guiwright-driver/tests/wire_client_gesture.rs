//! The client seam's gesture rows over the wire (m48): [`ClientConn::gesture`]
//! carries the §9 input vocabulary as typed seam sugar — one call answering
//! the [`GestureReceipt`] (the frame the batch applied at plus the synthesis
//! summary), the same shape the TypeScript client has spoken since M3
//! (`gesture()`) and the REPL since M7 (`mouse` / `key` / `text`). **No wire
//! change**: the sugar rides the `Gesture` domain byte-for-byte as M7
//! blessed it; every [`ClientConn`] impl inherits the row. Driven here over
//! the in-process door with the four-button fixture — the same reality
//! `input_gestures.rs` pins at the raw-request level.

use std::sync::{Arc, Mutex};

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::InputEvent;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{ErrorCode, GestureV1, PointerButtonV1};
use guiwright_proto::{DuplexClient, GwpError};

type InputLog = Arc<Mutex<Vec<(String, InputEvent)>>>;

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-gesture/0.1")
        .await
        .expect("handshake");
    client
}

/// The happy synthesis path on one seam call: the click rides the §9
/// recipes (move → down → up), answers the frame it applied at and the
/// synthesis summary — no raw request anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_clicks_through_the_synthesis_path() {
    let log: InputLog = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&log);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_input_log(shared),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let receipt = client
        .gesture(GestureV1::Click {
            x: 76.0,
            y: 32.0,
            button: PointerButtonV1::Primary,
            count: 1,
        })
        .await
        .expect("the click lands");
    assert_eq!(
        receipt.detail, "clicked at (76, 32) — 3 events",
        "the synthesis summary the REPL row renders"
    );
    assert!(
        receipt.frame_seq > 0,
        "the receipt reports the frame the batch applied at"
    );

    // The batch reached the adapter through the per-window surface,
    // tagged with the connection's default target (single-window → `main`).
    let logged = log.lock().expect("input log").clone();
    assert_eq!(logged.len(), 3, "move + down + up");
    assert_eq!(logged[0].0, "main");
    assert!(matches!(logged[0].1, InputEvent::PointerMove { .. }));
    assert!(matches!(logged[1].1, InputEvent::PointerDown { .. }));
    assert!(matches!(logged[2].1, InputEvent::PointerUp { .. }));

    handle.shutdown();
}

/// The §9 vocabulary answers one receipt per row: a double-click, a drag
/// (the §9 ratchet synthesis), a wheel, a key and a text commit — the
/// same summaries the REPL's `mouse` / `key` / `text` rows render.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_carries_the_whole_gesture_vocabulary() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;

    let receipt = client
        .gesture(GestureV1::Click {
            x: 76.0,
            y: 32.0,
            button: PointerButtonV1::Primary,
            count: 2,
        })
        .await
        .expect("the double-click lands");
    assert_eq!(receipt.detail, "double-clicked at (76, 32) — 5 events");

    let receipt = client
        .gesture(GestureV1::Drag {
            from_x: 76.0,
            from_y: 32.0,
            to_x: 76.0,
            to_y: 72.0,
            button: PointerButtonV1::Primary,
        })
        .await
        .expect("the drag lands");
    assert!(
        receipt.detail.starts_with("dragged (76, 32) → (76, 72)"),
        "{}",
        receipt.detail
    );

    let receipt = client
        .gesture(GestureV1::Wheel {
            dx: 0.0,
            dy: -120.0,
        })
        .await
        .expect("the wheel lands");
    assert_eq!(receipt.detail, "wheeled (0, -120) — 1 event");

    let receipt = client
        .gesture(GestureV1::Key {
            key: "Enter".into(),
            modifiers: vec!["Control".into()],
        })
        .await
        .expect("the key lands");
    assert_eq!(receipt.detail, "pressed Control+Enter — 2 events");

    let receipt = client
        .gesture(GestureV1::Type { text: "ada".into() })
        .await
        .expect("the text commit lands");
    assert_eq!(receipt.detail, "typed 3 chars — 1 event");

    handle.shutdown();
}

/// The connection's window scope steers the synthesis (the M6 handoff
/// the §9 grammar rides): after `window_use`, the same coordinates land
/// in the aimed window's pipeline — the log tags say which.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_gesture_rides_the_window_scope() {
    let fx = MultiWindowFixture::new();
    let log: InputLog = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&log);
    let scripted = fx.clone();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_multi_window(&scripted).with_input_log(shared),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    client
        .window_use(fx.w2.0.to_string())
        .await
        .expect("the second window aims");

    let receipt = client
        .gesture(GestureV1::Click {
            x: 10.0,
            y: 20.0,
            button: PointerButtonV1::Primary,
            count: 1,
        })
        .await
        .expect("the click lands in the aimed window");
    assert!(
        receipt.detail.contains("clicked at (10, 20)"),
        "{}",
        receipt.detail
    );

    let logged = log.lock().expect("input log").clone();
    assert_eq!(logged.len(), 3, "move + down + up");
    assert_eq!(
        logged[0].0,
        fx.w2.0.to_string(),
        "the batch landed in the aimed window's pipeline"
    );

    handle.shutdown();
}

/// The wire's own honesty rides the sugar: a zero-count click
/// synthesizes nothing — the session rejects the empty batch as a client
/// bug, and the seam answers the wire's typed error naming the cause
/// (pinned as shipped, the m47 record's contract rule).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zero_count_click_is_the_typed_rejection() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let err = client
        .gesture(GestureV1::Click {
            x: 76.0,
            y: 32.0,
            button: PointerButtonV1::Primary,
            count: 0,
        })
        .await
        .expect_err("count 0 is a client bug");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::Internal, "{message}");
            assert!(
                message.contains("count 0"),
                "the rejection names the cause: {message}"
            );
        }
        other => panic!("expected the typed rejection, got {other:?}"),
    }

    handle.shutdown();
}
