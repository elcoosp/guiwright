//! The negated assertion surface (§16 assertions row, M24):
//! `expect(locator).not().to_*` — Playwright's `not` modifier over the
//! same event-driven wait as every other assertion.
//!
//! ```text
//! cargo run -p guiwright-kit --example negated_assertions
//! ```
//!
//! Asserts the fixture's negative facts immediately (absent, hidden,
//! disabled), scripts the Save button's removal through the mock's shared
//! channel (the app mutates 300 ms in) and completes the waitable
//! negatives on that tree diff, then prints the failure shapes negation
//! still produces (strict mode, the negated timeout header).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

fn main() {
    // The scripted channel emulates app-side rendering: the Save button
    // leaves the fixture tree 300 ms in.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let removal = fixture.remove_save_update();
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let pusher = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        scripts.lock().expect("script channel").push(removal);
    });
    let mut app = App::launch("negated_assertions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("negated_assertions_demo.zip"));
    app.wait_for_settle();

    // The immediate negatives: absence and the filter-excluded button are
    // not visible, the disabled button is not enabled — each satisfied on
    // the initial evaluation, no pump (Playwright's `not.toBeVisible()`).
    expect(app.get_by_role(Role::Button, "Ghost"))
        .not()
        .to_be_visible()
        .expect("the absent button is not visible");
    expect(app.get_by_role(Role::Button, "Invisible"))
        .not()
        .to_be_visible()
        .expect("the hidden button is not visible");
    expect(app.get_by_role(Role::Button, "Confirm"))
        .not()
        .to_be_enabled()
        .expect("confirm is not enabled (disabled)");
    println!("not(visible) x2 / not(enabled): satisfied on the initial evaluation");

    // The negation is waitable: Save is visible right now, so not(visible)
    // pends — and completes exactly when the scripted removal lands the
    // tree diff (§15.3 — no polling, the waiter is woken by the change).
    expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_secs(2))
        .not()
        .to_be_visible()
        .expect("the removal lands and the negation holds");
    println!("not(visible) on Save: satisfied on the scripted removal");

    // The count seam composes too: the removal moved the set off 4, so
    // the negated count holds against the fixture's original size.
    expect(app.role_locator(Role::Button))
        .not()
        .to_have_count(4)
        .expect("the removal moved the count off 4");
    println!("not(count 4): satisfied on the same scripted removal");

    pusher.join().expect("script thread");

    // Failure shapes negation still produces: strict mode is a resolution
    // failure, not a state mismatch, so it fails under not() as well…
    let err = expect(app.role_locator(Role::Button))
        .not()
        .to_be_visible()
        .expect_err("the remaining buttons are ambiguous either way");
    println!("strict shape: {err}");

    // …and the timeout header names the inverted assertion plus the
    // locator it was built from (Confirm stays visible until timeout).
    let err = expect(app.get_by_role(Role::Button, "Confirm"))
        .with_timeout(Duration::from_millis(250))
        .not()
        .to_be_visible()
        .expect_err("confirm stays visible");
    println!("timeout shape: {err}");
}
