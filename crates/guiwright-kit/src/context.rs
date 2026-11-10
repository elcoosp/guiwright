//! Per-test context (§11 "process-per-test … Guiwright adds session-level
//! sharding" + §12 trace naming).
//!
//! Under cargo-nextest every test is its own process and nextest sets the
//! `NEXTEST_*` environment (§11 integration): `NEXTEST` = "1",
//! `NEXTEST_RUN_ID` (UUID per run), `NEXTEST_TEST_NAME`,
//! `NEXTEST_ATTEMPT` (1-indexed retry). The kit reads those at launch so
//! traces/screenshots/artifacts name themselves per test + attempt —
//! retry runs never overwrite the artifacts of earlier attempts.

use std::path::PathBuf;

/// Environment facts for the running test (§11).
#[derive(Clone, Debug, PartialEq)]
pub struct TestContext {
    /// Canonical test name: nextest's `NEXTEST_TEST_NAME` when present,
    /// else the function name the macro passed in.
    pub test_name: String,
    /// `NEXTEST_RUN_ID` under nextest, `None` in plain `cargo test`.
    pub run_id: Option<String>,
    /// `NEXTEST_ATTEMPT` (1-indexed); `None` outside nextest.
    pub attempt: Option<u32>,
    pub under_nextest: bool,
}

impl TestContext {
    /// Read the environment, with `test_name` as the plain-cargo fallback.
    pub fn from_env(test_name: &str) -> Self {
        let under_nextest = std::env::var("NEXTEST").ok().as_deref() == Some("1");
        let nextest_name = non_empty("NEXTEST_TEST_NAME");
        Self {
            test_name: nextest_name.unwrap_or_else(|| test_name.to_string()),
            run_id: non_empty("NEXTEST_RUN_ID"),
            attempt: std::env::var("NEXTEST_ATTEMPT")
                .ok()
                .and_then(|v| v.parse().ok()),
            under_nextest,
        }
    }

    /// Root directory for this test's artifacts (traces, screenshot
    /// artifacts): `target/guiwright/<run_id|local>/<test>/attempt-N`.
    ///
    /// The workspace root comes from nextest's `NEXTEST_WORKSPACE_ROOT`,
    /// falling back to `CARGO_MANIFEST_DIR` (plain cargo test).
    pub fn artifacts_dir(&self) -> PathBuf {
        let root = non_empty("NEXTEST_WORKSPACE_ROOT").map_or_else(
            || PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()),
            PathBuf::from,
        );
        root.join("target")
            .join("guiwright")
            .join(self.run_id.clone().unwrap_or_else(|| "local".into()))
            .join(sanitize(&self.test_name))
            .join(format!("attempt-{}", self.attempt.unwrap_or(1)))
    }

    /// Screenshot baseline root (`tests/screenshots`, §10 layout) inside
    /// the crate that owns the test.
    pub fn baseline_dir(&self) -> PathBuf {
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
            .join("tests")
            .join("screenshots")
    }

    /// `guiwright-<test>-<ts>.zip` (§12 archive naming).
    pub fn trace_path(&self) -> PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        self.artifacts_dir()
            .join("traces")
            .join(format!("guiwright-{}-{ts}.zip", sanitize(&self.test_name)))
    }
}

fn non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn sanitize(component: &str) -> String {
    let mut out = String::with_capacity(component.len());
    let mut last_dash = true;
    for c in component.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "unnamed".into()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_cargo_fallbacks() {
        let ctx = TestContext {
            test_name: "login_flow".into(),
            run_id: None,
            attempt: None,
            under_nextest: false,
        };
        assert!(!ctx.under_nextest);
        let trace = ctx.trace_path();
        let rendered = trace.to_string_lossy().to_string();
        assert!(
            rendered.contains("local/login_flow/attempt-1/traces/guiwright-login_flow-"),
            "{rendered}"
        );
        assert!(rendered.to_lowercase().ends_with(".zip"));
        assert!(
            ctx.baseline_dir()
                .to_string_lossy()
                .contains("tests/screenshots")
        );
    }

    #[test]
    fn nextest_naming_uses_run_id_and_attempt() {
        let ctx = TestContext {
            test_name: "crate::app::test_checkout".into(),
            run_id: Some("9f8e7d6c".into()),
            attempt: Some(3),
            under_nextest: true,
        };
        let dir = ctx.artifacts_dir();
        let rendered = dir.to_string_lossy().to_string();
        assert!(rendered.contains("9f8e7d6c/"), "{rendered}");
        assert!(rendered.contains("crate-app-test_checkout/"), "{rendered}");
        assert!(rendered.ends_with("attempt-3"), "{rendered}");
    }

    #[test]
    fn names_survive_path_characters() {
        assert_eq!(sanitize("tests::app::it works (2)"), "tests-app-it-works-2");
        assert_eq!(sanitize(""), "unnamed");
        assert_eq!(sanitize("---"), "unnamed");
    }
}
