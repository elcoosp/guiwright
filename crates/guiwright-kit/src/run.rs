//! The `#[test]` body runner (§5.2 kit: `#[guiwright::test]`, fixtures).
//!
//! The macro generates a `#[test]` that launches the fixture app, runs
//! the body and reports via [`TestOutcome`] — panics and `Err` returns
//! both fail the test with context (§11 "error context: call log"), and
//! an archive that was armed finalizes *before* the report so the
//! failure line carries the §12 deep link ("open on failure" — the
//! `guiwright show-trace` command with the archive path).

use std::fmt;
use std::fmt::Write as _;

/// Outcome reporting for macro-generated tests.
pub trait TestOutcome {
    fn report(self, test_name: &str);

    /// Reports the outcome with the §12 deep link: when the outcome is a
    /// failure and the macro's `App::finish` finalized an archive, the
    /// panic payload carries the error line plus
    /// `trace: <path> (guiwright show-trace <path>)` — §12's "'open on
    /// failure' deep-links from assertion errors". `None` (nothing
    /// recorded) keeps the legacy [`Self::report`] shape byte-for-byte.
    /// The default delegates to [`Self::report`], so third-party
    /// implementors keep their shape.
    fn report_with_trace(self, test_name: &str, trace: Option<&std::path::Path>)
    where
        Self: Sized,
    {
        let _ = trace;
        self.report(test_name);
    }
}

impl TestOutcome for () {
    fn report(self, _test_name: &str) {}
}

impl<E: fmt::Display> TestOutcome for Result<(), E> {
    fn report(self, test_name: &str) {
        self.report_with_trace(test_name, None);
    }

    fn report_with_trace(self, test_name: &str, trace: Option<&std::path::Path>)
    where
        Self: Sized,
    {
        if let Err(e) = self {
            let mut message = format!("guiwright test `{test_name}` failed: {e}");
            if let Some(path) = trace {
                let _ = write!(
                    message,
                    "\ntrace: {} (guiwright show-trace {})",
                    path.display(),
                    path.display()
                );
            }
            panic!("{message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_outcome_reports_silently() {
        <() as TestOutcome>::report((), "unit_test");
    }

    #[test]
    #[should_panic(expected = "guiwright test `failing_test` failed: boom")]
    fn error_outcome_panics_with_context() {
        let result: Result<(), String> = Err("boom".into());
        TestOutcome::report(result, "failing_test");
    }

    #[test]
    #[should_panic(expected = "guiwright test `typed_error_test` failed: no such widget")]
    fn any_display_error_works() {
        // A std error type through the generic impl.
        let result: Result<(), std::io::Error> = Err(std::io::Error::other("no such widget"));
        TestOutcome::report(result, "typed_error_test");
    }

    #[test]
    fn ok_results_pass() {
        let result: Result<(), String> = Ok(());
        TestOutcome::report(result, "passing_test");
    }

    /// The §12 deep link, exact: the legacy line, then the archive path
    /// and the `guiwright show-trace` command — the plan's words.
    #[test]
    fn trace_deep_link_names_the_archive() {
        let path = std::path::Path::new("/tmp/gw/deep.zip");
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TestOutcome::report_with_trace(
                Err::<(), String>("boom".into()),
                "deep_link",
                Some(path),
            );
        }))
        .expect_err("a failing outcome panics");
        let message = payload.downcast_ref::<String>().expect("string payload");
        assert_eq!(
            message,
            "guiwright test `deep_link` failed: boom\n\
             trace: /tmp/gw/deep.zip (guiwright show-trace /tmp/gw/deep.zip)"
        );
    }

    /// `None` (nothing recorded) keeps the legacy shape byte-for-byte —
    /// the no-trace run's report is unchanged.
    #[test]
    fn no_trace_keeps_the_legacy_line() {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TestOutcome::report_with_trace(Err::<(), String>("boom".into()), "deep_link", None);
        }))
        .expect_err("a failing outcome panics");
        let message = payload.downcast_ref::<String>().expect("string payload");
        assert_eq!(message, "guiwright test `deep_link` failed: boom");
        assert!(
            !message.contains("trace:"),
            "no archive, no link: {message}"
        );
    }

    /// The default `report_with_trace` delegates to `report` — a
    /// third-party implementor's shape is untouched by the deep link.
    #[test]
    fn foreign_implementors_keep_their_shape() {
        struct Foreign;
        impl TestOutcome for Foreign {
            fn report(self, test_name: &str) {
                assert_eq!(test_name, "foreign");
            }
        }
        Foreign.report_with_trace("foreign", Some(std::path::Path::new("/tmp/gw/x.zip")));
    }
}
