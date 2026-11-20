//! The CLI runner end-to-end over a **real** adapter (architecture §1
//! "Test runner integration", §16 v0.5): the `gwtest/v1` suite rides
//! `guiwright_cli::runner` against the real egui counter+form app —
//! the §4.1 embedded adapter, not the `MockAdapter` stand-in the
//! `runner_e2e` suite drives.
//!
//! Covered: the runner spawns the app (the §5.3 launch contract — the
//! app boots behind a `GwpServerHandle`, the runner attaches), one
//! fresh connection per attempt (the §14 isolation seam), the real
//! command vocabulary (`get role …` → `click` / `fill`, the m54 content
//! assertion `assert text … is …` — egui renders each label as a
//! `Label`/`TextRun` twin pair whose tree order rides content hashes,
//! so the assertion's locator pins the label's node family and the
//! expectation waits for the real content), **real app state carried
//! across connections** (the
//! second scenario's count continues the first's), and the same suite
//! green against a second, freshly booted app (per-run app isolation —
//! the §11 "one app per shard" story). The `guiwright` binary itself
//! then runs the same suite as a process (exit codes + `gwrun/v1`
//! report) — the `cargo test` experience §1 promises.

use std::path::{Path, PathBuf};
use std::process::Command;

use guiwright_adapter_egui::EguiAdapter;
use guiwright_cli::ConnectTarget;
use guiwright_cli::report::ScenarioStatus;
use guiwright_cli::runner::{self, TestOptions};

/// The real counter+form app: stock egui widgets, state owned by the
/// app closure (nothing captured by the factory — the `Send` bound is
/// the factory's, the adapter stays on the session thread).
const COUNTER_FORM_SPEC: &str = r#"
timeout_ms = 5_000

[[scenario]]
name = "counter clicks through the real pipeline"
steps = [
    "get role button Increment",
    "click",
    "get role button Increment",
    "click",
    "assert text text \"Count\" is \"Count: 2\"",
]

[[scenario]]
name = "form fill carries state across connections"
steps = [
    "get role TextInput",
    "fill \"Ada\"",
    "assert text text \"Welcome\" is \"Welcome, Ada\"",
    "get role button Increment",
    "click",
    "assert text text \"Count\" is \"Count: 3\"",
]
"#;

fn counter_form_app() -> EguiAdapter {
    let mut count: i64 = 0;
    let mut name = String::new();
    EguiAdapter::new(move |ctx| {
        ctx.root_ui(|ui| {
            ui.label(format!("Count: {count}"));
            if ui.button("Increment").clicked() {
                count += 1;
            }
            if ui.button("Decrement").clicked() {
                count -= 1;
            }
            ui.add(egui::TextEdit::singleline(&mut name).id_salt("name"));
            ui.label(format!("Welcome, {name}"));
        });
    })
}

/// Boot the real app behind a GWP server (the §5.3 launch contract) on
/// a leaked temp path (the `runner_e2e` helper pattern).
fn spawn_app(tag: &str) -> (PathBuf, guiwright_driver::server::GwpServerHandle) {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    let path = dir.path().join(format!("{tag}.sock"));
    let handle = guiwright_driver::server::GwpServerHandle::spawn_with_seams(
        path.clone(),
        counter_form_app,
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("egui app binds");
    (path, handle)
}

fn write_suite(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).expect("mkdir");
    std::fs::write(dir.join("counter_form.spec.toml"), COUNTER_FORM_SPEC).expect("write spec");
    dir.to_path_buf()
}

fn uds_opts(path: &Path) -> TestOptions {
    TestOptions {
        target: Some(ConnectTarget::Uds(path.to_path_buf())),
        ..TestOptions::default()
    }
}

/// The runner drives the real egui app to green, and the second
/// scenario observes the app's **real** state across the fresh
/// connection the attempt opened (the count the first scenario's
/// clicks left behind).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_drives_the_real_egui_counter_form_to_green() {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")))
        .path()
        .to_path_buf();
    write_suite(&dir);
    let (path, _app) = spawn_app("e2e-egui");

    let specs = runner::discover(std::slice::from_ref(&dir)).expect("discovery");
    assert_eq!(specs.len(), 1, "{specs:?}");

    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(!outcome.failed, "{:?}", outcome.report.scenarios);
    let scenarios = &outcome.report.scenarios;
    assert_eq!(scenarios.len(), 2);
    assert_eq!(
        scenarios[0].name,
        "counter clicks through the real pipeline"
    );
    assert_eq!(
        scenarios[1].name,
        "form fill carries state across connections"
    );
    for record in scenarios {
        assert_eq!(record.status, ScenarioStatus::Passed, "{record:?}");
        assert_eq!(record.attempts, 1, "{record:?}");
        assert!(record.error.is_none(), "{record:?}");
        assert!(record.duration_ms < 5_000, "{record:?}");
    }
}

/// The same suite, a **freshly booted** app: green again with the same
/// expectations (the first scenario's `Count: 2` only holds when the
/// app boots isolated — the per-run app isolation §11 grants).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_suite_is_green_against_a_fresh_app() {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")))
        .path()
        .to_path_buf();
    write_suite(&dir);
    let (path, _app) = spawn_app("e2e-egui-fresh");

    let specs = runner::discover(std::slice::from_ref(&dir)).expect("discovery");
    let outcome = runner::run(specs, &uds_opts(&path), None)
        .await
        .expect("run completes");
    assert!(!outcome.failed, "{:?}", outcome.report.scenarios);
    assert!(
        outcome
            .report
            .scenarios
            .iter()
            .all(|r| r.status == ScenarioStatus::Passed && r.attempts == 1),
        "{:?}",
        outcome.report.scenarios
    );
}

/// The `guiwright` binary itself: `guiwright test` over the real app,
/// exit code 0 and the `gwrun/v1` report on disk — the §1 "cargo test"
/// experience, validated as a process (config → discovery → run →
/// exit code).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_guiwright_binary_runs_the_real_app_suite() {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")))
        .path()
        .to_path_buf();
    write_suite(&dir);
    let out = dir.join("guiwright-report");
    let (path, _app) = spawn_app("e2e-egui-bin");

    let status = Command::new(env!("CARGO_BIN_EXE_guiwright"))
        .args([
            "test",
            dir.to_str().expect("utf8 dir"),
            "--uds",
            path.to_str().expect("utf8 sock"),
            "--report",
            "json",
            "--out",
            out.to_str().expect("utf8 out"),
            "-q",
        ])
        .status()
        .expect("spawn guiwright binary");
    assert!(status.success(), "exit code {status:?} — the run is red");

    let report = std::fs::read_to_string(out.join("report.json")).expect("report written");
    assert!(report.contains("\"schema\": \"gwrun/v1\""), "{report}");
    assert!(
        report.contains("\"passed\": 2, \"flaky\": 0, \"failed\": 0"),
        "{report}"
    );
    assert!(
        report.contains("counter clicks through the real pipeline"),
        "{report}"
    );
    assert!(
        report.contains("form fill carries state across connections"),
        "{report}"
    );
    assert!(report.contains("\"error\": null"), "{report}");
}
