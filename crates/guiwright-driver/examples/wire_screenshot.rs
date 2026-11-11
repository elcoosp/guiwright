//! The §7 render domain consumer (M10): a GWP server with a scripted
//! render surface + a Rust client that pulls window and element PNG
//! shots off the wire — the "CI container fetches visual evidence from a
//! remote app" flow.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_screenshot -- /tmp/shots
//! # writes /tmp/shots/window.png and /tmp/shots/save-button.png
//! ```
//!
//! The same conversation works over TCP (`GwpServerHandle::spawn_tcp`)
//! and from the TypeScript client (`screenshotWindow` /
//! `screenshotElement`).

use std::path::PathBuf;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{RequestV1, ResponseV1, ScreenshotV1, SelectorV1, TextMatchV1};
use guiwright_proto::uds::Client;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("guiwright-wire-shots"));
    std::fs::create_dir_all(&out_dir).expect("output dir");

    // App side: serve the buttons fixture with a scripted render surface
    // (§10 capture contract — a real adapter renders its own frame here).
    let socket =
        std::env::temp_dir().join(format!("guiwright-shot-demo-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let rgba: Vec<u8> = (0..240u32)
        .flat_map(|y| (0..480u32).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect();
    // Client side: handshake, then the two render-domain requests. The
    // server stack runs on tokio (§7 transports), so the demo hosts the
    // spawn + the client conversation in one runtime it keeps alive.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    let handle = runtime.block_on(async {
        let handle = spawn_session_server(socket.clone(), move || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(480, 240, rgba)
        })
        .expect("server binds");
        let mut client = Client::connect(&socket).await.expect("client connects");
        let adapter = client.handshake("wire-screenshot-demo/0.1").await.unwrap();
        println!("attached to {} ({})", adapter.name, adapter.framework);

        // Window shot: the full frame, rendered once (§10).
        let ResponseV1::Shot {
            frame_seq,
            width,
            height,
            png,
        } = client
            .request(RequestV1::Screenshot(ScreenshotV1::Window))
            .await
            .expect("window shot")
        else {
            panic!("expected Shot");
        };
        let window_path = out_dir.join("window.png");
        std::fs::write(&window_path, &png).expect("write window.png");
        println!(
            "window: {} ({}×{}, frame {frame_seq})",
            window_path.display(),
            width,
            height
        );

        // Element shot: resolve (auto-wait) + crop by the resolved bounds
        // — the fixture Save button is [16,16 → 136,48], so 120×32.
        let ResponseV1::Shot {
            frame_seq,
            width,
            height,
            png,
        } = client
            .request(RequestV1::Screenshot(ScreenshotV1::Element {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                timeout_ms: 2000,
            }))
            .await
            .expect("element shot")
        else {
            panic!("expected Shot");
        };
        let element_path = out_dir.join("save-button.png");
        std::fs::write(&element_path, &png).expect("write save-button.png");
        println!(
            "element: {} ({}×{}, frame {frame_seq})",
            element_path.display(),
            width,
            height
        );

        let _ = client.request(RequestV1::Bye).await;
        handle
    });

    handle.shutdown();
}
