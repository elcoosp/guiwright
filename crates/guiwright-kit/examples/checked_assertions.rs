//! The checked-state assertion surface (§16 assertions row, M20): the
//! AccessKit tri-state (`Toggled::True/False/Mixed`) read where tests live
//! — `expect(locator).to_be_checked()` and its `to_be_unchecked` /
//! `to_be_checked_mixed` pair, over the same event-driven wait as every
//! other `to_*` assertion (no polling; the same §8.3 vocabulary the
//! actionability gate speaks).
//!
//! ```text
//! cargo run -p guiwright-kit --example checked_assertions
//! ```
//!
//! Asserts all three states against the fixture's checkbox trio and prints
//! the failure shape the waitable (and the fail-fast) paths produce, then
//! leaves a `gwtrace/v1` archive of the session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The checkbox-trio fixture: the M0 buttons fixture plus
    // "Notifications" (checked), "Dark mode" (unchecked) and
    // "Partially installed" (mixed). Any real adapter exposing a checkbox
    // through its AccessKit tree drops in unchanged.
    let fixture = guiwright_core::test_util::Fixture::with_checkboxes();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("checked_assertions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("checked_assertions_demo.zip"));
    app.wait_for_settle();

    // The three tri-state assertions (§8 check vocabulary, M20). Each
    // completes exactly when the state holds — here, on the initial
    // evaluation, because the fixture already carries its final states.
    guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Notifications"))
        .to_be_checked()
        .expect("notifications is checked");
    println!("to_be_checked        → ok (Notifications is Toggled::True)");

    guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Dark mode"))
        .to_be_unchecked()
        .expect("dark mode is unchecked");
    println!("to_be_unchecked      → ok (Dark mode is Toggled::False)");

    guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Partially installed"))
        .to_be_checked_mixed()
        .expect("partially installed is mixed");
    println!("to_be_checked_mixed  → ok (Partially installed is Toggled::Mixed)");

    // The failure shape: a state that has not flipped yet waits (retry
    // until the state holds or the timeout expires) and names the
    // assertion, the locator and the mismatched state in the error.
    let err = guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Dark mode"))
        .with_timeout(std::time::Duration::from_millis(300))
        .to_be_checked()
        .expect_err("dark mode has not flipped");
    println!("\nfailure shape (waitable): {err}");

    // …and the fail-fast shape: a node that never exposes a checked state
    // refuses immediately instead of burning the timeout (the world cannot
    // change it).
    let started = std::time::Instant::now();
    let err = guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_be_checked()
        .expect_err("a button is not checkable");
    println!(
        "failure shape (fail-fast, {} ms): {err}",
        started.elapsed().as_millis()
    );

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
