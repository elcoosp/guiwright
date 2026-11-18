//! The bulk text-read surface (§16 assertions row — the m26 record's open
//! kit gap closed): `locator.all_text_contents()` — the text of every
//! match in document order as one snapshot read, the `all()` enumeration's
//! set read mapped to data instead of visitors. The same non-strict
//! `resolve_all` walk `count()` takes; the same zero-polling session loop
//! (reads do not wait — no polling anywhere).
//!
//! ```text
//! cargo run -p guiwright-kit --example all_text_contents
//! ```
//!
//! Reads the button fixture's whole set, the narrowed set, the
//! window-scoped set on the two-window fixture, and prints the
//! snapshot rules the read follows.

use accesskit::Role;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The button fixture: "Save", "Cancel", the hidden "Invisible" and the
    // disabled "Confirm" — the whole set, hidden nodes included (the
    // all()/count() set semantics; nothing filters the walk).
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("all_text_contents_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("all_text_contents_demo.zip"));
    app.wait_for_settle();

    let texts = app
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the set read");
    println!("role_locator(Button).all_text_contents() → {texts:?}");

    // Narrowing scopes the read exactly like it scopes an action.
    let texts = app
        .get_by_role(Role::Button, "Save")
        .first()
        .all_text_contents()
        .expect("the narrowed read");
    println!("first().all_text_contents()               → {texts:?}");

    // The per-node text rule: the same text the text locators match —
    // label first, then value.
    let texts = app
        .get_by_role(Role::TextInput, "Email")
        .all_text_contents()
        .expect("the input read");
    println!("get_by_role(TextInput).all_text_contents() → {texts:?}");

    // The window pin scopes the set to one subtree (§16 v0.4): the
    // two-window fixture carries a "Save" in each window; the pinned
    // handle reads only its own.
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "all_text_contents_windows",
        MockAdapter::with_multi_window(&fx),
    );
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    let texts = app
        .window(&w2)
        .expect("w2 aims")
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the scoped read");
    println!("window(w2).role_locator(Button).…          → {texts:?}");

    // The read is not a replayable step: like count(), nothing lands in
    // the codegen output — actions record, reads don't.
    app.enable_codegen();
    let _ = app
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the unrecorded read");
    let (code, warnings) = app.codegen_code().expect("recording is enabled");
    println!(
        "codegen after the read                     → {} line(s), {} warning(s) (reads record nothing)",
        code.trim().lines().count(),
        warnings.len()
    );

    let trace = app.finish();
    if let Some(path) = trace {
        println!("\ntrace: {}", path.display());
    }
}
