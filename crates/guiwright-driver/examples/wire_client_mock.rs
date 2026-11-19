//! The mock seam row (m52): [`ClientConn::mock`] carries the §14
//! state-seam domain as typed sugar — one call taking the wire's own
//! [`MockV1`] enum (install / clear / call / reset-state) and answering the
//! [`MockReceipt`] carrier, the same shape the TypeScript client has
//! spoken since M3 (`mock()` / `callSeam()` / `resetState()`) and the REPL
//! since M4 (`mock install` / `clear` / `call` / `reset`). No wire change:
//! the command rides the `Mock` domain byte-for-byte as §14 blessed it;
//! every [`ClientConn`] impl inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_mock
//! # prints the canned override landing, the clear restoring the real
//! # hook, the runner-context call, the fixture-scope reset, and the
//! # typed rejections for an unknown id and a failing hook
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::MockV1;

/// The runner context the hooks write (§14: state lives in the adapter's
/// runner context; the session only routes).
#[derive(Debug, Default)]
struct DemoState {
    rows: usize,
    fixture: Option<String>,
}

fn main() {
    // App-boot seams (§14): the hooks `Mock::Call`/`ResetState` run — the
    // same registry shape `#[guiwright_kit::seam]` + `register_seams!`
    // produce for a real app.
    let mut seams = guiwright_driver::SeamRegistry::new();
    seams.register("reset_state", |ctx, scope| {
        let state = ctx
            .downcast_mut::<DemoState>()
            .ok_or("wrong runner context type")?;
        state.rows = 0;
        state.fixture = Some(scope.to_string());
        Ok(format!("state reset to {scope}"))
    });
    seams.register("seed_users", |ctx, _scope| {
        let state = ctx
            .downcast_mut::<DemoState>()
            .ok_or("wrong runner context type")?;
        state.rows = 3;
        Ok("3 users seeded".into())
    });
    seams.register("flaky_hook", |_ctx, _scope| Err("db offline".into()));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || {
                MockAdapter::with_fixture(&Fixture::buttons()).with_state(DemoState {
                    rows: 9,
                    fixture: None,
                })
            },
            seams,
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("in-process client connects");
        client
            .handshake("wire-client-mock-demo/0.1")
            .await
            .expect("handshake");

        // Install a canned override for the reset seam: every reset_state
        // answers the preset until the override is cleared (§7
        // `InstallSeam` → `Clear`, the REPL's `mock install` / `mock clear`).
        let receipt = client
            .mock(MockV1::InstallSeam {
                id: "reset_state".into(),
                response: Some("canned".into()),
            })
            .await
            .expect("install");
        println!("install → {}: {}", receipt.id, receipt.detail);

        // The canned override wins over the real hook.
        let receipt = client
            .mock(MockV1::ResetState {
                scope: "seed-42".into(),
            })
            .await
            .expect("canned reset");
        println!("reset   → {}: {}", receipt.id, receipt.detail);

        // Clear the override: the real hook runs again, in the adapter's
        // runner context (§14 "state writes need the runner's context").
        let receipt = client
            .mock(MockV1::Clear {
                id: "reset_state".into(),
            })
            .await
            .expect("clear");
        println!("clear   → {}: {}", receipt.id, receipt.detail);
        let receipt = client
            .mock(MockV1::ResetState {
                scope: "seed-42".into(),
            })
            .await
            .expect("real reset");
        println!("reset   → {}: {}", receipt.id, receipt.detail);

        // Call a registered seam directly (the REPL's `mock call` row).
        let receipt = client
            .mock(MockV1::Call {
                id: "seed_users".into(),
                scope: "smoke".into(),
            })
            .await
            .expect("call");
        println!("call    → {}: {}", receipt.id, receipt.detail);

        // The honesty: an unknown id, and a hook that fails, answer the
        // wire's typed rejection naming the cause — surfaced, never
        // re-mapped.
        let err = client
            .mock(MockV1::Call {
                id: "ghost".into(),
                scope: "smoke".into(),
            })
            .await
            .expect_err("unknown seam");
        println!("ghost   → {err}");
        let err = client
            .mock(MockV1::Call {
                id: "flaky_hook".into(),
                scope: "smoke".into(),
            })
            .await
            .expect_err("the hook fails");
        println!("flaky   → {err}");

        handle.shutdown();
    });
}
