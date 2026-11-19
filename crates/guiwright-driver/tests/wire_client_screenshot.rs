//! The client seam's screenshot rows over the wire (m50): [`ClientConn::screenshot_window`]
//! and [`ClientConn::screenshot_element`] carry the §7 render domain as typed seam sugar —
//! one call answering the [`Shot`] carrier (the frame the pixels came from, the crop's pixel
//! dimensions, the PNG payload), the same shape the TypeScript client has spoken since M9
//! (`screenshotWindow()` / `screenshotElement()`) and the REPL since M10 (`screenshot
//! window` / `screenshot element`). **No wire change**: the sugar rides the `Screenshot`
//! domain byte-for-byte as M10 blessed it; every [`ClientConn`] impl inherits the row.
//! Driven here over the in-process door with the four-button fixture and a scripted
//! 480×240 frame — the same reality `wire_screenshot.rs` pins at the raw-request level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{ErrorCode, SelectorV1, TextMatchV1};
use guiwright_proto::{DuplexClient, GwpError};

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-screenshot/0.1")
        .await
        .expect("handshake");
    client
}

/// The scripted render surface: a deterministic 480×240 RGBA frame (the
/// same gradient the raw `wire_screenshot.rs` example paints).
fn scripted_frame() -> MockAdapter {
    let rgba: Vec<u8> = (0..240u32)
        .flat_map(|y| (0..480u32).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect();
    MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(480, 240, rgba)
}

const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];

/// The window shot on one seam call: the full frame rendered once (§10),
/// answered as the [`Shot`] carrier — the frame the pixels came from, the
/// pixel dimensions and a real PNG payload. No raw request anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_pulls_the_full_window_frame() {
    let (handle, connect) =
        GwpServerHandle::spawn_inprocess(scripted_frame, guiwright_driver::SeamRegistry::new())
            .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let shot = client.screenshot_window().await.expect("the window shot");
    assert_eq!(
        (shot.width, shot.height),
        (480, 240),
        "the full scripted frame's dimensions"
    );
    assert!(
        shot.frame_seq > 0,
        "the carrier reports the frame the pixels came from"
    );
    assert!(
        shot.png.starts_with(&PNG_MAGIC),
        "the payload is a real PNG encode"
    );

    handle.shutdown();
}

/// The element shot resolves (auto-wait) and crops the one frame by the
/// resolved bounds — the fixture's Save button is [16,16 → 136,48], so the
/// crop is 120×32, never a re-render (§10 element rule).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seam_crops_the_resolved_element() {
    let (handle, connect) =
        GwpServerHandle::spawn_inprocess(scripted_frame, guiwright_driver::SeamRegistry::new())
            .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let save = SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: "Save".into(),
            exact: true,
            ignore_case: false,
        }),
    };
    let shot = client
        .screenshot_element(save, 2000)
        .await
        .expect("the element shot");
    assert_eq!(
        (shot.width, shot.height),
        (120, 32),
        "the Save button's clip-aware crop"
    );
    assert!(
        shot.png.starts_with(&PNG_MAGIC),
        "the payload is a real PNG encode"
    );

    handle.shutdown();
}

/// The resolve's honesty rides the sugar: a locator that never matches
/// inside the budget answers the typed `Timeout` naming the stuck check —
/// the same auto-wait the raw `Screenshot::Element` arm runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_locator_is_the_typed_timeout() {
    let (handle, connect) =
        GwpServerHandle::spawn_inprocess(scripted_frame, guiwright_driver::SeamRegistry::new())
            .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let ghost = SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: "Ghost".into(),
            exact: true,
            ignore_case: false,
        }),
    };
    let err = client
        .screenshot_element(ghost, 300)
        .await
        .expect_err("the locator never matches");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::Timeout, "{message}");
            assert!(
                message.contains("attached"),
                "the timeout names the stuck check: {message}"
            );
        }
        other => panic!("expected the typed timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// Strict mode is the wire's own policy (§8.2): an ambiguous locator
/// answers the typed `StrictModeViolation` — the same resolve discipline
/// the m49 `tree_dump_under` sugar pinned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ambiguous_locator_is_the_typed_rejection() {
    let (handle, connect) =
        GwpServerHandle::spawn_inprocess(scripted_frame, guiwright_driver::SeamRegistry::new())
            .expect("in-process server starts");

    let mut client = connected(&connect).await;
    // No name → the fixture's four buttons: strict mode rejects the set.
    let any_button = SelectorV1::Role {
        role: "Button".into(),
        name: None,
    };
    let err = client
        .screenshot_element(any_button, 2000)
        .await
        .expect_err("the set is ambiguous");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::StrictModeViolation, "{message}");
        }
        other => panic!("expected the typed rejection, got {other:?}"),
    }

    handle.shutdown();
}
