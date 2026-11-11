//! The §7 render domain over the wire (M10 "wire screenshots", the
//! milestone docs/m9-locator-element-screenshots.md §4 recorded): window
//! and element PNG shots ride GWP so a CI container can pull visual
//! evidence out of a remote app — the same render-once → crop-by-bounds
//! path the embedded kit surface uses (§10 "never re-render").
//!
//! The mock adapter scripts RGBA pixels (`with_frame_pixels`, the capture
//! test contract since M1), so every byte of the returned PNG is checkable
//! against the known gradient.

use std::sync::{Arc, Mutex};

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{
    ErrorCode, RequestV1, ResponseV1, ScreenshotV1, SelectorV1, TextMatchV1,
};
use guiwright_proto::uds::Client;

/// A 480×240 deterministic gradient — `(x & 0xff, y & 0xff, 99, 255)` —
/// the same scripted pattern the bench sample uses.
fn gradient_rgba(width: u32, height: u32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect()
}

/// The fixture adapter with a scripted render surface (§6.2 caps row on).
fn scripted_adapter() -> MockAdapter {
    MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
        480,
        240,
        gradient_rgba(480, 240),
    )
}

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

/// Unwrap a `Shot` reply.
fn shot(resp: ResponseV1, what: &str) -> (u64, u32, u32, Vec<u8>) {
    match resp {
        ResponseV1::Shot {
            frame_seq,
            width,
            height,
            png,
        } => (frame_seq, width, height, png),
        other => panic!("expected Shot for {what}, got {other:?}"),
    }
}

/// Unwrap an `Error` reply — the UDS client maps `Error` frames to
/// [`GwpError::Rejected`], so both shapes land here.
fn wire_error(resp: Result<ResponseV1, guiwright_proto::GwpError>) -> (ErrorCode, String) {
    match resp {
        Err(guiwright_proto::GwpError::Rejected { code, message }) => (code, message),
        other => panic!("expected an Error reply, got {other:?}"),
    }
}

fn role_named(role: &str, name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_shot_over_the_wire_is_the_current_frame() {
    let path = socket_path("shot-window.sock");
    let handle = spawn_session_server(path.clone(), scripted_adapter).expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    let resp = client
        .request(RequestV1::Screenshot(ScreenshotV1::Window))
        .await
        .unwrap();
    let (_frame_seq, width, height, png) = shot(resp, "window");

    assert_eq!((width, height), (480, 240), "full window frame");
    let image = guiwright_shot::codec::decode_png(&png).expect("valid PNG");
    assert_eq!((image.width, image.height), (480, 240));
    // Byte-check the gradient: the PNG is the scripted frame, lossless.
    assert_eq!(image.pixel(0, 0), [0, 0, 99, 255]);
    assert_eq!(image.pixel(300, 7), [44, 7, 99, 255]);
    assert_eq!(image.pixel(479, 239), [223, 239, 99, 255]);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn element_shot_crops_the_window_frame_by_bounds() {
    let path = socket_path("shot-element.sock");
    let handle = spawn_session_server(path.clone(), scripted_adapter).expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // The fixture's Save button: bounds [16,16 → 136,48]. The element
    // resolve auto-waits attached within timeout_ms, exactly like `Resolve`
    // (§8.2 over the wire).
    let resp = client
        .request(RequestV1::Screenshot(ScreenshotV1::Element {
            selector: role_named("Button", "Save"),
            timeout_ms: 2000,
        }))
        .await
        .unwrap();
    let (_frame_seq, width, height, png) = shot(resp, "element");

    assert_eq!((width, height), (120, 32), "crop = the node's bounds");
    let image = guiwright_shot::codec::decode_png(&png).expect("valid PNG");
    assert_eq!((image.width, image.height), (120, 32));
    // The crop's (0,0) is the frame's (16,16) — the §10 "crop from the
    // window frame, never re-render" rule, byte-checked.
    assert_eq!(image.pixel(0, 0), [16, 16, 99, 255]);
    assert_eq!(image.pixel(119, 31), [135, 47, 99, 255]);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn element_shot_auto_waits_for_a_late_node() {
    // The node only exists after a scripted diff lands mid-session: the
    // element resolve must wait (Attached within timeout_ms), then crop.
    let path = socket_path("shot-wait.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        scripted_adapter().with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // Script the late "Export" button: an incremental update on the fixture
    // group (NodeId(1)), delivered on the session's next pump.
    let mut group = Node::new(Role::Group);
    let fixture = Fixture::buttons();
    group.set_children([
        fixture.save_button,
        fixture.cancel_button,
        fixture.welcome_label,
        fixture.email_input,
        fixture.hidden_button,
        fixture.disabled_button,
        NodeId(200),
    ]);
    let mut export = Node::new(Role::Button);
    export.set_label("Export");
    export.set_bounds(accesskit::Rect {
        x0: 200.0,
        y0: 16.0,
        x1: 280.0,
        y1: 48.0,
    });
    shared.lock().unwrap().push(TreeUpdate {
        nodes: vec![(NodeId(1), group), (NodeId(200), export)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    });

    let resp = client
        .request(RequestV1::Screenshot(ScreenshotV1::Element {
            selector: role_named("Button", "Export"),
            timeout_ms: 5000,
        }))
        .await
        .unwrap();
    let (_frame_seq, width, height, png) = shot(resp, "late element");
    assert_eq!((width, height), (80, 32));
    let image = guiwright_shot::codec::decode_png(&png).expect("valid PNG");
    assert_eq!(image.pixel(0, 0), [200, 16, 99, 255]);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shot_without_a_render_surface_is_unsupported() {
    // The §6.2 honest default: a fixture adapter without scripted pixels
    // has no render capability row, so the domain says Unsupported — the
    // same structure the M8 overlay errors keep end-to-end.
    let path = socket_path("shot-unsupported.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    let resp = client
        .request(RequestV1::Screenshot(ScreenshotV1::Window))
        .await;
    let (code, _message) = wire_error(resp);
    assert_eq!(code, ErrorCode::Unsupported);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn element_shot_without_bounds_is_an_honest_error() {
    let path = socket_path("shot-nobounds.sock");
    let handle = spawn_session_server(path.clone(), scripted_adapter).expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // The fixture window root has no bounds — nothing to crop (§10).
    let (_code, message) = wire_error(
        client
            .request(RequestV1::Screenshot(ScreenshotV1::Element {
                selector: SelectorV1::Role {
                    role: "Window".into(),
                    name: None,
                },
                timeout_ms: 1000,
            }))
            .await,
    );
    assert!(message.contains("bounds"), "honest error: {message}");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn element_shot_unknown_selector_times_out() {
    let path = socket_path("shot-missing.sock");
    let handle = spawn_session_server(path.clone(), scripted_adapter).expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    let (code, _message) = wire_error(
        client
            .request(RequestV1::Screenshot(ScreenshotV1::Element {
                selector: SelectorV1::TestId("no-such-node".into()),
                timeout_ms: 50,
            }))
            .await,
    );
    assert_eq!(code, ErrorCode::Timeout, "auto-wait expiry is a Timeout");

    handle.shutdown();
}
