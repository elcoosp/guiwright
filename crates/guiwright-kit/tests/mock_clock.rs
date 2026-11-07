//! Kit M2 tests (§14): the clock surface, the mock domain with state
//! seams, the `#[guiwright_kit::seam]` + `register_seams!` macro pair,
//! and the clock ledger landing in the trace archive.

use std::io::Read;
use std::time::Duration;

use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, SeamRegistry, register_seams};

/// A demo app state — the runner context the seams write through `run_in`.
#[derive(Debug, Default)]
struct DemoState {
    rows: usize,
    fixture: Option<String>,
}

/// §14 app code, exactly the architecture shape:
/// `#[guiwright::seam] fn reset_db() -> Result<(), AppError>`.
#[guiwright_kit::seam]
fn reset_db(state: &mut DemoState, scope: &str) -> Result<(), String> {
    state.rows = 0;
    state.fixture = Some(scope.to_string());
    Ok(())
}

#[guiwright_kit::seam]
fn seed_users(state: &mut DemoState) -> Result<String, String> {
    state.rows = 3;
    state.fixture = Some("seeded".into());
    Ok("3 users seeded".into())
}

fn fixture_app(test: &str) -> App<MockAdapter> {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture).with_state(DemoState {
        rows: 12,
        fixture: None,
    });
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch(test, adapter);
    // App boot: hand the #[seam]-generated registrations to the session.
    app.register_seams(&register_seams![reset_db, seed_users]);
    app
}

#[test]
fn clock_surface_full_vocabulary() {
    let mut app = fixture_app("clock_surface");
    let mut clock = app.clock();

    let report = clock.install_virtual().expect("install");
    assert_eq!(report.state.virtual_time, Some(Duration::ZERO));

    let report = clock.fast_forward(Duration::from_secs(1)).expect("ff");
    assert_eq!(report.steps, 60, "1 s at 60 Hz = frame-sized steps");
    assert_eq!(report.state.virtual_time, Some(Duration::from_secs(1)));

    let report = clock.pause().expect("pause");
    assert!(report.state.paused);
    clock
        .fast_forward(Duration::from_millis(20))
        .expect("virtual jumps allowed while paused");
    let err = clock
        .run_for(Duration::from_millis(10))
        .expect_err("run_for refused while paused");
    assert!(err.to_string().contains("paused"), "{err}");

    clock.resume().expect("resume");
    clock.run_for(Duration::from_millis(10)).expect("run_for");

    clock
        .set_time(Duration::from_millis(120_000))
        .expect("set time");
    let state = clock.state().expect("probe");
    assert_eq!(state.virtual_time, Some(Duration::from_secs(120)));
    assert!(!state.paused);
}

#[test]
fn seams_write_the_runner_context_via_run_in() {
    let mut app = fixture_app("mock_seams");
    let mock = app.mock();
    let ids = mock.ids();
    assert_eq!(ids, vec!["reset_db", "seed_users"], "boot registration");

    // §14: reset_state → fixture parity (state written in the runner ctx).
    // The demo app names its reset seam `reset_db`; point reset_state at it.
    app.mock().with_reset_seam("reset_db");
    let detail = app.mock().reset_state("seed-42").expect("reset");
    assert_eq!(detail, "ok", "unit-Ok seam reports ok");
    let state = app
        .session()
        .adapter()
        .state::<DemoState>()
        .expect("runner context");
    assert_eq!(state.rows, 0);
    assert_eq!(state.fixture.as_deref(), Some("seed-42"));

    // Direct call on a string-returning seam: the string is the detail.
    let detail = app.mock().call("seed_users", "").expect("seed");
    assert_eq!(detail, "3 users seeded");
    let state = app.session().adapter().state::<DemoState>().unwrap();
    assert_eq!(state.rows, 3);
}

#[test]
fn mock_overrides_win_and_clear_restores() {
    let mut app = fixture_app("mock_overrides");
    app.mock()
        .install("reset_db", Some("canned".into()))
        .call("reset_db", "seed-42")
        .expect("canned wins");
    let state = app.session().adapter().state::<DemoState>().unwrap();
    assert_eq!(state.rows, 12, "the real hook never ran");

    app.mock().clear("reset_db");
    let detail = app.mock().call("reset_db", "seed-42").expect("real hook");
    assert_eq!(detail, "ok");
    let state = app.session().adapter().state::<DemoState>().unwrap();
    assert_eq!(state.rows, 0, "the real hook ran after clear");
}

#[test]
fn unknown_seam_is_a_clean_error() {
    let mut app = fixture_app("mock_unknown");
    let err = app
        .mock()
        .call("missing", "")
        .expect_err("unregistered seam");
    assert!(err.to_string().contains("no seam registered"), "{err}");
}

#[test]
fn clock_ledger_lands_in_the_trace_archive() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = fixture_app("clock_ledger_trace");
    app.enable_trace(tmp.path().join("trace.zip"));
    let mut clock = app.clock();
    clock.install_virtual().expect("install");
    clock.fast_forward(Duration::from_millis(50)).expect("ff");
    let path = app.finish().expect("trace path");

    let bytes = std::fs::read(&path).expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut clock_bin = Vec::new();
    archive
        .by_name("clock.bin")
        .expect("clock.bin present")
        .read_to_end(&mut clock_bin)
        .expect("read");
    let jumps: Vec<guiwright_trace::ClockJump> =
        postcard::from_bytes(&clock_bin).expect("postcard ledger");
    assert!(
        jumps.len() >= 2,
        "install + folded fast-forward recorded, got {}",
        jumps.len()
    );
    assert!(matches!(
        jumps[0].command,
        guiwright_trace::ClockCommand::InstallVirtual
    ));
    let total_delta: u64 = jumps[1..]
        .iter()
        .map(|j| match j.command {
            guiwright_trace::ClockCommand::FastForward { micros } => micros,
            _ => 0,
        })
        .sum();
    assert_eq!(
        total_delta, 50_000,
        "the fast-forward delta replays exactly"
    );
}

/// An empty registry is legal (no seams, everything else works).
#[test]
fn empty_registry_registers_cleanly() {
    let reg = SeamRegistry::new();
    let mut app = App::launch_fixture("empty_seams");
    app.register_seams(&reg);
    assert_eq!(app.mock().ids(), Vec::<&str>::new());
    app.get_by_role(accesskit::Role::Button, "Save")
        .click()
        .expect("app still works without seams");
}

#[test]
fn finished_trace_exports_a_self_contained_viewer() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = fixture_app("viewer_export");
    app.enable_trace(tmp.path().join("trace.zip"));
    app.get_by_role(accesskit::Role::Button, "Save")
        .click()
        .expect("click");
    app.finish().expect("trace path");

    let out = tmp.path().join("viewer").join("trace.html");
    let written = app.export_trace_viewer(&out).expect("export");
    assert_eq!(written, out);
    let html = std::fs::read_to_string(&out).expect("read");
    assert!(html.starts_with("<!doctype html>"), "self-contained viewer");
    assert!(html.contains("viewer_export"), "test name embedded");
}

#[test]
fn viewer_export_without_trace_is_a_clean_error() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = App::launch_fixture("viewer_no_trace");
    let err = app
        .export_trace_viewer(tmp.path().join("v.html"))
        .expect_err("nothing finished");
    assert!(err.to_string().contains("no trace archive"), "{err}");
}
