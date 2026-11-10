//! The `#[test]` body runner (§5.2 kit: `#[guiwright::test]`, fixtures).
//!
//! The macro generates a `#[test]` that launches the fixture app, runs
//! the body and reports via [`TestOutcome`] — panics and `Err` returns
//! both fail the test with context (§11 "error context: call log").

use std::fmt;

/// Outcome reporting for macro-generated tests.
pub trait TestOutcome {
    fn report(self, test_name: &str);
}

impl TestOutcome for () {
    fn report(self, _test_name: &str) {}
}

impl<E: fmt::Display> TestOutcome for Result<(), E> {
    fn report(self, test_name: &str) {
        if let Err(e) = self {
            panic!("guiwright test `{test_name}` failed: {e}");
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
}
