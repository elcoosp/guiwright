//! The count comparators (§16 assertions row, M25): `expect(locator)
//! .to_have_count_greater_than(n)` / `.to_have_count_less_than(n)` — the
//! M22 record's reserved vocabulary widening on the count waiter, riding
//! the same event-driven wait, composing with the M24 negation decorator.
//!
//! ```text
//! cargo run -p guiwright-kit --example count_comparators
//! ```
//!
//! Asserts the fixture's counts against both comparators immediately,
//! scripts a late-appearing button plus a removal through the mock's
//! shared channel (the app mutates 300 ms in) so the comparators complete
//! event-driven in both directions, composes a negated comparator, and
//! prints the failure shape a timeout produces.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

fn main() {
    // The scripted channel emulates app-side rendering: an "Export"
    // button joins the fixture tree 300 ms in (count 4 → 5), then the
    // Save button leaves 300 ms after that (count 5 → 3 — the removal
    // update restates the parent without Save and without the export).
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
    let mut app = App::launch("count_comparators_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("count_comparators_demo.zip"));
    app.wait_for_settle();

    // The immediate comparators: 4 buttons clear > 3 and sit below < 5;
    // zero matches sit below any positive bound (strict mode is never
    // consulted — the whole match set is the answer).
    expect(app.role_locator(Role::Button))
        .to_have_count_greater_than(3)
        .expect("4 buttons clear the > 3 bound");
    expect(app.role_locator(Role::Button))
        .to_have_count_less_than(5)
        .expect("4 buttons sit below the < 5 bound");
    expect(app.get_by_role(Role::Button, "Ghost"))
        .to_have_count_less_than(1)
        .expect("0 ghosts sit below the < 1 bound");
    println!("count > 3 / count < 5 / count < 1: satisfied on the initial evaluation");

    // The mismatch is waitable in the widening direction: the assertion
    // completes exactly when the scripted diff lands the fifth button
    // (§15.3 — no polling, the waiter is woken by the tree change).
    expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_secs(2))
        .to_have_count_greater_than(4)
        .expect("the export button clears the > 4 bound");
    println!("count > 4: satisfied on the scripted late button");

    // …and in the narrowing direction: the removal drops the set below
    // the < 5 bound on the second scripted diff.
    expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_secs(2))
        .to_have_count_less_than(5)
        .expect("the removal drops the set below < 5");
    println!("count < 5: satisfied on the scripted removal");

    pusher.join().expect("script thread");

    // The M24 seam composes: not(> 99) holds right now (3 buttons do not
    // clear it), satisfied on the initial evaluation.
    expect(app.role_locator(Role::Button))
        .not()
        .to_have_count_greater_than(99)
        .expect("3 buttons do not clear > 99");
    println!("not(count > 99): satisfied through the negation decorator");

    // The failure shape: the timeout names the comparator, the locator
    // and the count the last evaluation found.
    let err = expect(app.role_locator(Role::Button))
        .with_timeout(Duration::from_millis(250))
        .to_have_count_greater_than(99)
        .expect_err("3 buttons never clear > 99");
    println!("timeout shape: {err}");
}
