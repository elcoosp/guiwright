//! The locator wait-for surface (§16 auto-waiting row, M21): Playwright's
//! `locator.waitFor(state)` over the session's event-driven loop — the
//! four-state vocabulary (`attached` / `visible` / `hidden` / `stable`)
//! mapped 1:1 onto the §8.3 check table.
//!
//! ```text
//! cargo run -p guiwright-kit --example wait_for_states
//! ```
//!
//! Scripts a late-appearing button through the mock's shared channel (the
//! app renders it 300 ms in), waits for it the way a real test would, and
//! prints the immediate states plus the failure shape a timeout produces.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, WaitState};

fn main() {
    // The scripted channel emulates app-side rendering: the "Export"
    // button joins the fixture tree 300 ms into the session.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let late = fixture.add_button_update("Export");
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let pusher = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        scripts.lock().expect("script channel").push(late);
    });
    let mut app = App::launch("wait_for_states_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("wait_for_states_demo.zip"));
    app.wait_for_settle();

    // The immediate states complete on the initial evaluation — no pump,
    // no poll (§15.3).
    app.get_by_role(Role::Button, "Save")
        .wait_for(WaitState::Visible)
        .expect("save is visible");
    println!("wait_for(visible)   → ok (Save is visible)");
    app.get_by_role(Role::Button, "Invisible")
        .wait_for(WaitState::Hidden)
        .expect("invisible button is hidden");
    println!("wait_for(hidden)    → ok (Invisible is hidden — absence passes)");

    // The late node: the wait completes exactly when the scripted diff
    // makes it visible — the event-driven wait, not a sleep.
    let started = std::time::Instant::now();
    app.get_by_role(Role::Button, "Export")
        .wait_for(WaitState::Visible)
        .expect("export appears and becomes visible");
    println!(
        "wait_for(visible)   → ok (Export after {} ms — completed on the tree diff)",
        started.elapsed().as_millis()
    );
    pusher.join().expect("script thread");

    // The failure shape: the timeout names the state and the locator.
    let err = app
        .get_by_role(Role::Button, "Ghost")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_millis(300))
        .expect_err("the ghost button never appears");
    println!("\nfailure shape: {err}");

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
