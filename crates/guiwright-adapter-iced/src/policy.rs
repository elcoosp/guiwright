//! Emulator determinism → pump budget (§4.6 "iced_test::Emulator drives a
//! real Program headlessly with determinism modes (`Zen` = settle-to-
//! quiescence, `Patient`, `Immediate`)").
//!
//! The policy is framework-free on purpose: the harness asks it "may I run
//! another pass?" and the answer never depends on iced internals.

use std::time::Instant;

use guiwright_driver::adapter::FrameDeadline;

/// iced Emulator determinism modes (§4.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Settle-to-quiescence: pump until the app asks for no more repaints
    /// (default — matches the session's settle contract, §15.4).
    #[default]
    Zen,
    /// Bounded settling — soft cap on passes per pump call.
    Patient,
    /// Exactly one pass per pump call (frame-quantized debugging).
    Immediate,
}

/// Pass budget for [`Mode::Zen`] — practically unbounded settling; the
/// deadline still cuts it off (§6.1 "pump until quiescent or deadline").
pub const PUMP_BUDGET_ZEN: usize = 10_000;

/// Passes the harness may run inside one `pump` call for the mode.
pub fn pump_budget(mode: Mode) -> usize {
    match mode {
        Mode::Zen => PUMP_BUDGET_ZEN,
        Mode::Patient => 32,
        Mode::Immediate => 1,
    }
}

/// May the harness run one more pass? (mode budget ∧ deadline).
pub fn may_run_pass(mode: Mode, passes_done: usize, deadline: &FrameDeadline) -> bool {
    if deadline.expired() {
        return false;
    }
    passes_done < pump_budget(mode)
}

/// Deadline the harness should use for a settle loop (mode-aware: Zen
/// gives the loop the full window, Immediate never waits).
pub fn settle_deadline(mode: Mode, within: std::time::Duration) -> (Instant, usize) {
    (Instant::now() + within, pump_budget(mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn budgets_match_the_documented_semantics() {
        assert_eq!(pump_budget(Mode::Zen), PUMP_BUDGET_ZEN);
        assert_eq!(pump_budget(Mode::Patient), 32);
        assert_eq!(pump_budget(Mode::Immediate), 1);
    }

    #[test]
    fn zen_settles_patient_bounds_immediate_is_single_pass() {
        let far = FrameDeadline::within(Duration::from_secs(1));
        assert!(may_run_pass(Mode::Zen, 9_999, &far));
        assert!(!may_run_pass(Mode::Zen, PUMP_BUDGET_ZEN, &far));
        assert!(may_run_pass(Mode::Patient, 31, &far));
        assert!(!may_run_pass(Mode::Patient, 32, &far));
        assert!(may_run_pass(Mode::Immediate, 0, &far));
        assert!(!may_run_pass(Mode::Immediate, 1, &far));
    }

    #[test]
    fn deadline_cuts_every_mode() {
        let expired = FrameDeadline::within(Duration::ZERO);
        std::thread::sleep(Duration::from_millis(1));
        assert!(!may_run_pass(Mode::Zen, 0, &expired));
        assert!(!may_run_pass(Mode::Immediate, 0, &expired));
    }

    #[test]
    fn mode_defaults_to_zen() {
        assert_eq!(Mode::default(), Mode::Zen);
    }
}
