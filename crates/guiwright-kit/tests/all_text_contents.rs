//! The bulk text-read surface (§16 assertions row — the m26 record's open
//! gap: "bulk *content* reads … a dedicated bulk reader lands when a
//! suite asks for it"): `locator.all_text_contents()` reads the text of
//! every match in document order as one snapshot — the `all()`
//! enumeration's set read mapped to data instead of visitors, riding the
//! same non-strict `resolve_all` walk `count()` reads.
//!
//! The tests pin the read end-to-end on the mock adapter: document order
//! over the button fixture, the empty set, the narrowing and window-pin
//! scoping, and the per-node text rule (the same text the text locators
//! match — label first, then value).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

#[test]
fn the_button_set_reads_in_document_order() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("text_contents_order", adapter);
    let texts = app
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the set read");
    // Document order, hidden nodes included (the `all()`/`count()` set
    // semantics — Playwright's `allTextContents` never filters).
    assert_eq!(
        texts,
        vec![
            "Save".to_string(),
            "Cancel".to_string(),
            "Invisible".to_string(),
            "Confirm".to_string(),
        ]
    );
}

#[test]
fn an_empty_set_enumerates_to_no_texts() {
    let mut app = App::launch_fixture("text_contents_empty");
    let texts = app
        .get_by_role(Role::Button, "Ghost")
        .all_text_contents()
        .expect("the empty read");
    assert!(texts.is_empty(), "{texts:?}");
}

#[test]
fn narrowing_scopes_the_read() {
    let mut app = App::launch_fixture("text_contents_narrowed");
    let texts = app
        .get_by_role(Role::Button, "Save")
        .first()
        .all_text_contents()
        .expect("the narrowed read");
    assert_eq!(texts, vec!["Save".to_string()]);
}

#[test]
fn the_read_covers_labels_and_values() {
    // The per-node text rule: the same text the text locators match —
    // label first, then value. The fixture's text input carries both;
    // its label wins.
    let mut app = App::launch_fixture("text_contents_label_value");
    let texts = app
        .get_by_role(Role::TextInput, "Email")
        .all_text_contents()
        .expect("the input read");
    assert_eq!(texts, vec!["Email".to_string()]);
    let texts = app
        .get_by_text("Welcome, Ada")
        .all_text_contents()
        .expect("the label read");
    assert_eq!(texts, vec!["Welcome, Ada".to_string()]);
}

#[test]
fn the_window_pin_scopes_the_read() {
    // Two windows, both with a "Save" button: the unscoped read is
    // ambiguous by construction, and the pin scopes the set to one
    // subtree (the same discriminator the window-handle tests use).
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("text_contents_window", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();
    let texts = app
        .window(&w2)
        .expect("w2 aims")
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the scoped read");
    assert_eq!(texts, vec!["Save".to_string()], "w2's Save only");
}

#[test]
fn a_late_button_joins_the_next_read() {
    // The snapshot semantics: each read is one `resolve_all` walk — a
    // node that arrives later shows up on the next read (the `all()`
    // enumeration rule, mapped to data). The script thread signals its
    // push through a rendezvous channel so the second read is
    // deterministic (settle-then-read, no wall-clock guess).
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let (pushed, landed) = std::sync::mpsc::channel();
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("text_contents_late", adapter);
    let before = app
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the first read");
    assert_eq!(before.len(), 4, "the fixture's four buttons");
    let late = fixture.add_button_update("Export");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        scripts.lock().expect("script channel").push(late);
        pushed.send(()).expect("rendezvous");
    });
    landed
        .recv_timeout(Duration::from_secs(5))
        .expect("the script thread lands the update");
    app.wait_for_settle();
    let after = app
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the second read");
    assert_eq!(
        after,
        vec![
            "Save".to_string(),
            "Cancel".to_string(),
            "Invisible".to_string(),
            "Confirm".to_string(),
            "Export".to_string(),
        ]
    );
}

#[test]
fn the_read_records_nothing_but_the_trace_event() {
    // The `count()` rule: a read is not a replayable step — nothing
    // lands in the codegen output (actions record; reads don't).
    let mut app = App::launch_fixture("text_contents_codegen");
    app.enable_codegen();
    let _ = app
        .role_locator(Role::Button)
        .all_text_contents()
        .expect("the read");
    let code = app.codegen_code().expect("recording is enabled");
    assert_eq!(code.0.trim().lines().count(), 0, "no lines: {code:?}");
}
