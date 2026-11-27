//! The §12 "open on failure" deep link at the kit surface — the plan's
//! own words pin the acceptance. The architecture doc's §12 viewer
//! paragraph:
//!
//! ```text
//! "open on failure" deep-links from assertion errors (`guiwright show-trace …`)
//! ```
//!
//! and the §16 parity map's error-context row claims the pair shipped:
//!
//! ```text
//! Error context (call log, DOM snapshot on failure) | error embeds tree
//! snapshot + trace link | MVP
//! ```
//!
//! The tree-snapshot half has ridden the core's timeout errors since M0
//! (`TreeSummary` in `CoreError::Timeout { last, .. }`); the trace-link
//! half never landed: the macro reported the failure *before*
//! `App::finish` ran, so a failing test's archive never finalized (the
//! runner's own rule — "a failing scenario's trace is the valuable one")
//! and no failure line named it. The failure report now finalizes the
//! archive first and carries the `guiwright show-trace` deep link.

use std::panic::AssertUnwindSafe;

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, TestOutcome};

/// The macro's exact sequence with a trace armed: launch, record, a
/// failing body (strict mode on a missing node — an assertion error, the
/// §12 deep-link's own subject), `finish` before the report. The
/// failing test's archive must exist on disk, and the panic payload must
/// carry the error line plus the plan's deep-link words with the path.
#[test]
fn the_failure_report_deep_links_the_finalized_archive() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("guiwright-deep-link-failure.zip");

    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("deep_link_failure", adapter);
    app.enable_trace(path.clone());

    let outcome = (|| -> Result<(), String> {
        app.get_by_role(Role::Button, "Never")
            .resolve()
            .map_err(|e| e.to_string())?;
        Ok(())
    })();

    let trace = app.finish().expect("the failure's archive finalizes");
    assert_eq!(trace, path, "finish answers the archive path");
    let bytes = std::fs::read(&path).expect("the archive is on disk");
    assert!(
        bytes.starts_with(b"PK"),
        "the zip finalizes ({} bytes)",
        bytes.len()
    );

    let payload = std::panic::catch_unwind(AssertUnwindSafe(|| {
        TestOutcome::report_with_trace(outcome, "deep_link_failure", Some(trace.as_path()));
    }))
    .expect_err("a failing outcome panics");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&'static str>()
                .map(|s| (*s).to_string())
        })
        .expect("the panic payload is a string");
    assert!(
        message.starts_with("guiwright test `deep_link_failure` failed: "),
        "the legacy line opens the report: {message}"
    );
    assert!(
        message.contains("no node matches"),
        "the error's own tree context rides along: {message}"
    );
    assert!(message.contains(&format!(
        "\ntrace: {} (guiwright show-trace {})",
        trace.display(),
        trace.display()
    )));
}

/// The macro end to end: the body arms a trace, returns `Err`, and the
/// generated failure report carries the deep link — the real codegen
/// path, not a re-enactment. The temp dir outlives the body by design
/// (`keep()` keeps it): the macro finalizes the archive *after* the
/// body returns, so a body-scoped guard would delete the directory
/// before the zip lands.
#[guiwright_kit::test]
#[should_panic(expected = "guiwright show-trace")]
fn macro_failure_deep_links_an_armed_trace(app: &mut App) -> Result<(), String> {
    let dir = tempfile::tempdir().expect("tmp");
    app.enable_trace(dir.keep().join("trace.zip"));
    Err("boom".into())
}

/// The no-archive shape stays the M0 line: `None` (no recording — the
/// common case) keeps the legacy report byte-for-byte.
#[test]
#[should_panic(expected = "guiwright test `deep_link_no_trace` failed: boom")]
fn the_no_trace_failure_keeps_the_report_shape() {
    TestOutcome::report_with_trace(Err::<(), String>("boom".into()), "deep_link_no_trace", None);
}

/// A passing outcome stays silent even when an archive finalized.
#[test]
fn the_passing_outcome_stays_silent() {
    TestOutcome::report_with_trace(
        Ok::<(), String>(()),
        "deep_link_pass",
        Some(std::path::Path::new("/tmp/gw-never-written.zip")),
    );
}

/// The unit-body outcome stays silent (the `()` arm never fails).
#[test]
fn the_unit_outcome_stays_silent() {
    TestOutcome::report_with_trace(
        (),
        "deep_link_unit",
        Some(std::path::Path::new("/tmp/gw-never-written.zip")),
    );
}

/// The legacy `report` method keeps its exact shape for third-party
/// callers (the trait's default `report_with_trace` delegates to it).
#[test]
#[should_panic(expected = "guiwright test `legacy_report` failed: boom")]
fn the_legacy_report_method_is_unchanged() {
    TestOutcome::report(Err::<(), String>("boom".into()), "legacy_report");
}
