//! The count surface (§16 assertions row, M22): `expect(locator).to_have_count(n)`
//! and `locator.count()` — Playwright's `toHaveCount` / `count()` escape
//! hatch from strict mode, riding the same event-driven wait as every
//! other assertion.
//!
//! ```text
//! cargo run -p guiwright-kit --example count_assertions
//! ```
//!
//! Counts the fixture's buttons immediately, scripts a late-appearing
//! button plus a removal through the mock's shared channel (the app
//! mutates 300 ms in), and prints the failure shape a timeout produces.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

fn main() {
    // The scripted channel emulates app-side rendering: an "Export"
    // button joins the fixture tree 300 ms in, then the Save button
    // leaves 300 ms after that.
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let late = fixture.add_button_update("Export");
    let removal = fixture.remove_save_update();
    let adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let pusher = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        scripts.lock().expect("script channel").push(late);
        std::thread::sleep(Duration::from_millis(300));
        scripts.lock().expect("script channel").push(removal);
    });
    let mut app = App::launch("count_assertions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("count_assertions_demo.zip"));
    app.wait_for_settle();

    // The sync read: how many nodes match right now — no strict check,
    // the whole match set is the answer (Playwright's locator.count()).
    let now = app.role_locator(Role::Button).count().expect("count reads");
    println!("buttons right now: {now}");

    // The waitable assertion: 4 buttons is the fixture's count, and 0
    // matches for a ghost is a count too (strict mode is never consulted).
    expect(app.role_locator(Role::Button))
        .to_have_count(4)
        .expect("four buttons");
    expect(app.get_by_role(Role::Button, "Ghost"))
        .to_have_count(0)
        .expect("no ghosts");
    println!("count 4 / count 0: satisfied on the initial evaluation");

    // The mismatch is waitable in both directions: the count completes
    // exactly when the scripted diff lands the fifth button (§15.3 — no
    // polling, the waiter is woken by the tree diff), then again when the
    // removal drops the set back to 4.
    expect(app.role_locator(Role::Button))
        .to_have_count(5)
        .expect("the export button lands the fifth");
    println!("count 5: satisfied on the scripted tree diff");
    expect(app.role_locator(Role::Button))
        .to_have_count(3)
        .expect("the removal drops the set to 3");
    println!("count 3: satisfied on the scripted removal");

    pusher.join().expect("script thread");

    // The failure shape: the timeout names the expected count, the
    // locator and the count the last evaluation found.
    let err = expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_millis(250))
        .to_have_count(99)
        .expect_err("99 buttons never arrive");
    println!("timeout shape: {err}");
}
