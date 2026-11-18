//! The value-first bulk read (§16 assertions row — the m29 record's
//! named follow-up): `locator.all_values()` — the value of every match
//! in document order as one snapshot read, the
//! [`all_text_contents`](all_text_contents) walk with the source order
//! flipped for form semantics (value first, label fallback — the
//! `getByValue` source order). The same non-strict `resolve_all` walk
//! `count()` takes; the same zero-polling session loop (reads do not
//! wait — no polling anywhere).
//!
//! ```text
//! cargo run -p guiwright-kit --example all_values
//! ```
//!
//! Reads the button fixture's whole set (label fallback), the Email
//! input (value-first — where `all_text_contents` reads "Email", this
//! reads "ada@example.dev"), the narrowed set, and the window-scoped
//! set on the two-window fixture.

use accesskit::Role;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The fixture's four buttons carry labels, no values: the fallback
    // keeps every node answering (the totality rule — positions stay
    // 1:1 with the set read's document order, hidden nodes included).
    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("all_values_demo", adapter);
    app.wait_for_settle();

    let values = app
        .role_locator(Role::Button)
        .all_values()
        .expect("the fallback read");
    println!("role_locator(Button).all_values()   → {values:?}");

    // The form-dump semantics: the Email input carries both a label and
    // a value — this read takes the value first, where
    // `all_text_contents` takes the label ("Email").
    let values = app
        .get_by_role(Role::TextInput, "Email")
        .all_values()
        .expect("the value read");
    println!("get_by_role(TextInput).all_values() → {values:?}");
    let texts = app
        .get_by_role(Role::TextInput, "Email")
        .all_text_contents()
        .expect("the label read");
    println!("get_by_role(TextInput).all_text_contents() → {texts:?}");

    // Narrowing scopes the read exactly like it scopes an action.
    let values = app
        .get_by_role(Role::TextInput, "Email")
        .first()
        .all_values()
        .expect("the narrowed read");
    println!("first().all_values()                → {values:?}");

    // The window pin scopes the set to one subtree (§16 v0.4): the
    // two-window fixture carries a "Save" in each window; the pinned
    // handle reads only its own.
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("all_values_windows", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    let values = app
        .window(&w2)
        .expect("w2 aims")
        .role_locator(Role::Button)
        .all_values()
        .expect("the scoped read");
    println!("window(w2).all_values()             → {values:?}");

    // The read is not a replayable step: like count(), nothing lands in
    // the codegen output — actions record, reads don't.
    app.enable_codegen();
    let _ = app
        .role_locator(Role::Button)
        .all_values()
        .expect("the unrecorded read");
    let (code, warnings) = app.codegen_code().expect("recording is enabled");
    println!(
        "codegen after the read              → {} line(s), {} warning(s) (reads record nothing)",
        code.trim().lines().count(),
        warnings.len()
    );
}
