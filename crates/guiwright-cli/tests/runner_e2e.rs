//! M5 end-to-end: the own runner over real sockets (§16 v0.5 "own runner"
//! and §5.2 "nextest/report glue, sharding"). The M4 pattern repeats — the
//! framework-free mock adapter + a real socket is the launch-mode contract.
//!
//! Covered: discovery (files, recursive directories, strict spec errors),
//! a green run over UDS, failure semantics + spec-level timeouts, retry
//! counting, list mode (no live app needed), shard filtering, workers > 1,
//! CLI target override over the spec target, and the report files.

use std::path::{Path, PathBuf};

use guiwright_cli::ConnectTarget;
use guiwright_cli::report::{ReportKind, ScenarioStatus};
use guiwright_cli::runner::{self, RunOutcome, RunnerError, TestOptions};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;

/// Leak a temp dir so paths survive until process end (the repl_e2e
/// helper pattern).
fn leak_dir(tag: &str) -> PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    let path = dir.path().join(tag);
    std::fs::create_dir_all(&path).expect("mkdir");
    path
}

fn write_spec(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("write spec");
}

fn spawn_server(tag: &str) -> (PathBuf, GwpServerHandle) {
    let dir = leak_dir(tag);
    let path = dir.join(format!("{tag}.sock"));
    let handle = GwpServerHandle::spawn_with_seams(
        path.clone(),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("server binds");
    (path, handle)
}

fn uds_opts(path: &Path) -> TestOptions {
    TestOptions {
        target: Some(ConnectTarget::Uds(path.to_path_buf())),
        ..TestOptions::default()
    }
}

const LOGIN_SPEC: &str = r#"
[[scenario]]
name = "save the document"
steps = [
    "get role button Save",
    "click",
    "expect role label \"Welcome, Ada\"",
]

[[scenario]]
name = "email value"
steps = ["get value ada@example.dev"]
"#;

const SMOKE_SPEC: &str = r#"
[[scenario]]
name = "ping"
steps = ["ping"]

[[scenario]]
name = "clock state"
steps = ["clock state"]
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs_discovered_specs_to_green_over_uds() {
    let dir = leak_dir("runner-green");
    write_spec(&dir, "login.spec.toml", LOGIN_SPEC);
    write_spec(&dir, "smoke.spec.toml", SMOKE_SPEC);
    let (path, _server) = spawn_server("runner-green");

    let specs = runner::discover(std::slice::from_ref(&dir)).expect("discovery finds both files");
    assert_eq!(specs.len(), 2);

    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(!outcome.failed);
    assert!(outcome.listed.is_none());
    let scenarios = &outcome.report.scenarios;
    assert_eq!(scenarios.len(), 4);
    // Discovery order: sorted files, then in-file scenario order.
    assert!(
        scenarios[0].file.ends_with("login.spec.toml"),
        "{:?}",
        scenarios[0].file
    );
    assert_eq!(scenarios[0].name, "save the document");
    assert_eq!(scenarios[1].name, "email value");
    assert!(
        scenarios[2].file.ends_with("smoke.spec.toml"),
        "{:?}",
        scenarios[2].file
    );
    assert_eq!(scenarios[2].name, "ping");
    assert_eq!(scenarios[3].name, "clock state");
    for record in scenarios {
        assert_eq!(record.status, ScenarioStatus::Passed, "{record:?}");
        assert_eq!(record.attempts, 1);
        assert!(record.error.is_none(), "{record:?}");
        assert!(record.duration_ms < 5_000, "{record:?}");
    }
    assert_eq!(outcome.report.workers, 1);
    assert!(outcome.report.shard.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failing_scenario_reports_expectation_and_respects_spec_timeout() {
    let dir = leak_dir("runner-fail");
    write_spec(
        &dir,
        "fail.spec.toml",
        r#"
timeout_ms = 150

[[scenario]]
name = "missing button"
steps = ["expect role button Nope"]
"#,
    );
    let (path, _server) = spawn_server("runner-fail");

    let specs = runner::discover(&[dir]).expect("discovery");
    let started = std::time::Instant::now();
    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    let wall = started.elapsed();
    assert!(outcome.failed);
    let record = &outcome.report.scenarios[0];
    assert_eq!(record.status, ScenarioStatus::Failed);
    assert_eq!(record.attempts, 1);
    let error = record.error.as_deref().expect("failure message");
    assert!(error.contains("expectation failed"), "{error}");
    assert!(error.contains("Nope"), "{error}");
    // The spec's 150 ms deadline was honored (not the 5000 ms default).
    assert!(
        record.duration_ms < 2_000,
        "record {:?}, wall {wall:?}",
        record
    );
    assert!(wall.as_millis() < 4_000, "{wall:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_rerun_failed_scenarios_and_count_attempts() {
    let dir = leak_dir("runner-retries");
    write_spec(
        &dir,
        "flaky.spec.toml",
        r#"
retries = 2

[[scenario]]
name = "always missing"
timeout_ms = 100
steps = ["expect role button Nope"]
"#,
    );
    let (path, _server) = spawn_server("runner-retries");

    let specs = runner::discover(&[dir]).expect("discovery");
    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(outcome.failed);
    let record = &outcome.report.scenarios[0];
    assert_eq!(record.status, ScenarioStatus::Failed);
    // retries = 2 → three attempts.
    assert_eq!(record.attempts, 3, "{record:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn error_step_fails_the_scenario_with_the_repl_message() {
    let dir = leak_dir("runner-action-error");
    write_spec(
        &dir,
        "action.spec.toml",
        r#"
[[scenario]]
name = "click without a node"
steps = ["click"]
"#,
    );
    let (path, _server) = spawn_server("runner-action-error");

    let specs = runner::discover(&[dir]).expect("discovery");
    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(outcome.failed);
    let record = &outcome.report.scenarios[0];
    assert_eq!(record.status, ScenarioStatus::Failed);
    let error = record.error.as_deref().expect("failure message");
    assert!(error.contains("no node resolved"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_mode_needs_no_live_app_and_is_shard_aware() {
    let dir = leak_dir("runner-list");
    write_spec(&dir, "login.spec.toml", LOGIN_SPEC);
    write_spec(&dir, "smoke.spec.toml", SMOKE_SPEC);

    // No server spawned on purpose: list mode must not touch the wire.
    let specs = runner::discover(std::slice::from_ref(&dir)).expect("discovery");
    let opts = TestOptions {
        list: true,
        ..TestOptions::default()
    };
    let outcome = runner::run(specs, &opts, None).await.expect("list run");
    let listed = outcome.listed.expect("list mode returns ids");
    assert_eq!(listed.len(), 4);
    assert!(listed[0].ends_with(":: save the document"), "{listed:?}");
    assert!(outcome.report.scenarios.is_empty());
    assert!(!outcome.failed);

    // Shard 2/2 lists the interleaved second half, deterministically.
    let specs = runner::discover(&[dir]).expect("discovery");
    let opts = TestOptions {
        list: true,
        shard: Some((2, 2)),
        ..TestOptions::default()
    };
    let outcome = runner::run(specs, &opts, None).await.expect("list run");
    let listed = outcome.listed.expect("list mode returns ids");
    assert_eq!(listed.len(), 2);
    assert!(listed[0].ends_with(":: email value"), "{listed:?}");
    assert!(listed[1].ends_with(":: clock state"), "{listed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workers_run_scenarios_in_parallel() {
    let dir = leak_dir("runner-workers");
    write_spec(&dir, "login.spec.toml", LOGIN_SPEC);
    write_spec(&dir, "smoke.spec.toml", SMOKE_SPEC);
    let (path, _server) = spawn_server("runner-workers");

    let specs = runner::discover(&[dir]).expect("discovery");
    let opts = TestOptions {
        workers: 2,
        ..uds_opts(&path)
    };
    let RunOutcome { report, failed, .. } = runner::run(specs, &opts, None)
        .await
        .expect("run completes");
    assert!(!failed);
    assert_eq!(report.scenarios.len(), 4);
    assert!(
        report
            .scenarios
            .iter()
            .all(|r| r.status == ScenarioStatus::Passed)
    );
    assert_eq!(report.workers, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_target_overrides_the_spec_target() {
    let dir = leak_dir("runner-target-override");
    // The spec points at a socket that does not exist…
    write_spec(
        &dir,
        "wrong.spec.toml",
        r#"
target = "uds:///nonexistent-gw-runner-test.sock"

[[scenario]]
name = "ping"
steps = ["ping"]
"#,
    );
    // …but the CLI target wins, so the run is green.
    let (path, _server) = spawn_server("runner-target-override");
    let specs = runner::discover(&[dir]).expect("discovery");
    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(!outcome.failed, "{:?}", outcome.report.scenarios);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_without_any_target_is_a_config_error() {
    let dir = leak_dir("runner-no-target");
    write_spec(
        &dir,
        "x.spec.toml",
        r#"
[[scenario]]
name = "ping"
steps = ["ping"]
"#,
    );
    let specs = runner::discover(&[dir]).expect("discovery");
    let err = runner::run(specs, &TestOptions::default(), None)
        .await
        .expect_err("missing target is a config error");
    assert!(err.to_string().contains("target"), "{err}");
}

#[test]
fn strict_spec_errors_surface_as_config_errors() {
    let dir = leak_dir("runner-strict");
    write_spec(
        &dir,
        "dup.spec.toml",
        r#"
[[scenario]]
name = "same"
steps = ["ping"]

[[scenario]]
name = "same"
steps = ["ping"]
"#,
    );
    let err = runner::discover(&[dir]).expect_err("duplicate scenario rejected");
    assert!(matches!(err, RunnerError::Config(_)), "{err:?}");
    assert!(err.to_string().contains("dup.spec.toml"), "{err}");
    assert!(err.to_string().contains("`same`"), "{err}");
}

#[test]
fn discovery_scans_directories_recursively_and_ignores_other_toml() {
    let root = leak_dir("runner-discovery");
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("mkdir");
    write_spec(&root, "a.spec.toml", SMOKE_SPEC);
    write_spec(&nested, "b.spec.toml", SMOKE_SPEC);
    write_spec(&root, "not-a-spec.toml", "x = 1");

    let specs = runner::discover(std::slice::from_ref(&root)).expect("discovery");
    let names: Vec<String> = specs.iter().map(|s| s.display.clone()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    // Deterministic order.
    assert!(names[0].ends_with("a.spec.toml"), "{names:?}");
    assert!(names[1].ends_with("b.spec.toml"), "{names:?}");

    // No specs at all → config error naming the root.
    let empty = leak_dir("runner-discovery-empty");
    let err = runner::discover(std::slice::from_ref(&empty)).expect_err("empty discovery");
    assert!(err.to_string().contains("no *.spec.toml"), "{err}");
    assert!(
        err.to_string().contains(empty.to_string_lossy().as_ref()),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn report_files_are_written_for_json_junit_and_html() {
    let dir = leak_dir("runner-reports");
    write_spec(
        &dir,
        "mixed.spec.toml",
        r#"
timeout_ms = 150

[[scenario]]
name = "green"
steps = ["ping"]

[[scenario]]
name = "red"
steps = ["expect role button Nope"]
"#,
    );
    let (path, _server) = spawn_server("runner-reports");
    let out_dir = leak_dir("runner-reports-out");

    let specs = runner::discover(&[dir]).expect("discovery");
    let opts = TestOptions {
        reports: vec![ReportKind::Json, ReportKind::Junit, ReportKind::Html],
        out: Some(out_dir.clone()),
        ..uds_opts(&path)
    };
    let outcome = runner::run(specs, &opts, None)
        .await
        .expect("run completes");
    assert!(outcome.failed);

    let written =
        runner::write_reports(&outcome.report, &opts.reports, &out_dir).expect("reports write");
    assert_eq!(written.len(), 3);
    let junit = std::fs::read_to_string(out_dir.join("junit.xml")).expect("junit file");
    assert!(junit.contains("tests=\"2\""), "{junit}");
    assert!(junit.contains("failures=\"1\""), "{junit}");
    assert!(junit.contains("expectation failed"), "{junit}");
    let json = std::fs::read_to_string(out_dir.join("report.json")).expect("json file");
    assert!(json.contains("\"schema\": \"gwrun/v1\""), "{json}");
    let html = std::fs::read_to_string(out_dir.join("report.html")).expect("html file");
    assert!(html.contains("guiwright run report"), "{html}");
}

/// M6 (§16 v0.4 multi-window): a spec step re-aims its connection at a
/// window target — the REPL command language works verbatim in specs (the
/// M5 promise), so `window use` + scoped locators are declarative too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_steps_run_in_specs() {
    use guiwright_core::test_util::MultiWindowFixture;

    let fx = MultiWindowFixture::new();
    let scripted = fx.clone();
    let dir = leak_dir("runner-window");
    write_spec(
        &dir,
        "unscoped.spec.toml",
        r#"
[[scenario]]
name = "unscoped strict violation"
steps = ["get role button Save"]
"#,
    );
    write_spec(
        &dir,
        "editor.spec.toml",
        &format!(
            r#"
[[scenario]]
name = "editor window targeting"
steps = [
    "window use {}",
    "get role button Save",
    "expect role label \"Preview text\"",        # only in the preview window
]
"#,
            fx.w1.0
        ),
    );
    write_spec(
        &dir,
        "preview.spec.toml",
        &format!(
            r#"
[[scenario]]
name = "preview window targeting"
steps = [
    "window use {}",
    "get role button Save",
]
"#,
            fx.w2.0
        ),
    );

    let server = GwpServerHandle::spawn_with_seams(
        dir.join("window.sock"),
        move || MockAdapter::with_multi_window(&scripted),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("server binds");

    let specs = runner::discover(std::slice::from_ref(&dir)).expect("discovery");
    assert_eq!(specs.len(), 3);

    let outcome = runner::run(specs, &uds_opts(&dir.join("window.sock")), None)
        .await
        .expect("run completes");
    let status_of = |name: &str| {
        outcome
            .report
            .scenarios
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("scenario {name} missing"))
            .status
    };
    use ScenarioStatus::*;
    // Unscoped, the two same-named Saves stay a strict-mode violation.
    assert_eq!(status_of("unscoped strict violation"), Failed);
    // Scoped to the editor: its Save resolves; the preview-only label
    // stays invisible (the scope hides other windows from every selector).
    assert_eq!(status_of("editor window targeting"), Failed);
    // Scoped to the preview: its Save resolves inside the target.
    assert_eq!(status_of("preview window targeting"), Passed);

    server.shutdown();
}

/// M7 (§9): the gesture vocabulary works verbatim as `gwtest/v1` steps —
/// the runner drives the same `Repl::step` core, so raw input needs no
/// spec-format change (the M5 design).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gesture_steps_run_in_specs() {
    let (path, server) = spawn_server("runner-gestures");
    let dir = leak_dir("runner-gestures-specs");
    write_spec(
        &dir,
        "gestures.spec.toml",
        r#"
[[scenario]]
name = "raw input vocabulary"
steps = [
    "mouse move 5 6",
    "mouse click 10 20",
    "mouse dblclick 4 8",
    "mouse drag 0 0 100 0",
    "mouse wheel 0 -120",
    "key Enter",
    "text héllo",
]
"#,
    );

    let specs = runner::discover(std::slice::from_ref(&dir)).expect("discovery");
    assert_eq!(specs.len(), 1);
    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(!outcome.failed, "{:?}", outcome.report.scenarios);
    let scenarios = &outcome.report.scenarios;
    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0].name, "raw input vocabulary");
    assert_eq!(scenarios[0].status, ScenarioStatus::Passed, "{scenarios:?}");
    assert_eq!(scenarios[0].attempts, 1);

    server.shutdown();
}
