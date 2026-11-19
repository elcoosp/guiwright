//! State seams (§14 — the mock domain): the `#[guiwright_kit::seam]`
//! attribute marking app functions as reset/init hooks, `register_seams!`
//! handing them to the session at app boot, and the full `app.mock()`
//! vocabulary — install a canned override, clear it back to the real
//! hook, call a registered seam once, and `reset_state(scope)` for
//! fixture parity between tests (each kit test gets a fresh app state,
//! mirroring iced's `Preset` and masonry's `edit_root_widget` fixture
//! pattern).
//!
//! ```text
//! cargo run -p guiwright-kit --example seam_mock
//! # prints the registered seam ids, the override answering instead of
//! # the hook, the clear restoring the hook (the runner context writing
//! # app state through run_in), the direct call, the fixture-scope
//! # reset, and the typed rejections for an unknown id and a hook that
//! # fails
//! ```
//!
//! The mock adapter runs the seams in its runner context (§14 "state
//! writes run in the runner context via `run_in`"); real adapters execute
//! the same compiled task behind the same seam.

use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, register_seams};

/// The demo app state — the runner context the seams write through
/// `run_in` (§14: app state is confined to the UI thread, so a seam is
/// compiled into a task and executed inside the adapter's context).
#[derive(Debug, Default)]
struct DemoState {
    rows: usize,
    fixture: Option<String>,
}

/// §14 app code, exactly the architecture shape: the reset hook the
/// `reset_state(scope)` convention runs (the default seam name). A
/// `Result<String, _>` answer rides the detail back to the test; an
/// empty scope is a test-authoring bug, so the hook refuses it (seams
/// may fail — the mock domain surfaces the error).
#[guiwright_kit::seam]
fn reset_state(state: &mut DemoState, scope: &str) -> Result<String, String> {
    if scope.trim().is_empty() {
        return Err("reset_state needs a non-empty scope".into());
    }
    state.rows = 0;
    state.fixture = Some(scope.to_string());
    Ok(format!("state reset to {scope}"))
}

/// A direct-call hook: seeds the demo rows (and refuses when the fixture
/// is closed — seams may fail, the mock domain surfaces the error).
#[guiwright_kit::seam]
fn seed_users(state: &mut DemoState) -> Result<String, String> {
    if state.fixture.is_none() {
        return Err("cannot seed a closed fixture".into());
    }
    state.rows = 3;
    Ok("3 users seeded".into())
}

fn main() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture).with_state(DemoState {
        rows: 12,
        fixture: None,
    });
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("seam_mock_demo", adapter);
    // App boot: hand the #[seam]-generated registrations to the session.
    app.register_seams(&register_seams![reset_state, seed_users]);
    app.wait_for_settle();

    // The registered ids (diagnostics — the REPL's `mock ids` row shape).
    println!("registered seams        → {:?}", app.mock().ids());

    // The honesty, first: the fixture starts closed (`fixture: None`), so
    // the seed hook refuses — a hook that fails surfaces its own error
    // text (the seam's `Result`), a clean named error, never a silent
    // no-op.
    let err = app
        .mock()
        .call("seed_users", "")
        .expect_err("closed fixture");
    println!("call seed_users (closed)→ clean error: {err}");

    // Install a canned override for the reset seam: reset_state answers
    // the preset until the override is cleared (§7 `InstallSeam` →
    // `Clear`), the same flip the REPL's `mock install` / `mock clear`
    // rows render.
    app.mock().install("reset_state", Some("canned".into()));
    let detail = app
        .mock()
        .reset_state("seed-42")
        .expect("the override answers");
    println!("reset_state (override)  → {detail}");

    // Clear the override: the real hook runs again, in the adapter's
    // runner context — app state is written through `run_in` (§14), and
    // the fixture context is now open.
    app.mock().clear("reset_state");
    let detail = app.mock().reset_state("seed-42").expect("the real hook");
    println!("reset_state (real hook) → {detail}");

    // The direct call now succeeds (the fixture is open — the hook's own
    // contract, run inside the runner context).
    let detail = app.mock().call("seed_users", "").expect("the seed");
    println!("call seed_users (open)  → {detail}");

    // An unknown id is a clean, named error too.
    let err = app.mock().call("ghost", "").expect_err("unknown id");
    println!("call ghost              → clean error: {err}");
}
