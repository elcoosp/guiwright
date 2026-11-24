//! The §15.6 shared-frame ring consumer (m88): a GWP server that
//! publishes every presented frame into a file-backed ring, plus the
//! same-host reader half that pulls the newest RGBA8 pixels straight out
//! of the store — the "same-host client reads pixels outside the door"
//! flow beside the blob side-channel (no wire change).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_frame_ring
//! # frames land in /tmp/guiwright-frame-ring-demo-<pid>.gwring;
//! # the reader prints the newest committed frame per poll round
//! ```
//!
//! The driver-side enablement is the `ServerOptions::frame_ring` knob
//! (remote-mode default: the `GUIWRIGHT_FRAME_RING` env var, the
//! `GUIWRIGHT_VIDEO_DIR` pattern); the reader half is
//! `guiwright_proto::shmring::FrameRingReader`, usable from any process
//! that can open the path. The `FramePresented` events (§7, M12) announce
//! publication — the event-fed consumer route; the example's bounded
//! `wait_latest` poll is the tool-convenience shape.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use guiwright_core::test_util::Fixture;
use guiwright_driver::ServerOptions;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server_with;
use guiwright_proto::ClientConn;
use guiwright_proto::schema::{RequestV1, SelectorV1, SemActionV1};
use guiwright_proto::shmring::FrameRingReader;
use guiwright_proto::uds::Client;

/// The scripted 240×120 RGBA8 gradient the mock adapter presents.
fn scripted_pixels() -> Vec<u8> {
    const W: u32 = 240;
    const H: u32 = 120;
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect()
}

fn main() {
    let tag = std::process::id();
    let socket = std::env::temp_dir().join(format!("guiwright-ring-demo-{tag}.sock"));
    let ring = std::env::args().nth(1).map_or_else(
        || std::env::temp_dir().join(format!("guiwright-frame-ring-demo-{tag}.gwring")),
        PathBuf::from,
    );
    let _ = std::fs::remove_file(&socket);

    // App side: the buttons fixture with a scripted render surface —
    // every pump presents one frame, every presented frame lands in the
    // ring (the §15.6 publisher rides the same last_frame tap as the §10
    // video recorder). The server stack runs on tokio (§7 transports), so
    // the demo hosts spawn + conversation in one runtime it keeps alive.
    let rgba = scripted_pixels();
    let ring_for_server = ring.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    let handle = runtime.block_on(async {
        let handle = spawn_session_server_with(
            socket.clone(),
            move || {
                MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(240, 120, rgba)
            },
            ServerOptions {
                frame_ring: Some(ring_for_server),
                ..ServerOptions::default()
            },
        )
        .expect("server binds");

        let mut client = Client::connect(&socket).await.expect("client connects");
        client.handshake("guiwright-ring-demo/0.1").await.unwrap();
        println!("attached; driving two clicks (each command pumps a frame)");

        let click = SelectorV1::Role {
            role: "Button".into(),
            name: Some(guiwright_proto::schema::TextMatchV1 {
                text: "Save".into(),
                exact: false,
                ignore_case: true,
            }),
        };
        for _ in 0..2 {
            // Resolve → act: each command batch pumps a frame, each
            // presented frame publishes into the ring.
            let row = client
                .resolve(click.clone(), 2000)
                .await
                .expect("Save resolves");
            client
                .action(
                    row.node.clone(),
                    row.generation,
                    SemActionV1::Click,
                    false,
                    5000,
                )
                .await
                .expect("the click lands");
        }
        let _ = client.request(RequestV1::Bye).await;
        handle
    });

    // The same-host reader half opens the store (created during session
    // boot, before the first publish) and serves the newest committed
    // frame.
    let reader = FrameRingReader::open(&ring).expect("reader opens the ring");
    println!(
        "ring open: {} slots × {} bytes, dims {:?}",
        reader.config().slots,
        reader.config().slot_bytes,
        reader.dims(),
    );
    let frame = reader
        .wait_latest(1, Instant::now() + Duration::from_secs(2))
        .expect("ring read")
        .expect("frames committed during the session");
    println!(
        "newest frame: seq {} ({}×{}, {} bytes), digest-validated ✓",
        frame.seq,
        frame.width,
        frame.height,
        frame.pixels.len()
    );
    assert_eq!(
        frame.pixels,
        scripted_pixels(),
        "the ring carries the pixels"
    );
    let stats = reader.writer_stats().expect("header stats");
    println!(
        "publisher stats: {} published, {} wrapped, {} skipped",
        stats.published, stats.wrapped, stats.skipped
    );
    handle.shutdown();
}
