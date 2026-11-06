//! `expect(...)` assertions with retry-until-timeout semantics
//! (architecture §16 "Assertions with retry").
//!
//! The builder produces a [`ChecksWaiter`]; the *session loop* (driver, §5.1)
//! owns the deadline and feeds it events. This keeps assertions event-driven:
//! they complete exactly when the asserted condition becomes true, or time
//! out with the last tree summary embedded in the error.

use std::time::Duration;

use crate::CoreError;
use crate::actionability::Check;
use crate::selector::{Selector, TextMatch};
use crate::tree::TreeService;
use crate::wait::{ChecksWaiter, WaitOutcome, Waiter, WaiterEvent};

/// Default assertion timeout (Playwright default: 5 s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Entry point: `expect(locator).to_be_visible().check(&tree, deadline)`.
pub fn expect(locator: Selector) -> ExpectBuilder {
    ExpectBuilder {
        locator,
        timeout: DEFAULT_TIMEOUT,
    }
}

/// Builder for retry-until-timeout assertions.
#[derive(Debug)]
pub struct ExpectBuilder {
    locator: Selector,
    timeout: Duration,
}

impl ExpectBuilder {
    /// Override the timeout for this assertion.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `expect(locator).toBeVisible()`
    pub fn to_be_visible(self) -> Expectation {
        self.checks(vec![Check::Visible], "visible")
    }

    /// `expect(locator).toBeHidden()` — satisfied by absence too
    /// (Playwright parity).
    pub fn to_be_hidden(self) -> Expectation {
        self.checks(vec![Check::Hidden], "hidden")
    }

    /// `expect(locator).toBeEnabled()`
    pub fn to_be_enabled(self) -> Expectation {
        self.checks(vec![Check::Enabled], "enabled")
    }

    /// `expect(locator).toBeDisabled()`
    pub fn to_be_disabled(self) -> Expectation {
        self.checks(vec![Check::Disabled], "disabled")
    }

    /// `expect(locator).toBeFocused()`
    pub fn to_be_focused(self) -> Expectation {
        self.checks(vec![Check::Focused], "focused")
    }

    /// `expect(locator).toHaveText(text)`
    pub fn to_have_text(self, text: impl Into<TextMatch>) -> Expectation {
        self.checks(vec![Check::HasText(text.into())], "text")
    }

    /// `expect(locator).toHaveValue(value)`
    pub fn to_have_value(self, value: impl Into<TextMatch>) -> Expectation {
        // Values surface through the node value; HasText covers label-or-value.
        self.checks(vec![Check::HasText(value.into())], "value")
    }

    fn checks(self, checks: Vec<Check>, name: &str) -> Expectation {
        Expectation {
            waiter: ChecksWaiter::new(self.locator, checks),
            timeout: self.timeout,
            name: name.to_string(),
        }
    }
}

/// A ready-to-run assertion. Drive it through [`Expectation::check`]
/// (single-shot) or feed events manually via [`Expectation::waiter`].
#[derive(Debug)]
pub struct Expectation {
    waiter: ChecksWaiter,
    timeout: Duration,
    name: String,
}

impl Expectation {
    pub fn waiter(&self) -> &ChecksWaiter {
        &self.waiter
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Human description used in errors and traces.
    pub fn describe(&self) -> String {
        self.waiter.describe()
    }

    /// Single-event evaluation helper (used by tests and by sessions that
    /// already know the condition holds). Not a poll loop: production waits
    /// are driven by events through [`Self::waiter`].
    pub fn check_now(&mut self, tree: &TreeService) -> Result<(), CoreError> {
        match self.waiter.on_event(WaiterEvent::Initial, tree) {
            WaitOutcome::Satisfied(_) => Ok(()),
            WaitOutcome::Pending => Err(CoreError::Timeout {
                operation: self.name.clone(),
                timeout: self.timeout,
                last: Box::new(tree.summary()),
            }),
            WaitOutcome::Failed(failure) => Err(CoreError::Timeout {
                operation: format!("{} ({})", self.name, failure.reason),
                timeout: self.timeout,
                last: Box::new(tree.summary()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use accesskit::Role;

    use crate::test_util::Fixture;
    use crate::tree::TreeService;

    use super::*;

    fn tree_with_fixture() -> TreeService {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        tree
    }

    #[test]
    fn visible_passes_for_bounded_button() {
        let tree = tree_with_fixture();
        expect(Selector::role_named(Role::Button, "Save"))
            .to_be_visible()
            .check_now(&tree)
            .expect("save is visible");
    }

    #[test]
    fn hidden_passes_for_hidden_and_absent() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        expect(Selector::role_named(Role::Button, "Invisible"))
            .to_be_hidden()
            .check_now(&tree)
            .expect("hidden button is hidden");
        expect(Selector::role_named(Role::Button, "Ghost"))
            .to_be_hidden()
            .check_now(&tree)
            .expect("absent button is hidden");
    }

    #[test]
    fn disabled_passes_for_disabled_button() {
        let fixture = Fixture::buttons();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        expect(Selector::role_named(Role::Button, "Confirm"))
            .to_be_disabled()
            .check_now(&tree)
            .expect("confirm is disabled");
        expect(Selector::role_named(Role::Button, "Save"))
            .to_be_disabled()
            .check_now(&tree)
            .expect_err("save is not disabled");
    }

    #[test]
    fn to_have_text_matches_input_value() {
        let tree = tree_with_fixture();
        expect(Selector::role(Role::TextInput))
            .to_have_text("ada@example.dev")
            .check_now(&tree)
            .expect("value matches");
    }

    #[test]
    fn timeout_error_carries_tree_summary() {
        let tree = tree_with_fixture();
        let err = expect(Selector::role_named(Role::Button, "Nope"))
            .to_be_visible()
            .check_now(&tree)
            .expect_err("no such button");
        let msg = err.to_string();
        assert!(msg.contains("visible"), "{msg}");
        assert!(msg.contains("8 nodes"), "{msg}");
    }

    #[test]
    fn strict_violation_is_reported() {
        let tree = tree_with_fixture();
        let err = expect(Selector::role(Role::Button))
            .to_be_visible()
            .check_now(&tree)
            .expect_err("4 buttons is ambiguous");
        let msg = err.to_string();
        assert!(msg.contains("resolved to 4 nodes"), "{msg}");
    }
}
