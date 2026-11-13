//! Rust-client window parity (§16 v0.4 "targets per window", M6): the
//! `Window` domain's two requests as typed methods on the client seam —
//! [`ClientConn::window_list`] and [`ClientConn::window_use`] — so a Rust
//! consumer gets the same conversation the REPL (`window list` /
//! `window use`) and the TypeScript client (`windowList()` /
//! `windowUse()`) already speak, over any door. Driven here over the
//! in-process door with the two-window fixture, whose two "Save" buttons
//! make the scope the only discriminator.

use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{
    ErrorCode, NodeKeyV1, RequestV1, ResponseV1, SelectorV1, TextMatchV1,
};
use guiwright_proto::{ClientConn, DuplexClient, GwpError};

/// Two-window server + pre-extracted handles: the fixture moves into the
/// adapter closure (FnOnce — one adapter per server), so the identity
/// bytes come out before the move.
fn spawn_two_window() -> (
    GwpServerHandle,
    guiwright_proto::duplex::DuplexConnect,
    String,
    String,
    [u8; 16],
) {
    let fx = MultiWindowFixture::default();
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();
    let w2_bytes = *fx.w2.0.as_bytes();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_multi_window(&fx),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, w1, w2, w2_bytes)
}

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client.handshake("wire-windows-test/0.1").await.unwrap();
    client
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_list_reports_the_adapter_targets() {
    let (_handle, connect, w1, w2, _w2_bytes) = spawn_two_window();
    let mut client = connected(&connect).await;

    let windows = client.window_list().await.expect("window targets");
    let rendered: Vec<(String, String, bool)> = windows
        .into_iter()
        .map(|w| (w.handle, w.title, w.focused))
        .collect();
    assert_eq!(
        rendered,
        vec![
            (w1, "Editor".to_string(), true),
            (w2, "Preview".to_string(), false),
        ],
        "the adapter's two targets, in report order"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_use_scopes_later_resolution() {
    let (_handle, connect, _w1, w2, w2_bytes) = spawn_two_window();
    let mut client = connected(&connect).await;

    // Aim at Preview, then resolve the ambiguous "Save": the connection's
    // active target scopes it to the Preview subtree (§16 v0.4).
    let used = client.window_use(&w2).await.expect("Preview aimed");
    assert_eq!(used, w2, "the ack echoes the aimed handle");

    let response = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: Some(TextMatchV1 {
                    text: "Save".into(),
                    exact: true,
                    ignore_case: false,
                }),
            },
            timeout_ms: 5000,
        })
        .await
        .expect("scoped resolve");
    match response {
        ResponseV1::Resolved { node, .. } => match node {
            NodeKeyV1::AccessKit { tree, .. } => {
                assert_eq!(
                    tree, w2_bytes,
                    "the resolved Save belongs to the aimed window"
                );
            }
            other => panic!("expected an AccessKit key, got {other:?}"),
        },
        other => panic!("expected Resolved, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_use_of_a_dead_handle_is_a_typed_error() {
    let (_handle, connect, _w1, _w2, _w2_bytes) = spawn_two_window();
    let mut client = connected(&connect).await;

    let err = client
        .window_use("00000000-dead-beef-0000-000000000000")
        .await
        .expect_err("a dead handle is a typed error");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::UnknownWindow, "{message}");
            assert!(
                message.contains("00000000-dead-beef-0000-000000000000"),
                "the error names the handle: {message}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_methods_ride_the_client_seam_not_a_transport() {
    // The methods live on the trait, so a consumer written against
    // `ClientConn` — the viewer's live feed, a downstream recorder —
    // gets window parity over any door without naming the transport.
    async fn targets<C: ClientConn>(conn: &mut C) -> Vec<String> {
        conn.window_list()
            .await
            .expect("window targets over the seam")
            .into_iter()
            .map(|w| w.handle)
            .collect()
    }

    let (_handle, connect, _w1, _w2, _w2_bytes) = spawn_two_window();
    let mut client = connected(&connect).await;
    let handles = targets(&mut client).await;
    assert_eq!(handles.len(), 2, "both targets through the generic seam");
}
