//! `guiwright test` — the own runner (M5, architecture §16 v0.5 "Test
//! runner, fixtures, parallel/shard, retries, reports" + §5.2 "nextest/
//! report glue, sharding").
//!
//! Discovers `gwtest/v1` specs ([`crate::spec`]), resolves per-scenario
//! deadlines/retries (CLI > scenario > file > default), splits them
//! deterministically for `--shard k/n` (§11 "multiple apps per host"),
//! and runs them over GWP — one fresh connection per attempt, every step
//! fed through the same [`crate::repl::Repl`] core the interactive
//! debugger uses (no protocol change, the M2/M4 conclusion).
//!
//! Parallelism is honest: one driver session serializes all connections,
//! so `--workers N` spreads scenarios across N tasks that each own their
//! connection per attempt; N apps per host (one `--uds`/`--tcp` per CI
//! shard) scale linearly. Isolation between scenarios that mutate app
//! state is the §14 reset-seam story: a `mock reset <scope>` step.
//!
//! With `--trace`, the runner also owns per-scenario trace emission (the
//! m5 §4 deferral, closed): the M18 live feed rides a second connection
//! per attempt — subscribed, so the server pushes tree/frame events to
//! it while the attempt's steps run on the first — and the last
//! attempt's capture finalizes as the M23 scrubber HTML under the report
//! directory, its file name riding the record's artifacts beside the m82
//! `shot:`/`video:` step-written paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use guiwright_viewer::LiveFeed;

use crate::report::{
    ReportKind, RunReport, ScenarioRecord, ScenarioStatus, render_html, render_json, render_junit,
};
use crate::session::{ConnectTarget, Connection};
use crate::spec::{Spec, SpecError};

/// The default spec directory when `guiwright test` gets no paths (§11's
/// in-repo convention).
pub const DEFAULT_SPEC_DIR: &str = "tests/guiwright";

/// The default resolve/action deadline (the REPL's default, §8.3).
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunnerError {
    /// Configuration/discovery problem → exit code 2.
    #[error("{0}")]
    Config(String),
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// One discovered spec file.
#[derive(Clone, Debug)]
pub struct DiscoveredSpec {
    /// Path as given to the discovery scan.
    pub path: PathBuf,
    /// Display form (the report's `file` field).
    pub display: String,
    pub spec: Spec,
}

/// Runner options (the `guiwright test` flag surface).
#[derive(Clone, Debug, Default)]
pub struct TestOptions {
    /// CLI target override — beats every spec file's `target`. `None`
    /// requires every spec to carry a target.
    pub target: Option<ConnectTarget>,
    /// Parallel scenario workers (default 1).
    pub workers: usize,
    /// Global retry override (beats scenario/spec values).
    pub retries: Option<u32>,
    /// Global step deadline override in ms (beats scenario/spec values).
    pub timeout_ms: Option<u64>,
    /// `--shard k/n` (1-indexed): run scenarios where `index % n == k-1`
    /// over the deterministically sorted discovery list.
    pub shard: Option<(u32, u32)>,
    /// Print the discovered scenario ids and exit — no wire traffic.
    pub list: bool,
    /// Suppress per-scenario progress lines.
    pub quiet: bool,
    /// File reports to write.
    pub reports: Vec<crate::report::ReportKind>,
    /// Report output directory (default [`DEFAULT_REPORT_DIR`]).
    pub out: Option<PathBuf>,
    /// Record a per-scenario trace and deep-link it from the reports
    /// (the m5 §4 deferral, closed — "clickable links wait for the
    /// runner to own trace emission"): the M18 live feed rides a second
    /// connection per attempt, the last attempt finalizes as the M23
    /// scrubber HTML under the report directory, and the file name rides
    /// the record's artifacts beside the m82 `shot:`/`video:` paths.
    pub trace: bool,
}

/// The default report (and trace) directory.
pub const DEFAULT_REPORT_DIR: &str = "guiwright-report";

/// The per-step trace pump window: one short pump per step boundary
/// collects what the step produced on the feed's own connection — the
/// event pushes ride the server's per-connection channel, so a step's
/// tree changes are queued by the time the step's reply came back.
const TRACE_PUMP_WINDOW: Duration = Duration::from_millis(25);

/// The live-progress callback (`run`'s `on_finish`): every finished
/// scenario record, in completion order.
pub type OnFinish = std::sync::Arc<dyn Fn(&ScenarioRecord) + Send + Sync>;

/// The result of one [`run`].
#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub report: RunReport,
    /// True when any scenario ended [`ScenarioStatus::Failed`] (exit 1).
    pub failed: bool,
    /// `--list` mode: the scenario ids (`<file> :: <name>`), no scenarios
    /// run, no target needed.
    pub listed: Option<Vec<String>>,
}

/// Discover spec files: explicit files pass through, directories are
/// scanned recursively for `*.spec.toml`. The result is sorted by display
/// path so sharding and report order are deterministic.
pub fn discover(paths: &[PathBuf]) -> Result<Vec<DiscoveredSpec>, RunnerError> {
    let mut files: Vec<PathBuf> = Vec::new();
    for path in paths {
        let meta = std::fs::metadata(path).map_err(|source| RunnerError::Io {
            path: path.clone(),
            source,
        })?;
        if meta.is_dir() {
            scan_dir(path, &mut files)?;
        } else {
            files.push(path.clone());
        }
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(RunnerError::Config(format!(
            "no *.spec.toml specs found under {} — pass spec files or directories, \
             or drop specs into the {DEFAULT_SPEC_DIR}/ convention",
            paths
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let mut specs = Vec::with_capacity(files.len());
    for path in &files {
        let display = path.to_string_lossy().into_owned();
        let text = std::fs::read_to_string(path).map_err(|source| RunnerError::Io {
            path: path.clone(),
            source,
        })?;
        let spec = Spec::parse(&display, &text).map_err(|e| spec_error(&e))?;
        specs.push(DiscoveredSpec {
            path: path.clone(),
            display,
            spec,
        });
    }
    Ok(specs)
}

fn scan_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), RunnerError> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(|source| RunnerError::Io {
            path: dir.to_path_buf(),
            source,
        })?
        .collect::<Result<_, _>>()
        .map_err(|source| RunnerError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, out)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(crate::spec::SPEC_SUFFIX))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn spec_error(err: &SpecError) -> RunnerError {
    RunnerError::Config(err.to_string())
}

/// One resolved scenario: everything an attempt needs, decided up front
/// (CLI > scenario > file > default).
#[derive(Clone, Debug)]
struct Case {
    file: String,
    name: String,
    steps: Vec<String>,
    timeout_ms: u64,
    retries: u32,
}

fn resolve_cases(specs: &[DiscoveredSpec], opts: &TestOptions) -> Vec<Case> {
    let mut cases = Vec::new();
    for discovered in specs {
        for scenario in &discovered.spec.scenarios {
            let timeout_ms = opts
                .timeout_ms
                .or(scenario.timeout_ms)
                .or(discovered.spec.timeout_ms)
                .unwrap_or(DEFAULT_TIMEOUT_MS);
            let retries = opts
                .retries
                .or(scenario.retries)
                .or(discovered.spec.retries)
                .unwrap_or(0);
            cases.push(Case {
                file: discovered.display.clone(),
                name: scenario.name.clone(),
                steps: scenario.steps.clone(),
                timeout_ms,
                retries,
            });
        }
    }
    cases
}

fn shard_keeps(index: usize, shard: (u32, u32)) -> bool {
    let (k, n) = shard;
    (index as u64 % u64::from(n)) == (u64::from(k) - 1)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// Run the discovered specs. `on_finish` (when given) receives every
/// scenario record as it completes, in completion order — the live
/// progress lines. The report itself is always in discovery order.
pub async fn run(
    specs: Vec<DiscoveredSpec>,
    opts: &TestOptions,
    on_finish: Option<OnFinish>,
) -> Result<RunOutcome, RunnerError> {
    let mut cases = resolve_cases(&specs, opts);
    if let Some(shard) = opts.shard {
        cases = cases
            .into_iter()
            .enumerate()
            .filter(|(i, _)| shard_keeps(*i, shard))
            .map(|(_, case)| case)
            .collect();
    }
    if opts.list {
        let listed = cases
            .iter()
            .map(|case| format!("{} :: {}", case.file, case.name))
            .collect();
        return Ok(RunOutcome {
            report: RunReport {
                started_ms: now_ms(),
                duration_ms: 0,
                workers: opts.workers.max(1),
                shard: opts.shard,
                scenarios: Vec::new(),
            },
            failed: false,
            listed: Some(listed),
        });
    }
    if cases.is_empty() {
        return Err(RunnerError::Config(
            "no scenarios to run — the shard filter matched nothing".into(),
        ));
    }

    let target = opts.target.as_ref().ok_or_else(|| {
        RunnerError::Config(
            "no GWP target — pass --uds <path> / --tcp <addr> or set `target` in the specs".into(),
        )
    })?;

    let started_ms = now_ms();
    let started = Instant::now();
    let workers = opts.workers.max(1);
    let queue = Arc::new(AtomicUsize::new(0));
    let slots: Arc<Vec<Mutex<Option<ScenarioRecord>>>> =
        Arc::new((0..cases.len()).map(|_| Mutex::new(None)).collect());
    let trace_paths = trace_paths_for(opts, &cases);
    let cases = Arc::new(cases);
    let target = target.clone();

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let cases = Arc::clone(&cases);
        let slots = Arc::clone(&slots);
        let queue = Arc::clone(&queue);
        let on_finish = on_finish.clone();
        let target = target.clone();
        let trace_paths = trace_paths.clone();
        handles.push(tokio::spawn(async move {
            loop {
                let index = queue.fetch_add(1, Ordering::SeqCst);
                if index >= cases.len() {
                    break;
                }
                let case = &cases[index];
                let trace_path = trace_paths.as_ref().map(|paths| paths[index].as_path());
                let record = run_case(&target, case, trace_path).await;
                if let Some(on_finish) = &on_finish {
                    on_finish(&record);
                }
                *slots[index].lock().expect("scenario slot lock") = Some(record);
            }
        }));
    }
    for handle in handles {
        handle.await.expect("worker task does not panic");
    }

    let mut scenarios = Vec::with_capacity(cases.len());
    for slot in slots.iter() {
        let record = slot
            .lock()
            .expect("scenario slot lock")
            .take()
            .expect("every scenario produced a record");
        scenarios.push(record);
    }
    let failed = scenarios.iter().any(|r| r.status == ScenarioStatus::Failed);
    let report = RunReport {
        started_ms,
        duration_ms: started.elapsed().as_millis() as u64,
        workers,
        shard: opts.shard,
        scenarios,
    };
    Ok(RunOutcome {
        report,
        failed,
        listed: None,
    })
}

/// One scenario with its retry loop. `trace_path` (when `--trace` is on)
/// is the scenario's trace file; the LAST attempt's capture is the one
/// that persists — the attempt the record's status reflects.
async fn run_case(
    target: &ConnectTarget,
    case: &Case,
    trace_path: Option<&Path>,
) -> ScenarioRecord {
    let case_started = Instant::now();
    let (attempts, error, artifacts) =
        run_with_retries(case.retries, || run_attempt(target, case, trace_path)).await;
    let status = match (&error, attempts) {
        (None, 1) => ScenarioStatus::Passed,
        (None, _) => ScenarioStatus::Flaky,
        (Some(_), _) => ScenarioStatus::Failed,
    };
    ScenarioRecord {
        file: case.file.clone(),
        name: case.name.clone(),
        status,
        attempts,
        duration_ms: case_started.elapsed().as_millis() as u64,
        error,
        artifacts,
    }
}

/// The retry loop, generic over the attempt future so the flaky/pass
/// classification is unit-testable without a live session. Artifacts
/// accumulate across every attempt, failures included (a scenario that
/// screenshotted before its failing step still shows the file).
async fn run_with_retries<F, Fut>(
    retries: u32,
    mut attempt: F,
) -> (u32, Option<String>, Vec<String>)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = (Result<(), String>, Vec<String>)>,
{
    let total = retries + 1;
    let mut last_error = None;
    let mut artifacts = Vec::new();
    for attempt_no in 1..=total {
        let (outcome, mut found) = attempt().await;
        artifacts.append(&mut found);
        match outcome {
            Ok(()) => return (attempt_no, None, artifacts),
            Err(err) => last_error = Some(err),
        }
    }
    (total, last_error, artifacts)
}

/// One attempt: connect → handshake → steps → bye. The first error line
/// fails the attempt; the remaining steps do not run. Artifact paths
/// written by steps before the failure still ride home (`shot:` /
/// `video:` Ok lines, m82) — the attempt result pairs the outcome with
/// whatever the steps emitted.
///
/// With a `trace_path`, the attempt attaches the M18 live feed on a
/// second connection (subscribe + initial dump), pumps one short window
/// per step boundary, and finalizes the scrubber HTML after the attempt
/// — success or failure; a failing scenario's trace is the valuable one.
/// An attach failure degrades to no trace (the scenario's own steps
/// still run; the artifacts list then carries no trace link) — the
/// runner never fails a scenario for its sidecar's sake.
async fn run_attempt(
    target: &ConnectTarget,
    case: &Case,
    trace_path: Option<&Path>,
) -> (Result<(), String>, Vec<String>) {
    let mut feed = match trace_path {
        Some(_) => attach_trace_feed(target, case).await.ok(),
        None => None,
    };
    let mut artifacts = Vec::new();
    let mut repl = match crate::repl::Repl::connect(target).await {
        Ok(repl) => repl,
        Err(err) => {
            finalize_trace(&mut feed, trace_path, &mut artifacts).await;
            return (
                Err(format!("cannot connect to {target:?}: {err}")),
                artifacts,
            );
        }
    };
    repl.set_timeout_ms(case.timeout_ms);
    for step in &case.steps {
        let outcome = repl.step(step).await;
        if let Some(feed) = feed.as_mut() {
            let _ = feed.pump(TRACE_PUMP_WINDOW).await;
        }
        for line in &outcome.lines {
            match line {
                crate::repl::Line::Ok(text) => {
                    if let Some(path) = artifact_from_line(text) {
                        artifacts.push(path);
                    }
                }
                crate::repl::Line::Err(message) => {
                    finalize_trace(&mut feed, trace_path, &mut artifacts).await;
                    return (Err(message.clone()), artifacts);
                }
                crate::repl::Line::Info(_) | crate::repl::Line::Warn(_) => {}
            }
        }
    }
    // The runner owns the session lifecycle: close cleanly. A failed Bye
    // races the disconnect — the session ends either way.
    let _ = repl.step("bye").await;
    finalize_trace(&mut feed, trace_path, &mut artifacts).await;
    (Ok(()), artifacts)
}

/// Attach the M18 live feed on a second connection for the attempt's
/// trace capture: handshake as the runner's trace agent, subscribe
/// (tree + frame events), initial dump, and the scenario id as the
/// emitter's test label (`<file> :: <name>`, the report's own id).
async fn attach_trace_feed(
    target: &ConnectTarget,
    case: &Case,
) -> Result<LiveFeed<Connection>, guiwright_viewer::ViewerError> {
    let connection = Connection::connect(target).await?;
    let mut feed = LiveFeed::new(connection);
    feed.attach(&format!("{} (trace)", crate::AGENT)).await?;
    feed.set_test(format!("{} :: {}", case.file, case.name));
    Ok(feed)
}

/// Finalize the attempt's trace into the M23 scrubber HTML (stamped
/// `gwtrace/v1 (live)`, the scenario id in the header). The file name —
/// the report links artifacts by it, and the trace lives beside the
/// reports — rides home in `artifacts` when the write succeeded.
async fn finalize_trace(
    feed: &mut Option<LiveFeed<Connection>>,
    trace_path: Option<&Path>,
    artifacts: &mut Vec<String>,
) {
    let (Some(feed), Some(path)) = (feed.as_mut(), trace_path) else {
        return;
    };
    let html = feed.render_html();
    let _ = feed.bye().await;
    if let Some(dir) = path.parent() {
        if let Err(_sidecar) = std::fs::create_dir_all(dir) {
            return;
        }
    }
    if let Err(_sidecar) = std::fs::write(path, html) {
        return;
    }
    // The report links artifacts by the recorded string; the trace file
    // lives beside the reports, so its file name is the honest link.
    let name = path.file_name().and_then(|name| name.to_str()).map_or_else(
        || path.display().to_string(),
        std::string::ToString::to_string,
    );
    artifacts.push(name);
}

/// The run's per-scenario trace paths (`--trace`): the report directory
/// plus one sanitized file per case. The files are named up front —
/// discovery order, deterministic even under sharding — so parallel
/// workers never race a name.
fn trace_paths_for(opts: &TestOptions, cases: &[Case]) -> Option<Vec<PathBuf>> {
    if !opts.trace {
        return None;
    }
    let dir = opts
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_REPORT_DIR));
    let mut used = std::collections::HashSet::new();
    Some(
        cases
            .iter()
            .map(|case| trace_file_path(&dir, case, &mut used))
            .collect(),
    )
}

/// The per-scenario trace file: `<spec-stem>.<scenario>.trace.html`
/// under the report directory. The spec stem drops the `.spec.toml`
/// suffix (the file name is `login.spec.toml`; the stem is `login`).
/// Spec paths and scenario names are user data, so the
/// report-unfriendly characters collapse (consecutive separators
/// included) and same-name scenarios disambiguate with a `-2`-style
/// run-order suffix.
fn trace_file_path(
    dir: &Path,
    case: &Case,
    used: &mut std::collections::HashSet<String>,
) -> PathBuf {
    let file_name = Path::new(&case.file)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("spec");
    let spec_stem = file_name
        .strip_suffix(crate::spec::SPEC_SUFFIX)
        .unwrap_or_else(|| {
            Path::new(file_name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("spec")
        });
    let base = format!(
        "{}.{}",
        sanitize_trace_part(spec_stem),
        sanitize_trace_part(&case.name)
    );
    let mut file = format!("{base}.trace.html");
    let mut suffix = 1_u32;
    while !used.insert(file.clone()) {
        suffix += 1;
        file = format!("{base}-{suffix}.trace.html");
    }
    dir.join(file)
}

/// Report-filename-safe form of one trace-name part: alphanumeric,
/// `_` and `.` survive; every other character (path separators and
/// drive prefixes included) collapses to `-`, runs included.
fn sanitize_trace_part(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    let mut last_separator = false;
    for ch in part.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            out.push(ch);
            last_separator = false;
        } else if !last_separator {
            out.push('-');
            last_separator = true;
        }
    }
    out.trim_matches('-').to_owned()
}

/// The artifact path an artifact-emitting Ok line reports (§10 capture):
/// `screenshot` answers `shot: <path> (<w>×<h>, frame <n>)` and `video
/// stop` answers `video: <path> (<n> frames, …)`. Non-file lines of the
/// same family (`video: recording started`) carry no ` (` trailer and
/// harvest nothing — the runner only links paths its steps actually wrote.
pub fn artifact_from_line(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix("shot: ")
        .or_else(|| text.strip_prefix("video: "))?;
    let end = rest.find(" (")?;
    let path = rest[..end].trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// Write the file reports (`--report json|junit|html`) under `out_dir`.
/// `list` writes nothing. Returns the written paths in kind order.
pub fn write_reports(
    report: &RunReport,
    kinds: &[crate::report::ReportKind],
    out_dir: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for kind in kinds {
        if !seen.insert(*kind) {
            continue;
        }
        let path = match kind {
            ReportKind::List => continue,
            ReportKind::Json | ReportKind::Junit | ReportKind::Html => {
                out_dir.join(kind.file_name())
            }
        };
        std::fs::create_dir_all(out_dir)?;
        let body = match kind {
            ReportKind::List => continue,
            ReportKind::Json => render_json(report),
            ReportKind::Junit => render_junit(report),
            ReportKind::Html => render_html(report),
        };
        std::fs::write(&path, body)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_filter_interleaves_deterministically() {
        // 6 cases, 3 shards, 1-indexed: k takes i where i % 3 == k-1.
        let kept: Vec<usize> = (0..6).filter(|i| shard_keeps(*i, (1, 3))).collect();
        assert_eq!(kept, vec![0, 3]);
        let kept: Vec<usize> = (0..6).filter(|i| shard_keeps(*i, (2, 3))).collect();
        assert_eq!(kept, vec![1, 4]);
        let kept: Vec<usize> = (0..6).filter(|i| shard_keeps(*i, (3, 3))).collect();
        assert_eq!(kept, vec![2, 5]);
        // Two shards partition.
        let a: Vec<usize> = (0..5).filter(|i| shard_keeps(*i, (1, 2))).collect();
        let b: Vec<usize> = (0..5).filter(|i| shard_keeps(*i, (2, 2))).collect();
        assert_eq!(a, vec![0, 2, 4]);
        assert_eq!(b, vec![1, 3]);
        // Single shard keeps everything.
        let all: Vec<usize> = (0..4).filter(|i| shard_keeps(*i, (1, 1))).collect();
        assert_eq!(all, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn retry_loop_classifies_pass_flaky_and_fail() {
        // Pass first try.
        let mut calls = 0;
        let (attempts, error, artifacts) = run_with_retries(2, || {
            calls += 1;
            async move {
                assert_eq!(calls, 1);
                (Ok::<(), String>(()), Vec::new())
            }
        })
        .await;
        assert_eq!((attempts, error.as_deref()), (1, None));
        assert!(
            artifacts.is_empty(),
            "the green first try wrote: {artifacts:?}"
        );

        // Flaky: fails once, passes on the retry.
        let mut calls = 0;
        let (attempts, error, artifacts) = run_with_retries(2, || {
            calls += 1;
            let first = calls == 1;
            async move {
                if first {
                    (Err("transient".into()), vec!["before.png".into()])
                } else {
                    (Ok(()), vec!["after.png".into()])
                }
            }
        })
        .await;
        assert_eq!(attempts, 2);
        assert_eq!(error, None);
        // Artifacts from both attempts ride home, attempt order preserved.
        assert_eq!(artifacts, vec!["before.png", "after.png"]);

        // Failing: every attempt runs, the last error surfaces.
        let mut calls = 0;
        let (attempts, error, artifacts) = run_with_retries(2, || {
            calls += 1;
            async move { (Err(format!("boom {calls}")), Vec::new()) }
        })
        .await;
        assert_eq!(attempts, 3);
        assert_eq!(error.as_deref(), Some("boom 3"));
        assert!(
            artifacts.is_empty(),
            "the failed attempts wrote: {artifacts:?}"
        );

        // Zero retries runs exactly one attempt.
        let (attempts, error, _) = run_with_retries(0, || async move {
            (Err::<(), String>("nope".into()), Vec::new())
        })
        .await;
        assert_eq!(attempts, 1);
        assert_eq!(error.as_deref(), Some("nope"));
    }

    #[test]
    fn artifact_from_line_reads_the_capture_shapes() {
        // `screenshot` (§10): the shot line carries the written path.
        assert_eq!(
            artifact_from_line("shot: /tmp/shots/window.png (320×240, frame 3)").as_deref(),
            Some("/tmp/shots/window.png")
        );
        // `video stop` (§10): the file line carries the written path.
        assert_eq!(
            artifact_from_line("video: /tmp/clips/run.webm (30 frames, 0 dropped, 1200ms)")
                .as_deref(),
            Some("/tmp/clips/run.webm")
        );
        // Same-family status lines name no file.
        assert_eq!(artifact_from_line("video: recording started"), None);
        assert_eq!(artifact_from_line("pong"), None);
        assert_eq!(artifact_from_line("timeout = 5 ms"), None);
        // Degenerate shapes harvest nothing.
        assert_eq!(artifact_from_line("shot: "), None);
        assert_eq!(artifact_from_line("video:  (no path)"), None);
    }

    #[test]
    fn trace_file_paths_sanitize_and_disambiguate() {
        // A plain spec + scenario: stem and name, separated, under the
        // report directory.
        let mut used = std::collections::HashSet::new();
        let dir = Path::new(DEFAULT_REPORT_DIR);
        let case = Case {
            file: "specs/login.spec.toml".into(),
            name: "happy path".into(),
            steps: Vec::new(),
            timeout_ms: 1,
            retries: 0,
        };
        let path = trace_file_path(dir, &case, &mut used);
        assert_eq!(
            path,
            Path::new("guiwright-report/login.happy-path.trace.html")
        );

        // User data collapses: separators, spaces, unicode, and drive
        // prefixes never reach a report file name.
        let weird = Case {
            file: "C:\\ci\\specs\\a&b.spec.toml".into(),
            name: "шаг / второй".into(),
            steps: Vec::new(),
            timeout_ms: 1,
            retries: 0,
        };
        let path = trace_file_path(dir, &weird, &mut used);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("ascii name");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "report-unfriendly characters collapse: {name}"
        );
        assert!(name.ends_with(".trace.html"));

        // Same-name scenarios in one run disambiguate deterministically.
        let mut used = std::collections::HashSet::new();
        let first = trace_file_path(dir, &case, &mut used);
        let second = trace_file_path(dir, &case, &mut used);
        assert_ne!(first, second);
        assert_eq!(
            second.file_name().and_then(|n| n.to_str()),
            Some("login.happy-path-2.trace.html")
        );
    }

    #[test]
    fn resolve_cases_applies_the_precedence_chain() {
        let spec_with_defaults: Spec = Spec::parse(
            "a.spec.toml",
            r#"
target = "uds:///tmp/a.sock"
timeout_ms = 1000
retries = 1

[[scenario]]
name = "plain"
steps = ["ping"]

[[scenario]]
name = "overridden"
timeout_ms = 2000
retries = 2
steps = ["ping"]
"#,
        )
        .expect("spec parses");
        let discovered = vec![DiscoveredSpec {
            path: PathBuf::from("a.spec.toml"),
            display: "a.spec.toml".into(),
            spec: spec_with_defaults,
        }];

        // No CLI overrides: file values apply.
        let cases = resolve_cases(&discovered, &TestOptions::default());
        assert_eq!(cases[0].timeout_ms, 1000);
        assert_eq!(cases[0].retries, 1);
        assert_eq!(cases[1].timeout_ms, 2000);
        assert_eq!(cases[1].retries, 2);

        // CLI overrides beat everything.
        let cases = resolve_cases(
            &discovered,
            &TestOptions {
                timeout_ms: Some(300),
                retries: Some(7),
                ..TestOptions::default()
            },
        );
        assert_eq!(cases[0].timeout_ms, 300);
        assert_eq!(cases[0].retries, 7);
        assert_eq!(cases[1].timeout_ms, 300);
        assert_eq!(cases[1].retries, 7);

        // Bare spec: defaults apply.
        let bare = Spec::parse(
            "b.spec.toml",
            r#"
[[scenario]]
name = "plain"
steps = ["ping"]
"#,
        )
        .expect("spec parses");
        let cases = resolve_cases(
            &[DiscoveredSpec {
                path: PathBuf::from("b.spec.toml"),
                display: "b.spec.toml".into(),
                spec: bare,
            }],
            &TestOptions::default(),
        );
        assert_eq!(cases[0].timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(cases[0].retries, 0);
    }

    #[test]
    fn write_reports_dedupes_and_skips_list() {
        let report = RunReport {
            started_ms: 0,
            duration_ms: 5,
            workers: 1,
            shard: None,
            scenarios: vec![ScenarioRecord {
                file: "a.spec.toml".into(),
                name: "s".into(),
                status: ScenarioStatus::Passed,
                attempts: 1,
                duration_ms: 5,
                error: None,
                artifacts: Vec::new(),
            }],
        };
        let dir = tempfile::tempdir().expect("tmpdir");
        let written = write_reports(
            &report,
            &[
                crate::report::ReportKind::Json,
                crate::report::ReportKind::Json,
                crate::report::ReportKind::List,
                crate::report::ReportKind::Junit,
            ],
            dir.path(),
        )
        .expect("writes");
        assert_eq!(written.len(), 2);
        assert!(dir.path().join("report.json").exists());
        assert!(dir.path().join("junit.xml").exists());
        assert!(!dir.path().join("report.html").exists());
    }
}
