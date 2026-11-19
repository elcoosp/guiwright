//! The screenshot seam rows (m50): [`ClientConn::screenshot_window`] and
//! [`ClientConn::screenshot_element`] carry the §7 render domain as typed
//! sugar — one call answering the [`Shot`] carrier (the frame the pixels
//! came from, the crop's pixel dimensions, the PNG payload), the same
//! shape the TypeScript client has spoken since M9 (`screenshotWindow()` /
//! `screenshotElement()`) and the REPL since M10 (`screenshot window` /
//! `screenshot element`). No wire change: the shots ride the `Screenshot`
//! domain byte-for-byte as M10 blessed it; every [`ClientConn`] impl
//! inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_screenshot -- /tmp/shots
//! # writes /tmp/shots/window.png and /tmp/shots/save-button.png, then
//! # prints the typed rejection for the missing locator
//! ```

use std::path::PathBuf;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{ErrorCode, SelectorV1, TextMatchV1};
use guiwright_proto::{DuplexClient, GwpError};

fn main() {
    let out_dir = std::env::args().nth(1).map_or_else(
        || std::env::temp_dir().join("guiwright-client-shots"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&out_dir).expect("output dir");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // App side: serve the buttons fixture with a scripted render
        // surface (§10 capture contract — a real adapter renders its own
        // frame here).
        let rgba: Vec<u8> = (0..240u32)
            .flat_map(|y| (0..480u32).map(move |x| (x, y)))
            .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
            .collect();
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || {
                MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(480, 240, rgba)
            },
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-client-screenshot-demo/0.1")
            .await
            .expect("handshake");

        // Window shot: the full frame, rendered once (§10) — the carrier
        // names the frame the pixels came from.
        let shot = client.screenshot_window().await.expect("window shot");
        let window_path = out_dir.join("window.png");
        std::fs::write(&window_path, &shot.png).expect("write window.png");
        println!(
            "window  → {} ({}×{}, frame {})",
            window_path.display(),
            shot.width,
            shot.height,
            shot.frame_seq
        );

        // Element shot: resolve (auto-wait) + crop by the resolved bounds —
        // the fixture Save button is [16,16 → 136,48], so 120×32.
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
            .expect("element shot");
        let element_path = out_dir.join("save-button.png");
        std::fs::write(&element_path, &shot.png).expect("write save-button.png");
        println!(
            "element → {} ({}×{}, frame {})",
            element_path.display(),
            shot.width,
            shot.height,
            shot.frame_seq
        );

        // The resolve's honesty rides the sugar: a locator that never
        // matches inside the budget answers the typed timeout naming the
        // stuck check.
        let ghost = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Ghost".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        match client.screenshot_element(ghost, 300).await {
            Err(GwpError::Rejected { code, message }) => {
                println!("missing → {code:?}: {message}");
                assert_eq!(code, ErrorCode::Timeout);
            }
            other => panic!("expected the typed rejection, got {other:?}"),
        }

        handle.shutdown();
    });
}
