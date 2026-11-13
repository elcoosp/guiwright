//! `expect(...)` assertions with retry-until-timeout semantics
//! (architecture §16 "Assertions with retry").
//!
//! The builder produces a [`ChecksWaiter`]; the *session loop* (driver, §5.1)
//! owns the deadline and feeds it events. This keeps assertions event-driven:
//! they complete exactly when the asserted condition becomes true, or time
//! out with the last tree summary embedded in the error.

use std::fmt::Write;
use std::time::Duration;

use crate::CoreError;
use crate::actionability::Check;
use crate::selector::{Selector, TextMatch};
use crate::tree::TreeService;
use crate::wait::{ChecksWaiter, CountWaiter, WaitOutcome, Waiter, WaiterEvent};

/// Default assertion timeout (Playwright default: 5 s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Entry point: `expect(locator).to_be_visible()` — a builder whose
/// `to_*` methods evaluate now (`Expectation::check_now(&tree)`) or
/// retry-until-timeout through [`crate::wait`].
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

    /// `expect(locator).toBeChecked()` — the node's AccessKit tri-state is
    /// `true` (M20: the §8 check vocabulary row).
    pub fn to_be_checked(self) -> Expectation {
        self.checks(vec![Check::Checked(accesskit::Toggled::True)], "checked")
    }

    /// `expect(locator).toBeChecked({ checked: false })` — the negated
    /// pair (`to_be_checked`'s complement, Playwright parity).
    pub fn to_be_unchecked(self) -> Expectation {
        self.checks(vec![Check::Checked(accesskit::Toggled::False)], "unchecked")
    }

    /// `expect(locator).toBeChecked({ checked: 'mixed' })` — the tri-state
    /// middle (indeterminate checkboxes, `Toggled::Mixed`).
    pub fn to_be_checked_mixed(self) -> Expectation {
        self.checks(
            vec![Check::Checked(accesskit::Toggled::Mixed)],
            "mixed-checked",
        )
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

    /// `expect(locator).toHaveCount(count)` — the selector-level assertion
    /// (M22): how many nodes match, strict mode never consulted. The
    /// mismatch is waitable (nodes stream in and out, Playwright parity);
    /// the timeout error names the expected count, the locator and the
    /// count the last evaluation found.
    pub fn to_have_count(self, expected: usize) -> Expectation {
        Expectation {
            waiter: Box::new(CountWaiter::new(self.locator.clone(), expected)),
            timeout: self.timeout,
            operation: format!("count {expected} — {}", self.locator.render()),
        }
    }

    fn checks(self, checks: Vec<Check>, name: &str) -> Expectation {
        Expectation {
            waiter: Box::new(ChecksWaiter::new(self.locator.clone(), checks)),
            timeout: self.timeout,
            operation: format!("{name} — {}", self.locator.render()),
        }
    }
}

/// A ready-to-run assertion. Drive it through [`Expectation::check`]
/// (single-shot) or feed events manually via [`Expectation::waiter`].
pub struct Expectation {
    /// Any waiter state machine (the M22 seam: the count assertion made
    /// this a dispatcher over `Waiter` instead of a hard-wired
    /// [`ChecksWaiter`] — selector-level assertions ride the same loop).
    waiter: Box<dyn Waiter>,
    timeout: Duration,
    /// Playwright-shaped failure header: the assertion name plus the
    /// locator it was built from ("checked — button \"Dark mode\"").
    /// Timeout errors embed it (§8.3 "the error embeds the last tree
    /// snapshot" — and names what was being waited for).
    operation: String,
}

impl std::fmt::Debug for Expectation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Expectation")
            .field("waiter", &self.waiter.describe())
            .field("timeout", &self.timeout)
            .field("operation", &self.operation)
            .finish()
    }
}

impl Expectation {
    pub fn waiter(&self) -> &dyn Waiter {
        &*self.waiter
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
            WaitOutcome::Pending => {
                // A waiter that can name its mismatch does (the M17
                // `stuck on …` convention): the count assertion reports the
                // count its last evaluation found.
                let mut operation = self.operation.clone();
                if let Some(stuck) = self.waiter.stuck_on() {
                    // Writing to a String cannot fail (the `render()`
                    // convention — the ignored result is the documented one).
                    let _ = write!(operation, " ({stuck})");
                }
                Err(CoreError::Timeout {
                    operation,
                    timeout: self.timeout,
                    last: Box::new(tree.summary()),
                })
            }
            WaitOutcome::Failed(failure) => Err(CoreError::Timeout {
                operation: format!("{} ({})", self.operation, failure.reason),
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
    fn checked_assertions_reflect_the_tri_state() {
        let fixture = Fixture::with_checkboxes();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        expect(Selector::role_named(Role::CheckBox, "Notifications"))
            .to_be_checked()
            .check_now(&tree)
            .expect("notifications is checked");
        expect(Selector::role_named(Role::CheckBox, "Dark mode"))
            .to_be_unchecked()
            .check_now(&tree)
            .expect("dark mode is unchecked");
        expect(Selector::role_named(Role::CheckBox, "Partially installed"))
            .to_be_checked_mixed()
            .check_now(&tree)
            .expect("partially installed is mixed");
    }

    #[test]
    fn a_wrong_checked_state_times_out_with_the_state_in_the_error() {
        let fixture = Fixture::with_checkboxes();
        let mut tree = TreeService::new();
        tree.apply_update(fixture.initial_update());
        let err = expect(Selector::role_named(Role::CheckBox, "Dark mode"))
            .to_be_checked()
            .check_now(&tree)
            .expect_err("dark mode is not checked");
        let msg = err.to_string();
        assert!(msg.contains("checked"), "{msg}");
        assert!(msg.contains("Dark mode"), "{msg}");
    }

    #[test]
    fn a_node_without_a_checked_state_fails_the_assertion() {
        let tree = tree_with_fixture();
        // A plain button never exposes a checked state: fail fast, no
        // five-second wait (the M20 fail-fast rule).
        let err = expect(Selector::role_named(Role::Button, "Save"))
            .to_be_checked()
            .check_now(&tree)
            .expect_err("a button is not checkable");
        let msg = err.to_string();
        assert!(msg.contains("no checked state"), "{msg}");
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

    // --- The count assertion (§16 assertions row, M22) --------------------

    #[test]
    fn to_have_count_passes_on_the_whole_match_set() {
        let tree = tree_with_fixture();
        // 4 buttons would trip strict mode for every node-level assertion;
        // count talks about the set, so this is a valid answer (the
        // non-strict seam, Playwright's toHaveCount).
        expect(Selector::role(Role::Button))
            .to_have_count(4)
            .check_now(&tree)
            .expect("the fixture ships 4 buttons");
        expect(Selector::role_named(Role::Button, "Ghost"))
            .to_have_count(0)
            .check_now(&tree)
            .expect("zero matches is a count too");
    }

    #[test]
    fn a_count_mismatch_times_out_naming_expected_and_found() {
        let tree = tree_with_fixture();
        let err = expect(Selector::role(Role::Button))
            .to_have_count(99)
            .check_now(&tree)
            .expect_err("4 buttons is not 99");
        let msg = err.to_string();
        assert!(msg.contains("count 99"), "{msg}");
        assert!(msg.contains("getByRole"), "{msg}");
        assert!(msg.contains("found 4"), "{msg}");
    }
}
