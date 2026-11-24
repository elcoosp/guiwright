//! The §15.6 shared-frame ring, end to end: a session's presented frames
//! are visible to a same-host reader (embedded and over the UDS door), the
//! pixel bytes ride outside the GWP conversation, and the publisher's
//! declines stay counted.
//!
//! The plan's acceptance fact (architecture §15.6, m88): *"a shared-memory
//! frame ring for same-host clients (zero-copy pixels), mirroring CDP's
//! binary socket philosophy"* — the reader serves the newest committed
//! frame's RGBA8 bytes with the session's frame seq, no wire change.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::ServerOptions;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::session::Session;
use guiwright_driver::spawn_session_server_with;
use guiwright_proto::schema::{RequestV1, ScreenshotV1};
use guiwright_proto::shmring::{FrameRingReader, RingConfig};
use guiwright_proto::uds::Client;

fn ring_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(format!("{tag}.gwring"))
}

/// The scripted 8×8 RGBA8 buffer the mock adapter presents on every frame.
fn scripted_pixels(seed: u8) -> Vec<u8> {
    vec![seed; 8 * 8 * 4]
}

/// Embedded: frames offered to the ring land in the store; the same-host
/// reader serves the newest committed seq with the exact pixels.
#[test]
fn embedded_session_publishes_presented_frames_to_the_ring() {
    let path = ring_path("embedded");
    let mut session = Session::new(
        MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(8, 8, scripted_pixels(3)),
    );
    session.frame_ring_start(&path).expect("ring starts");
    for _ in 0..3 {
        session.pump();
    }
    let stats = session.frame_ring_stats().expect("publishing");
    assert_eq!(stats.published, 3, "one commit per presented frame");
    assert_eq!(stats.skipped, 0);
    let reader = FrameRingReader::open(&path).expect("same-host reader opens");
    let frame = reader.latest().expect("read").expect("newest frame");
    assert_eq!(frame.seq, session.frame_seq(), "the session's newest frame");
    assert_eq!((frame.width, frame.height), (8, 8));
    assert_eq!(frame.pixels, scripted_pixels(3), "the exact pixels");
    session.frame_ring_stop().expect("stop");
    assert!(
        session.frame_ring_stats().is_none(),
        "not publishing after the stop"
    );
}

/// Pixel-less adapters skip honestly: no ring file materializes, the skip
/// counter carries the presented frames.
#[test]
fn a_pixel_less_session_publishes_nothing_and_names_the_skips() {
    let path = ring_path("pixel-less");
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    session.frame_ring_start(&path).expect("ring starts");
    for _ in 0..4 {
        session.pump();
    }
    let stats = session.frame_ring_stats().expect("publishing");
    assert_eq!(stats.published, 0);
    assert_eq!(stats.skipped, 4, "every presented frame declined, counted");
    assert!(!path.exists(), "no frames, no store");
}

/// The single-writer contract: a second `frame_ring_start` names the
/// rejection, and stopping an un-started ring does too.
#[test]
fn the_single_writer_contract_names_its_rejections() {
    let path = ring_path("single-writer");
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    session.frame_ring_start(&path).expect("first start");
    let err = session
        .frame_ring_start(&ring_path("other"))
        .expect_err("already publishing");
    assert!(err.to_string().contains("already publishing"), "{err}");
    session.frame_ring_stop().expect("stop");
    let err = session.frame_ring_stop().expect_err("stopped twice");
    assert!(err.to_string().contains("no frame ring"), "{err}");
}

/// A custom slot budget rides `frame_ring_start_with`: the ring wraps at
/// the configured width and the reader still sees the newest seq.
#[test]
fn a_custom_slot_budget_wraps_and_keeps_the_newest() {
    let path = ring_path("budget");
    let config = RingConfig {
        slots: 2,
        slot_bytes: 8 * 8 * 4,
    };
    let mut session = Session::new(
        MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(8, 8, scripted_pixels(1)),
    );
    session
        .frame_ring_start_with(&path, config)
        .expect("ring starts");
    for _ in 0..5 {
        session.pump();
    }
    let stats = session.frame_ring_stats().expect("publishing");
    assert_eq!(stats.published, 5);
    assert_eq!(
        stats.wrapped, 3,
        "slots 2, seqs 3..5 recycled committed slots"
    );
    let reader = FrameRingReader::open(&path).expect("reader opens");
    let frame = reader.latest().expect("read").expect("newest frame");
    assert_eq!(frame.seq, session.frame_seq());
    let writer_stats = reader.writer_stats().expect("header stats");
    assert_eq!(writer_stats, stats, "the header mirrors the session");
}

/// Over the UDS door: a server spawned with `ServerOptions::frame_ring`
/// publishes every presented frame; a same-host client (here: the test
/// process, the reader half) reads the newest frame after driving the
/// session — pixels outside the GWP conversation, no wire change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_uds_server_publishes_presented_frames_to_the_ring() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let socket = dir.path().join("ring-demo.sock");
    let ring = dir.path().join("ring-demo.gwring");
    let handle = spawn_session_server_with(
        socket.clone(),
        || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
                8,
                8,
                scripted_pixels(5),
            )
        },
        ServerOptions {
            frame_ring: Some(ring.clone()),
            ..ServerOptions::default()
        },
    )
    .expect("server binds");
    let mut client = Client::connect(&socket).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();
    // One pump per command batch (§15.4): the handshake settle + this
    // request present frames into the ring.
    let resp = client
        .request(RequestV1::Screenshot(ScreenshotV1::Window))
        .await
        .expect("window shot");
    let _ = resp;
    // The same-host reader half: newest committed frame, exact pixels.
    let reader = FrameRingReader::open(&ring).expect("reader opens the ring");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let frame = reader
        .wait_latest(1, deadline)
        .expect("read")
        .expect("frames published during attach + shot");
    assert_eq!((frame.width, frame.height), (8, 8));
    assert_eq!(frame.pixels, scripted_pixels(5));
    handle.shutdown();
}
