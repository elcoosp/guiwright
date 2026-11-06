//! Kit integration tests (M1 contract): the §6.3 client API end-to-end on
//! the embedded fixture app, the §10 screenshot flow, §12 traces, and the
//! `#[guiwright_kit::test]` macro.

use std::io::Read;
use std::time::Duration;

use accesskit::Role;
use guiwright_kit::{App, BlessMode, SnapshotOutcome, expect};

#[test]
fn locator_click_runs_actionability_and_reaches_the_adapter() {
    let mut app = App::launch_fixture("locator_click");
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .click()
        .expect("click");
    assert_eq!(outcome.detail, "clicked");
    let actions = app.session().adapter().actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].1, guiwright_driver::SemAction::Click);
}

#[test]
fn locator_fill_and_expect_value() {
    let mut app = App::launch_fixture("fill_value");
    app.get_by_role(Role::TextInput, "Email")
        .fill("ada@lovelace.dev")
        .expect("fill");
    // The mock records the action (it cannot mutate a scripted tree);
    // the expect path asserts against the fixture's value.
    let actions = app.session().adapter().actions();
    assert_eq!(
        actions[0].1,
        guiwright_driver::SemAction::SetValue {
            text: "ada@lovelace.dev".into()
        }
    );
    expect(app.get_by_role(Role::TextInput, "Email"))
        .to_have_value("ada@example.dev")
        .expect("value asserted");
}

#[test]
fn expect_hidden_and_visible() {
    let mut app = App::launch_fixture("expect_visibility");
    expect(app.get_by_role(Role::Button, "Save"))
        .to_be_visible()
        .expect("visible");
    expect(app.get_by_role(Role::Button, "Ghost"))
        .to_be_hidden()
        .expect("hidden by absence (Playwright parity)");
    expect(app.get_by_role(Role::Button, "Confirm"))
        .to_be_disabled()
        .expect("disabled");
}

#[test]
fn strict_mode_rejects_ambiguous_role_locator() {
    let mut app = App::launch_fixture("strict_mode");
    let err = app
        .role_locator(Role::Button)
        .resolve()
        .expect_err("four buttons match");
    assert!(err.to_string().contains("resolved to"), "{err}");
}

#[test]
fn filters_narrow_ambiguous_locators() {
    let mut app = App::launch_fixture("filters");
    let clicked = app
        .role_locator(Role::Button)
        .first()
        .click()
        .expect("first button clicked");
    assert_eq!(clicked.detail, "clicked");
}

#[test]
fn settle_and_clock_flow() {
    let mut app = App::launch_fixture("clock_flow");
    app.wait_for_settle();
    app.clock_fast_forward(Duration::from_secs(30))
        .expect("mock adapter supports the clock");
    assert_eq!(
        app.session().adapter().virtual_time(),
        Some(Duration::from_secs(30))
    );
}

#[test]
fn screenshot_flow_bless_match_mismatch_artifacts() {
    let tmp = tempfile::tempdir().expect("tmp");
    let pixels = vec![10u8; 8 * 8 * 4];

    // Missing baseline without bless → guidance error.
    let mut app = App::launch_fixture_with("shots", |adapter| {
        adapter.with_frame_pixels(8, 8, pixels.clone())
    })
    .with_baseline_dir(tmp.path().to_path_buf());
    let err = app
        .screenshot("main-empty", BlessMode::Off)
        .expect_err("no baseline yet");
    assert!(err.to_string().contains("GUIWRIGHT_BLESS=1"), "{err}");

    // Bless → baseline written, outcome Blessed.
    let outcome = app
        .screenshot("main-empty", BlessMode::Failing)
        .expect("bless");
    assert_eq!(outcome, SnapshotOutcome::Blessed { forced: false });

    // Same pixels → Matched, nothing written.
    let outcome = app.screenshot("main-empty", BlessMode::Off).expect("match");
    assert_eq!(outcome, SnapshotOutcome::Matched { failed_pixels: 0 });
    let baseline = tmp.path().join("main-empty");
    let names: Vec<_> = std::fs::read_dir(&baseline)
        .expect("baseline dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !names.iter().any(|f| f.contains(".new.png")),
        "no artifacts on success: {names:?}"
    );

    // Different pixels → Mismatch + artifacts next to the baseline.
    let mut app = App::launch_fixture_with("shots", |adapter| {
        adapter.with_frame_pixels(8, 8, vec![250u8; 8 * 8 * 4])
    })
    .with_baseline_dir(tmp.path().to_path_buf());
    let err = app
        .screenshot("main-empty", BlessMode::Off)
        .expect_err("pixels changed");
    let msg = err.to_string();
    assert!(msg.contains("pixels differ"), "{msg}");
    let names: Vec<_> = std::fs::read_dir(&baseline)
        .expect("baseline dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().any(|f| f.contains(".new.png")), "{names:?}");
    assert!(names.iter().any(|f| f.contains(".diff.png")), "{names:?}");
}

#[test]
fn screenshot_without_pixel_support_is_loud() {
    let mut app = App::launch_fixture("no_pixels");
    let err = app
        .screenshot("whatever", BlessMode::Off)
        .expect_err("mock has no pixels by default");
    assert!(err.to_string().contains("render_frame"), "{err}");
}

#[test]
fn trace_lifecycle_writes_the_gwtrace_archive() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = App::launch_fixture("traced_test");
    app.enable_trace(tmp.path().join("trace.zip"));
    app.get_by_role(Role::Button, "Save")
        .click()
        .expect("click");
    app.get_by_role(Role::TextInput, "Email")
        .fill("ada@lovelace.dev")
        .expect("fill");
    let path = app.finish().expect("trace path");
    assert!(path.exists(), "{path:?}");

    let bytes = std::fs::read(&path).expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut meta = String::new();
    archive
        .by_name("meta.json")
        .expect("meta")
        .read_to_string(&mut meta)
        .expect("read");
    let meta: serde_json::Value = serde_json::from_str(&meta).expect("json");
    assert_eq!(meta["schema"], "gwtrace/v1");
    assert_eq!(meta["test"], "traced_test");
    assert_eq!(meta["adapter"]["name"], "mock");

    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .expect("events")
        .read_to_string(&mut events)
        .expect("read");
    assert!(events.contains("\"click\""), "click recorded: {events}");
    assert!(events.contains("fill"), "fill recorded");
}

#[test]
fn finish_is_idempotent() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = App::launch_fixture("idem_test");
    app.enable_trace(tmp.path().join("idem.zip"));
    let first = app.finish().expect("first");
    let second = app.finish().expect("second returns the same path");
    assert_eq!(first, second);
}

#[test]
fn nextest_context_names_artifacts() {
    let app = App::launch_fixture("ctx_test");
    let ctx = app.context();
    assert_eq!(ctx.test_name, "ctx_test");
    assert!(!ctx.under_nextest, "plain cargo test in this suite");
    assert!(ctx.attempt.is_none() || ctx.under_nextest);
}

// --- #[guiwright_kit::test] macro end-to-end -------------------------------

#[guiwright_kit::test]
fn macro_unit_body(app: &mut guiwright_kit::App) {
    app.get_by_role(Role::Button, "Save").click().unwrap();
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_be_visible()
        .unwrap();
}

#[guiwright_kit::test]
fn macro_result_body(app: &mut guiwright_kit::App) -> Result<(), String> {
    app.get_by_role(Role::TextInput, "Email")
        .fill("ada@lovelace.dev")
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[guiwright_kit::test]
#[should_panic(expected = "guiwright test `macro_failing_result` failed: boom")]
fn macro_failing_result(_app: &mut guiwright_kit::App) -> Result<(), String> {
    Err("boom".into())
}

#[guiwright_kit::test]
#[should_panic(expected = "resolved to")]
fn macro_strict_mode_panics(app: &mut guiwright_kit::App) {
    // Four buttons — the unnamed role locator must fail strict mode.
    app.role_locator(Role::Button).resolve().unwrap();
}
