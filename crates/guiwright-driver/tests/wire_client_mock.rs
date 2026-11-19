//! The client seam's mock rows over the wire (m52): [`ClientConn::mock`]
//! carries the §14 state-seam domain as typed seam sugar — one call taking
//! the wire's own [`MockV1`] enum (install / clear / call / reset-state)
//! and answering the [`MockReceipt`] carrier (the seam id the reply names
//! and the detail the hook or override produced), the same shape the
//! TypeScript client has spoken since M3 (`mock()` / `callSeam()` /
//! `resetState()`) and the REPL since M4 (`mock install` / `clear` / `call`
//! / `reset`). **No wire change**: the sugar rides the `Mock` domain
//! byte-for-byte as §14 blessed it; every [`ClientConn`] impl inherits the
//! row. Driven here over the in-process door — the same reality
//! `remote_session.rs` pins at the raw-request level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{ErrorCode, MockV1};

/// The runner context the hooks write (§14: state lives in the adapter's
/// runner context; the session only routes).
#[derive(Debug, Default)]
struct SpikeState {
    rows: usize,
    fixture: Option<String>,
}

/// The app-boot seams the conversations drive: the session's default reset
/// seam, a direct-call seed hook, and a hook that fails so the wire's
/// error honesty is pinned as shipped.
fn spike_seams() -> guiwright_driver::SeamRegistry {
    let mut seams = guiwright_driver::SeamRegistry::new();
    seams.register("reset_state", |ctx, scope| {
        let state = ctx
            .downcast_mut::<SpikeState>()
            .ok_or("wrong runner context type")?;
        state.rows = 0;
        state.fixture = Some(scope.to_string());
        Ok(format!("state reset to {scope}"))
    });
    seams.register("seed_users", |ctx, _scope| {
        let state = ctx
            .downcast_mut::<SpikeState>()
            .ok_or("wrong runner context type")?;
        state.rows = 3;
        Ok("3 users seeded".into())
    });
    seams.register("flaky_hook", |_ctx, _scope| Err("db offline".into()));
    seams
}

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-mock/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    GwpServerHandle::spawn_inprocess(
        || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_state(SpikeState {
                rows: 9,
                fixture: None,
            })
        },
        spike_seams(),
    )
    .expect("in-process server starts")
}

/// The canned override wins until cleared (§7 `InstallSeam` → `Clear`):
/// `ResetState` answers the preset detail, and after the clear the real
/// hook runs again in the runner context — the same flip the REPL's
/// `mock install` / `mock clear` rows render and the TypeScript
/// `mock()` / `callSeam()` helpers chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_canned_override_wins_until_cleared() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let installed = client
        .mock(MockV1::InstallSeam {
            id: "reset_state".into(),
            response: Some("canned".into()),
        })
        .await
        .expect("the install");
    assert_eq!(installed.id, "reset_state", "the reply names the seam");
    assert_eq!(installed.detail, "seam override installed");

    let canned = client
        .mock(MockV1::ResetState {
            scope: "seed-42".into(),
        })
        .await
        .expect("the canned reset");
    assert_eq!(canned.detail, "canned", "the preset beats the real hook");

    let cleared = client
        .mock(MockV1::Clear {
            id: "reset_state".into(),
        })
        .await
        .expect("the clear");
    assert_eq!(cleared.detail, "seam override cleared");

    let real = client
        .mock(MockV1::ResetState {
            scope: "seed-42".into(),
        })
        .await
        .expect("the real reset");
    assert_eq!(
        real.detail, "state reset to seed-42",
        "the hook runs again after the clear"
    );

    handle.shutdown();
}

/// `Call` runs the registered hook once, inside the adapter's runner
/// context (§14 "state writes need the runner's context") — the detail is
/// the hook's own summary, the same line the REPL's `mock call` row prints.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_runs_the_real_hook_in_the_runner_context() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let receipt = client
        .mock(MockV1::Call {
            id: "seed_users".into(),
            scope: "smoke".into(),
        })
        .await
        .expect("the call");
    assert_eq!(receipt.id, "seed_users");
    assert_eq!(receipt.detail, "3 users seeded");

    handle.shutdown();
}

/// `ResetState` runs the session's configured reset seam (the default
/// `"reset_state"` id) with the caller's fixture scope — and the wire
/// names the scope in the `id` field of the reply (the domain reuses the
/// `MockDone.id` slot for the scope; the carrier surfaces it verbatim).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_state_runs_the_session_reset_seam() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let receipt = client
        .mock(MockV1::ResetState {
            scope: "seed-7".into(),
        })
        .await
        .expect("the reset");
    assert_eq!(receipt.id, "seed-7", "the reply names the scope");
    assert_eq!(receipt.detail, "state reset to seed-7");

    handle.shutdown();
}

/// The boundary's honesty: a seam id nothing registered answers the wire's
/// typed rejection, never a synthesized `MockDone` — the message names the
/// unknown id and the registered set (the session's own diagnostics).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_seam_answers_the_typed_error() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let err = client
        .mock(MockV1::Call {
            id: "ghost".into(),
            scope: "smoke".into(),
        })
        .await
        .expect_err("unknown seam is a typed rejection");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::Internal);
            assert!(
                message.contains("ghost"),
                "the message names the unknown id: {message}"
            );
            assert!(
                message.contains("registered"),
                "the message lists the registered ids: {message}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    handle.shutdown();
}

/// A hook that fails surfaces its own error text through the wire's typed
/// rejection (the adapter's runner-context error, mapped, not swallowed) —
/// the same honesty the kit's `#[seam]` flows give embedded sessions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_hook_surfaces_its_error() {
    let (handle, connect) = spawn();
    let mut client = connected(&connect).await;

    let err = client
        .mock(MockV1::Call {
            id: "flaky_hook".into(),
            scope: "smoke".into(),
        })
        .await
        .expect_err("the hook's error surfaces");
    match err {
        GwpError::Rejected { message, .. } => {
            assert!(
                message.contains("db offline"),
                "the hook's own error text rides the rejection: {message}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    handle.shutdown();
}
