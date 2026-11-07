//! Run reports (M5, §16 "JUnit/HTML reports" + §5.2 "report glue"): the
//! live progress lines, the `gwrun/v1` JSON document, the JUnit XML (one
//! `<testsuite>` per spec file) and the self-contained HTML summary (the
//! `trace.html` precedent). All renderers are pure functions over
//! [`RunReport`] — file writing is the runner's job.

/// Which report `--report <kind>` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReportKind {
    /// stdout progress + summary (the default; no file).
    List,
    /// `report.json` — the `gwrun/v1` document.
    Json,
    /// `junit.xml` — one `<testsuite>` per spec file.
    Junit,
    /// `report.html` — self-contained summary + table.
    Html,
}

impl ReportKind {
    /// Parse the `--report` flag value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "list" => Some(ReportKind::List),
            "json" => Some(ReportKind::Json),
            "junit" => Some(ReportKind::Junit),
            "html" => Some(ReportKind::Html),
            _ => None,
        }
    }

    /// The file written under the report output dir.
    pub fn file_name(self) -> &'static str {
        match self {
            ReportKind::List | ReportKind::Html => "report.html",
            ReportKind::Json => "report.json",
            ReportKind::Junit => "junit.xml",
        }
    }
}

/// Final status of one scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioStatus {
    /// First attempt passed.
    Passed,
    /// Failed at least once, passed on a retry.
    Flaky,
    /// Every attempt failed (or the session could not be established).
    Failed,
}

impl ScenarioStatus {
    /// The lowercase tag used in the JSON report and progress lines.
    pub fn tag(self) -> &'static str {
        match self {
            ScenarioStatus::Passed => "passed",
            ScenarioStatus::Flaky => "flaky",
            ScenarioStatus::Failed => "failed",
        }
    }
}

/// One scenario's run record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioRecord {
    /// The spec file as discovered (display path).
    pub file: String,
    /// The scenario name within the file.
    pub name: String,
    pub status: ScenarioStatus,
    /// 1 = passed first try; > 1 with [`ScenarioStatus::Flaky`].
    pub attempts: u32,
    /// Wall time across all attempts (ms).
    pub duration_ms: u64,
    /// The last failure message (`None` when passed).
    pub error: Option<String>,
}

/// Aggregated counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub flaky: usize,
    pub failed: usize,
}

/// The whole run's report model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunReport {
    /// Epoch milliseconds when the run started.
    pub started_ms: u128,
    /// Wall time of the whole run (ms).
    pub duration_ms: u64,
    pub workers: usize,
    /// `--shard k/n` (1-indexed), when sharded.
    pub shard: Option<(u32, u32)>,
    /// In discovery order, regardless of completion order.
    pub scenarios: Vec<ScenarioRecord>,
}

impl RunReport {
    pub fn summary(&self) -> RunSummary {
        let mut summary = RunSummary {
            total: self.scenarios.len(),
            passed: 0,
            flaky: 0,
            failed: 0,
        };
        for record in &self.scenarios {
            match record.status {
                ScenarioStatus::Passed => summary.passed += 1,
                ScenarioStatus::Flaky => summary.flaky += 1,
                ScenarioStatus::Failed => summary.failed += 1,
            }
        }
        summary
    }
}

/// The spec file's stem: `a/b/login.spec.toml` → `login` (the JUnit
/// `classname`).
pub fn spec_stem(file: &str) -> &str {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    base.strip_suffix(crate::spec::SPEC_SUFFIX)
        .or_else(|| base.strip_suffix(".toml"))
        .unwrap_or(base)
}

/// Milliseconds rendered the way the CLI prints durations.
fn fmt_ms(ms: u64) -> String {
    format!("{ms} ms")
}

/// XML metacharacter escaping (`"` included — every attribute is quoted).
fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// HTML text escaping (attribute-safe: quotes included).
fn escape_html(text: &str) -> String {
    escape_xml(text)
}

fn seconds(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// The live progress line for one finished scenario (stdout).
pub fn progress_line(record: &ScenarioRecord) -> String {
    let id = format!("{} :: {}", record.file, record.name);
    match record.status {
        ScenarioStatus::Passed => format!(" PASS  {} ({})", id, fmt_ms(record.duration_ms)),
        ScenarioStatus::Failed => match &record.error {
            Some(error) => format!(
                " FAIL  {} ({})\n       {}",
                id,
                fmt_ms(record.duration_ms),
                error
            ),
            None => format!(" FAIL  {} ({})", id, fmt_ms(record.duration_ms)),
        },
        ScenarioStatus::Flaky => format!(
            " FLAKY {} (attempt {}, {})",
            id,
            record.attempts,
            fmt_ms(record.duration_ms)
        ),
    }
}

/// The run's closing summary line (stdout).
pub fn summary_line(report: &RunReport) -> String {
    let summary = report.summary();
    let mut line = format!(
        "{} scenario{}: {} passed, {} failed, {} flaky ({})",
        summary.total,
        if summary.total == 1 { "" } else { "s" },
        summary.passed,
        summary.failed,
        summary.flaky,
        fmt_ms(report.duration_ms),
    );
    if let Some((index, of)) = report.shard {
        line.push_str(&format!(" — shard {index}/{of}"));
    }
    line
}

/// JSON string literal with escapes (RFC 8259 subset).
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The `gwrun/v1` JSON document.
pub fn render_json(report: &RunReport) -> String {
    let summary = report.summary();
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema\": \"gwrun/v1\",\n");
    out.push_str(&format!("  \"started_ms\": {},\n", report.started_ms));
    out.push_str(&format!("  \"duration_ms\": {},\n", report.duration_ms));
    out.push_str(&format!("  \"workers\": {},\n", report.workers));
    if let Some((index, of)) = report.shard {
        out.push_str(&format!(
            "  \"shard\": {{\"index\": {index}, \"of\": {of}}},\n"
        ));
    }
    out.push_str(&format!(
        "  \"summary\": {{\"total\": {}, \"passed\": {}, \"flaky\": {}, \"failed\": {}}},\n",
        summary.total, summary.passed, summary.flaky, summary.failed
    ));
    out.push_str("  \"scenarios\": [");
    for (i, record) in report.scenarios.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("\n    {");
        out.push_str(&format!("\"file\": {}", json_string(&record.file)));
        out.push_str(&format!(", \"name\": {}", json_string(&record.name)));
        out.push_str(&format!(", \"status\": \"{}\"", record.status.tag()));
        out.push_str(&format!(", \"attempts\": {}", record.attempts));
        out.push_str(&format!(", \"duration_ms\": {}", record.duration_ms));
        match &record.error {
            Some(error) => out.push_str(&format!(", \"error\": {}", json_string(error))),
            None => out.push_str(", \"error\": null"),
        }
        out.push('}');
    }
    if !report.scenarios.is_empty() {
        out.push_str("\n  ");
    }
    out.push_str("]\n}");
    out
}

/// The JUnit XML document (one `<testsuite>` per spec file; flaky counts
/// as passed — JUnit has no flaky concept, the JSON/HTML reports carry it).
pub fn render_junit(report: &RunReport) -> String {
    let summary = report.summary();
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites tests=\"{}\" failures=\"{}\" time=\"{}\">\n",
        summary.total,
        summary.failed,
        seconds(report.duration_ms)
    ));
    // Group in discovery order, preserving each file's first appearance.
    let mut order: Vec<&str> = Vec::new();
    for record in &report.scenarios {
        if !order.contains(&record.file.as_str()) {
            order.push(&record.file);
        }
    }
    for file in order {
        let suite: Vec<&ScenarioRecord> = report
            .scenarios
            .iter()
            .filter(|r| r.file == file)
            .collect();
        let failures = suite
            .iter()
            .filter(|r| r.status == ScenarioStatus::Failed)
            .count();
        let time: u64 = suite.iter().map(|r| r.duration_ms).sum();
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" time=\"{}\">\n",
            escape_xml(spec_stem(file)),
            suite.len(),
            failures,
            seconds(time)
        ));
        for record in suite {
            match record.status {
                ScenarioStatus::Failed => {
                    let error = record.error.as_deref().unwrap_or("scenario failed");
                    out.push_str(&format!(
                        "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\">\n",
                        escape_xml(&record.name),
                        escape_xml(spec_stem(&record.file)),
                        seconds(record.duration_ms)
                    ));
                    out.push_str(&format!(
                        "      <failure message=\"{}\" type=\"assertion\">{}</failure>\n",
                        escape_xml(error),
                        escape_xml(error)
                    ));
                    out.push_str("    </testcase>\n");
                }
                ScenarioStatus::Passed | ScenarioStatus::Flaky => {
                    out.push_str(&format!(
                        "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\"/>\n",
                        escape_xml(&record.name),
                        escape_xml(spec_stem(&record.file)),
                        seconds(record.duration_ms)
                    ));
                }
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// The self-contained HTML report (the `trace.html` precedent: one file,
/// no assets).
pub fn render_html(report: &RunReport) -> String {
    let summary = report.summary();
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>guiwright run report</title>\n");
    out.push_str("<style>\n");
    out.push_str(":root { color-scheme: light dark; }\n");
    out.push_str("body { font-family: system-ui, sans-serif; margin: 2rem auto; max-width: 60rem; padding: 0 1rem; }\n");
    out.push_str("h1 { font-size: 1.3rem; }\n");
    out.push_str(".meta { opacity: 0.7; margin-bottom: 1.5rem; }\n");
    out.push_str(".cards { display: flex; gap: 1rem; margin-bottom: 1.5rem; flex-wrap: wrap; }\n");
    out.push_str(".card { border: 1px solid color-mix(in srgb, currentColor 25%, transparent); border-radius: 8px; padding: 0.6rem 1.2rem; }\n");
    out.push_str(".card .num { font-size: 1.4rem; font-weight: 600; }\n");
    out.push_str("table { border-collapse: collapse; width: 100%; }\n");
    out.push_str("th, td { text-align: left; padding: 0.4rem 0.7rem; border-bottom: 1px solid color-mix(in srgb, currentColor 18%, transparent); vertical-align: top; }\n");
    out.push_str("td.err { font-family: ui-monospace, monospace; font-size: 0.85rem; white-space: pre-wrap; }\n");
    out.push_str(".passed { color: #1a7f37; font-weight: 600; }\n");
    out.push_str(".failed { color: #cf222e; font-weight: 600; }\n");
    out.push_str(".flaky { color: #9a6700; font-weight: 600; }\n");
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str("<h1>guiwright run report</h1>\n");
    let shard = match report.shard {
        Some((index, of)) => format!(" &middot; shard {index}/{of}"),
        None => String::new(),
    };
    out.push_str(&format!(
        "<p class=\"meta\">{} workers{} &middot; total {}</p>\n",
        report.workers,
        shard,
        escape_html(&fmt_ms(report.duration_ms))
    ));
    out.push_str("<div class=\"cards\">\n");
    for (label, class, num) in [
        ("passed", "passed", summary.passed),
        ("flaky", "flaky", summary.flaky),
        ("failed", "failed", summary.failed),
        ("total", "", summary.total),
    ] {
        out.push_str(&format!(
            "<div class=\"card\"><div class=\"num {class}\">{num}</div><div>{label}</div></div>\n"
        ));
    }
    out.push_str("</div>\n");
    out.push_str("<table>\n<tr><th>spec</th><th>scenario</th><th>status</th><th>attempts</th><th>time</th><th>error</th></tr>\n");
    for record in &report.scenarios {
        let error = record.error.as_deref().unwrap_or("");
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{} ms</td><td class=\"err\">{}</td></tr>\n",
            escape_html(&record.file),
            escape_html(&record.name),
            record.status.tag(),
            record.status.tag(),
            record.attempts,
            record.duration_ms,
            escape_html(error)
        ));
    }
    out.push_str("</table>\n</body>\n</html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        name: &str,
        status: ScenarioStatus,
        attempts: u32,
        error: Option<&str>,
    ) -> ScenarioRecord {
        ScenarioRecord {
            file: "tests/guiwright/login.spec.toml".into(),
            name: name.into(),
            status,
            attempts,
            duration_ms: 112,
            error: error.map(Into::into),
        }
    }

    fn sample() -> RunReport {
        RunReport {
            started_ms: 1_730_970_000_000,
            duration_ms: 890,
            workers: 2,
            shard: Some((1, 3)),
            scenarios: vec![
                record("save the document", ScenarioStatus::Passed, 1, None),
                record(
                    "missing button",
                    ScenarioStatus::Failed,
                    1,
                    Some("expectation failed: role=text 'Saved' did not resolve within 5000 ms"),
                ),
                record("retry path", ScenarioStatus::Flaky, 2, None),
            ],
        }
    }

    #[test]
    fn kind_parses_flags() {
        assert_eq!(ReportKind::parse("list"), Some(ReportKind::List));
        assert_eq!(ReportKind::parse("json"), Some(ReportKind::Json));
        assert_eq!(ReportKind::parse("junit"), Some(ReportKind::Junit));
        assert_eq!(ReportKind::parse("html"), Some(ReportKind::Html));
        assert_eq!(ReportKind::parse("xml"), None);
        assert_eq!(ReportKind::parse(""), None);
    }

    #[test]
    fn kind_file_names() {
        assert_eq!(ReportKind::Json.file_name(), "report.json");
        assert_eq!(ReportKind::Junit.file_name(), "junit.xml");
        assert_eq!(ReportKind::Html.file_name(), "report.html");
    }

    #[test]
    fn summary_counts_statuses() {
        let summary = sample().summary();
        assert_eq!(
            summary,
            RunSummary {
                total: 3,
                passed: 1,
                flaky: 1,
                failed: 1
            }
        );
    }

    #[test]
    fn json_document_shape() {
        let json = render_json(&sample());
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(value["schema"], "gwrun/v1");
        assert_eq!(value["duration_ms"], 890);
        assert_eq!(value["workers"], 2);
        assert_eq!(value["shard"]["index"], 1);
        assert_eq!(value["shard"]["of"], 3);
        assert_eq!(value["summary"]["total"], 3);
        assert_eq!(value["summary"]["passed"], 1);
        assert_eq!(value["summary"]["flaky"], 1);
        assert_eq!(value["summary"]["failed"], 1);
        let scenarios = value["scenarios"].as_array().unwrap();
        assert_eq!(scenarios.len(), 3);
        assert_eq!(scenarios[0]["file"], "tests/guiwright/login.spec.toml");
        assert_eq!(scenarios[0]["name"], "save the document");
        assert_eq!(scenarios[0]["status"], "passed");
        assert_eq!(scenarios[0]["attempts"], 1);
        assert_eq!(scenarios[0]["error"], serde_json::Value::Null);
        assert_eq!(scenarios[1]["status"], "failed");
        assert!(
            scenarios[1]["error"]
                .as_str()
                .unwrap()
                .contains("did not resolve")
        );
        assert_eq!(scenarios[2]["status"], "flaky");
        assert_eq!(scenarios[2]["attempts"], 2);
    }

    #[test]
    fn json_omits_shard_when_unsharded() {
        let mut report = sample();
        report.shard = None;
        let json = render_json(&report);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert!(value.get("shard").is_none());
    }

    #[test]
    fn json_escapes_special_characters() {
        let mut report = sample();
        report.scenarios = vec![record(
            "quote\" and \\ back",
            ScenarioStatus::Failed,
            1,
            Some("line one\nline \"two\""),
        )];
        let json = render_json(&report);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(value["scenarios"][0]["name"], "quote\" and \\ back");
        assert_eq!(value["scenarios"][0]["error"], "line one\nline \"two\"");
    }

    #[test]
    fn junit_one_suite_per_file_with_failures() {
        let xml = render_junit(&sample());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<testsuites tests=\"3\" failures=\"1\""));
        assert!(xml.contains("<testsuite name=\"login\" tests=\"3\" failures=\"1\""));
        assert!(
            xml.contains("<testcase name=\"save the document\" classname=\"login\" time=\"0.112\"/>")
        );
        let failure = xml
            .match_indices("<failure")
            .map(|(i, _)| &xml[i..i + 160])
            .next()
            .expect("one failure element");
        assert!(failure.contains("message=\"expectation failed:"), "{failure}");
        assert!(failure.contains("did not resolve"), "{failure}");
        // Flaky testcase (self-closing) has no failure child.
        let flaky_idx = xml.find("retry path").expect("flaky testcase present");
        let rest = &xml[flaky_idx..];
        let end = rest.find("/>").expect("self-closing flaky testcase");
        assert!(!rest[..end].contains("<failure"));
    }

    #[test]
    fn junit_escapes_xml_metacharacters() {
        let mut report = sample();
        report.scenarios = vec![record(
            "bad <name> & \"quote\"",
            ScenarioStatus::Failed,
            1,
            Some("expected <expected> & got \"other\""),
        )];
        let xml = render_junit(&report);
        assert!(
            xml.contains("bad &lt;name&gt; &amp; &quot;quote&quot;"),
            "{xml}"
        );
        assert!(!xml.contains("bad <name>"), "{xml}");
        assert!(
            xml.contains("expected &lt;expected&gt; &amp; got &quot;other&quot;"),
            "{xml}"
        );
    }

    #[test]
    fn junit_empty_run_is_well_formed() {
        let mut report = sample();
        report.scenarios.clear();
        let xml = render_junit(&report);
        assert!(xml.contains("<testsuites tests=\"0\" failures=\"0\""));
        assert!(!xml.contains("<testsuite "));
    }

    #[test]
    fn html_contains_summary_and_escaped_rows() {
        let html = render_html(&sample());
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("guiwright"));
        assert!(html.contains("save the document"));
        assert!(html.contains("missing button"));
        assert!(html.contains("did not resolve"));
        assert!(html.contains("flaky"));
        assert!(html.contains("shard 1/3"));
        assert!(html.contains("890 ms"));
    }

    #[test]
    fn html_escapes_user_strings() {
        let mut report = sample();
        report.scenarios = vec![record(
            "evil <img src=x onerror=alert(1)>",
            ScenarioStatus::Failed,
            1,
            Some("<script>alert(1)</script>"),
        )];
        let html = render_html(&report);
        assert!(!html.contains("<script>alert"), "{html}");
        assert!(!html.contains("<img src=x"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn progress_lines_render_by_status() {
        let pass = progress_line(&record("save the document", ScenarioStatus::Passed, 1, None));
        assert_eq!(
            pass,
            " PASS  tests/guiwright/login.spec.toml :: save the document (112 ms)"
        );
        let fail = progress_line(&record(
            "missing button",
            ScenarioStatus::Failed,
            1,
            Some("expectation failed: boom"),
        ));
        assert_eq!(
            fail,
            " FAIL  tests/guiwright/login.spec.toml :: missing button (112 ms)\n       expectation failed: boom"
        );
        let flaky = progress_line(&record("retry path", ScenarioStatus::Flaky, 2, None));
        assert_eq!(
            flaky,
            " FLAKY tests/guiwright/login.spec.toml :: retry path (attempt 2, 112 ms)"
        );
    }

    #[test]
    fn summary_line_counts_and_shard() {
        let line = summary_line(&sample());
        assert!(line.contains("3 scenarios"), "{line}");
        assert!(line.contains("1 passed"), "{line}");
        assert!(line.contains("1 failed"), "{line}");
        assert!(line.contains("1 flaky"), "{line}");
        assert!(line.contains("(890 ms)"), "{line}");
        assert!(line.contains("shard 1/3"), "{line}");
        let mut unsharded = sample();
        unsharded.shard = None;
        assert!(!summary_line(&unsharded).contains("shard"));
    }

    #[test]
    fn spec_stem_strips_only_the_suffix() {
        assert_eq!(spec_stem("tests/guiwright/login.spec.toml"), "login");
        assert_eq!(spec_stem("weird.name.spec.toml"), "weird.name");
        assert_eq!(spec_stem("plain.toml"), "plain");
    }
}
