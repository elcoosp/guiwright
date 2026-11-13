//! The locator wait-for surface (§16 auto-waiting row, M21): Playwright's
//! `locator.waitFor(state)` — an explicit, one-shot wait that completes
//! exactly when the state holds, driven by the same event loop as every
//! locator action (§15.3 zero polling: waiters are woken by tree diffs
//! and presented frames, never by a sleep loop).
//!
//! The vocabulary maps 1:1 onto the §8.3 check table the actionability
//! gate already speaks — [`WaitState::Stable`] is the visible check plus
//! the M17-published stability semantics. No new session or wire surface:
//! the session's `wait_for` is the same loop `expect(locator).to_*`
//! drives.

use std::time::{Duration, Instant};

use guiwright_core::CoreError;
use guiwright_core::actionability::Check;
use guiwright_core::expect::DEFAULT_TIMEOUT;

use crate::app::Locator;
use guiwright_driver::adapter::FrameworkAdapter;

/// Playwright's `waitFor(state)` vocabulary (§16 auto-waiting row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitState {
    /// The node is present in the (consumer) tree.
    Attached,
    /// The node passes the §8.3 visible check (filter-included, bounded).
    Visible,
    /// The node is hidden — satisfied by absence too (Playwright parity).
    Hidden,
    /// Visible **and** bounds-identical across two presented frames (the
    /// §8.3 `Stable` semantics the M17 wire gate ships).
    Stable,
}

impl WaitState {
    /// The §8.3 check vocabulary each state maps onto (the session's
    /// `wait_for` drives exactly this list).
    pub fn checks(self) -> Vec<Check> {
        match self {
            WaitState::Attached => vec![Check::Attached],
            WaitState::Visible => vec![Check::Visible],
            WaitState::Hidden => vec![Check::Hidden],
            WaitState::Stable => vec![Check::Visible, Check::Stable],
        }
    }

    /// Stable rendering for errors and traces.
    pub fn describe(self) -> &'static str {
        match self {
            WaitState::Attached => "attached",
            WaitState::Visible => "visible",
            WaitState::Hidden => "hidden",
            WaitState::Stable => "stable",
        }
    }
}

impl<A: FrameworkAdapter> Locator<'_, A> {
    /// `locator.waitFor(state)` — waits until the state holds or the 5 s
    /// default timeout expires (Playwright parity, §16). The error names
    /// the state and the locator; nothing is recorded but the codegen
    /// line (`locator.wait_for();`), and the trace event rides the same
    /// `record_action` tap every locator action uses.
    pub fn wait_for(self, state: WaitState) -> Result<(), CoreError> {
        self.wait_for_with_timeout(state, DEFAULT_TIMEOUT)
    }

    /// `wait_for` with an explicit deadline (the 250 ms shape tests use).
    pub fn wait_for_with_timeout(
        self,
        state: WaitState,
        timeout: Duration,
    ) -> Result<(), CoreError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let deadline = Instant::now() + timeout;
        // A window-pinned locator (§16 v0.4) scopes the wait loop to its
        // subtree — the same act-time validation every scoped path uses.
        app.run_scoped(window.as_deref(), move |session| {
            session.wait_for(&selector, state.checks(), deadline)
        })
        .map(|_| ())
        .map_err(|e| match e {
            // Keep the failure header Playwright-shaped: the state and
            // the locator (§8.3 — errors name what was waited for).
            CoreError::Timeout { last, .. } => CoreError::Timeout {
                operation: format!("{} — {}", state.describe(), recorded.render()),
                timeout,
                last,
            },
            other => other,
        })?;
        app.record_scoped_action("wait_for", &recorded, None, window.as_deref());
        Ok(())
    }
}
