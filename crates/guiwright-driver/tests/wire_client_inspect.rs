//! The client seam's inspect rows over the wire (m53): [`ClientConn::inspect`]
//! carries the §13 inspector domain as typed seam sugar — one call taking
//! the wire's own [`InspectV1`] enum (start/stop the codegen recording,
//! enable/disable the overlay, aim/clear the highlight) and answering the
//! [`InspectReply`] union: the ack-only commands answer
//! [`InspectReply::Applied`], the recording stop answers
//! [`InspectReply::Codegen`] carrying the generated kit code and its
//! strict-mode warnings. The same shapes the TypeScript client has spoken
//! since M3 (`inspectOverlay()` / `inspectHighlight()`) and the REPL since
//! M4/M8 (`codegen start` / `stop`, `inspect` / `highlight`). **No wire
//! change**: the sugar rides the `Inspect` domain byte-for-byte as §13
//! blessed it; every [`ClientConn`] impl inherits the row. Driven here over
//! the in-process door — the same reality `remote_session.rs` pins at the
//! raw-request level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::InspectReply;
use guiwright_proto::schema::{
    ErrorCode, InspectV1, NodeKeyV1, SelectorV1, SemActionV1, TextMatchV1,
};

fn named_button(name: Option<&str>) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: name.map(|name| TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-inspect/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts")
}

/// The overlay session is ack-only on the wire: enable and disable answer
/// [`InspectReply::Applied`] — the same acks the REPL's `inspect` rows
/// render and the TypeScript `inspectOverlay()` helper chains.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlay_enable_disable_answer_the_ack() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let ack = client
        .inspect(InspectV1::EnableOverlay)
        .await
        .expect("the enable");
    assert!(matches!(ack, InspectReply::Applied), "got {ack:?}");

    let ack = client
        .inspect(InspectV1::DisableOverlay)
        .await
        .expect("the disable");
    assert!(matches!(ack, InspectReply::Applied), "got {ack:?}");

    handle.shutdown();
}

/// The highlight aims the outline at a resolved row (the dump-row rule's
/// handle form), and `Highlight { node: None }` clears it while the
/// overlay session stays on. A handle the tree does not know answers the
/// wire's typed `UnknownNode` — the M8 boundary's own honesty.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn highlight_aims_clears_and_rejects_ghost_handles() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let row = client
        .resolve(named_button(Some("Save")), 5_000)
        .await
        .expect("the resolve");

    let ack = client
        .inspect(InspectV1::Highlight {
            node: Some(row.node),
        })
        .await
        .expect("the highlight");
    assert!(matches!(ack, InspectReply::Applied), "got {ack:?}");

    let ack = client
        .inspect(InspectV1::Highlight { node: None })
        .await
        .expect("the clear");
    assert!(matches!(ack, InspectReply::Applied), "got {ack:?}");

    let ghost = NodeKeyV1::AccessKit {
        tree: [0u8; 16],
        node: 999,
    };
    let err = client
        .inspect(InspectV1::Highlight { node: Some(ghost) })
        .await
        .expect_err("the ghost handle cannot highlight");
    let GwpError::Rejected {
        code: ErrorCode::UnknownNode,
        ..
    } = err
    else {
        panic!("expected UnknownNode, got {err:?}");
    };

    handle.shutdown();
}

/// The §13 recorder, end to end over the seam: start the recording, act
/// (the aim-and-act sugar rides the same session), stop and receive the
/// generated kit code naming the acted locator — the same artifact the
/// REPL's `codegen stop` row prints. A stop with no recording active
/// answers the wire's typed rejection (the domain's idempotence rule).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codegen_records_the_acted_surface_and_stop_refuses_when_idle() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let ack = client
        .inspect(InspectV1::StartRecording)
        .await
        .expect("the start");
    assert!(matches!(ack, InspectReply::Applied), "got {ack:?}");

    let row = client
        .resolve(named_button(Some("Save")), 5_000)
        .await
        .expect("the resolve");
    client
        .action(row.node, row.generation, SemActionV1::Click, false, 5_000)
        .await
        .expect("the click");

    let reply = client
        .inspect(InspectV1::StopRecording)
        .await
        .expect("the stop");
    let InspectReply::Codegen(output) = reply else {
        panic!("expected Codegen, got {reply:?}");
    };
    assert!(
        output.code.contains("get_by_role"),
        "the generated code names the locator: {}",
        output.code
    );
    assert!(
        output.code.contains("click()"),
        "the generated code names the action: {}",
        output.code
    );

    let err = client
        .inspect(InspectV1::StopRecording)
        .await
        .expect_err("no recording is active anymore");
    let GwpError::Rejected { message, .. } = err else {
        panic!("expected Rejected");
    };
    assert!(
        message.contains("no codegen recording is active"),
        "the message names the cause: {message}"
    );

    handle.shutdown();
}

/// Starting a second recording while one is active answers the wire's
/// typed rejection — the session-side recorder is single-slot (§13).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_start_refuses_while_a_recording_is_active() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    client
        .inspect(InspectV1::StartRecording)
        .await
        .expect("the first start");
    let err = client
        .inspect(InspectV1::StartRecording)
        .await
        .expect_err("the second start");
    let GwpError::Rejected { message, .. } = err else {
        panic!("expected Rejected");
    };
    assert!(
        message.contains("already active"),
        "the message names the cause: {message}"
    );

    handle.shutdown();
}
