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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::report::{
    render_html, render_json, render_junit, ReportKind, RunReport, ScenarioRecord,
    ScenarioStatus,
};
use crate::session::ConnectTarget;
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
    /// Report output directory (default `guiwright-report/`).
    pub out: Option<PathBuf>,
}

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
        let spec = Spec::parse(&display, &text).map_err(spec_error)?;
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
    entries.sort_by_key(|e| e.file_name());
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

fn spec_error(err: SpecError) -> RunnerError {
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
    (index as u64 % n as u64) == (k as u64 - 1)
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
            "no GWP target — pass --uds <path> / --tcp <addr> or set `target` in the specs"
                .into(),
        )
    })?;

    let started_ms = now_ms();
    let started = Instant::now();
    let workers = opts.workers.max(1);
    let queue = Arc::new(AtomicUsize::new(0));
    let slots: Arc<Vec<Mutex<Option<ScenarioRecord>>>> =
        Arc::new((0..cases.len()).map(|_| Mutex::new(None)).collect());
    let cases = Arc::new(cases);
    let target = target.clone();

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let cases = Arc::clone(&cases);
        let slots = Arc::clone(&slots);
        let queue = Arc::clone(&queue);
        let on_finish = on_finish.clone();
        let target = target.clone();
        handles.push(tokio::spawn(async move {
            loop {
                let index = queue.fetch_add(1, Ordering::SeqCst);
                if index >= cases.len() {
                    break;
                }
                let case = &cases[index];
                let record = run_case(&target, case).await;
                if let Some(on_finish) = &on_finish {
                    on_finish(&record);
                }
                *slots[index]
                    .lock()
                    .expect("scenario slot lock") = Some(record);
            }
        }));
    }
    for handle in handles {
        handle
            .await
            .expect("worker task does not panic");
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
    let failed = scenarios
        .iter()
        .any(|r| r.status == ScenarioStatus::Failed);
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

/// One scenario with its retry loop.
async fn run_case(target: &ConnectTarget, case: &Case) -> ScenarioRecord {
    let case_started = Instant::now();
    let (attempts, error) =
        run_with_retries(case.retries, || run_attempt(target, case)).await;
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
    }
}

/// The retry loop, generic over the attempt future so the flaky/pass
/// classification is unit-testable without a live session.
async fn run_with_retries<F, Fut>(retries: u32, mut attempt: F) -> (u32, Option<String>)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let total = retries + 1;
    let mut last_error = None;
    for attempt_no in 1..=total {
        match attempt().await {
            Ok(()) => return (attempt_no, None),
            Err(err) => last_error = Some(err),
        }
    }
    (total, last_error)
}

/// One attempt: connect → handshake → steps → bye. The first error line
/// fails the attempt; the remaining steps do not run.
async fn run_attempt(target: &ConnectTarget, case: &Case) -> Result<(), String> {
    let mut repl = crate::repl::Repl::connect(target)
        .await
        .map_err(|err| format!("cannot connect to {target:?}: {err}"))?;
    repl.set_timeout_ms(case.timeout_ms);
    for step in &case.steps {
        let outcome = repl.step(step).await;
        for line in &outcome.lines {
            if let crate::repl::Line::Err(message) = line {
                return Err(message.clone());
            }
        }
    }
    // The runner owns the session lifecycle: close cleanly. A failed Bye
    // races the disconnect — the session ends either way.
    let _ = repl.step("bye").await;
    Ok(())
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
            ReportKind::Json => out_dir.join(kind.file_name()),
            ReportKind::Junit => out_dir.join(kind.file_name()),
            ReportKind::Html => out_dir.join(kind.file_name()),
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
        let (attempts, error) = run_with_retries(2, || {
            calls += 1;
            async move {
                assert_eq!(calls, 1);
                Ok::<(), String>(())
            }
        })
        .await;
        assert_eq!((attempts, error), (1, None));

        // Flaky: fails once, passes on the retry.
        let mut calls = 0;
        let (attempts, error) = run_with_retries(2, || {
            calls += 1;
            let first = calls == 1;
            async move {
                if first {
                    Err("transient".into())
                } else {
                    Ok(())
                }
            }
        })
        .await;
        assert_eq!(attempts, 2);
        assert_eq!(error, None);

        // Failing: every attempt runs, the last error surfaces.
        let mut calls = 0;
        let (attempts, error) = run_with_retries(2, || {
            calls += 1;
            async move { Err(format!("boom {calls}")) }
        })
        .await;
        assert_eq!(attempts, 3);
        assert_eq!(error.as_deref(), Some("boom 3"));

        // Zero retries runs exactly one attempt.
        let mut calls = 0;
        let (attempts, error) = run_with_retries(0, || {
            calls += 1;
            async move { Err("nope".into()) }
        })
        .await;
        assert_eq!(attempts, 1);
        assert_eq!(error.as_deref(), Some("nope"));
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
