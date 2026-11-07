//! The slint clock plan (§4.4 + §14): map [`ClockCmd`]s onto the testing
//! backend's `mock_elapsed_time` — first-class virtual time.
//!
//! Slint's testing backend has **no wall clock**: everything (animations,
//! timers, change handlers, repeaters) advances through
//! `mock_elapsed_time(ms)`. The plan is the pure mapping the glue calls;
//! stepping (frame-sized fast-forward) lives in the driver's clock
//! service (§14), so each step here is exactly one `mock_elapsed_time`.

use guiwright_driver::adapter::ClockCmd;

/// One slint clock action (the glue executes these in order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlintClockStep {
    /// `slint::testing::mock_elapsed_time(ms)` — advances animations,
    /// timers, change handlers, repeaters (§4.4 "first-class").
    MockElapsedTime { ms: u64 },
    /// Bookkeeping only: the testing backend is always virtual, so
    /// install is a no-op marker (the trace still records it, §12).
    MarkInstalled,
    /// Bookkeeping: pause/resume gate the driver's run_for, not slint.
    MarkPaused { paused: bool },
}

/// Pure plan for a clock command (§14 slint row).
pub fn plan(cmd: ClockCmd) -> Vec<SlintClockStep> {
    match cmd {
        ClockCmd::InstallVirtual => vec![SlintClockStep::MarkInstalled],
        ClockCmd::FastForward(ms) | ClockCmd::RunFor(ms) => {
            vec![SlintClockStep::MockElapsedTime {
                ms: ms.as_millis() as u64,
            }]
        }
        ClockCmd::SetTime(t) => vec![SlintClockStep::MockElapsedTime {
            ms: t.as_millis() as u64,
        }],
        ClockCmd::Pause => vec![SlintClockStep::MarkPaused { paused: true }],
        ClockCmd::Resume => vec![SlintClockStep::MarkPaused { paused: false }],
        // Probes are driver-side bookkeeping; slint needs no action.
        ClockCmd::Probe => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fast_forward_maps_to_mock_elapsed_time() {
        let steps = plan(ClockCmd::FastForward(Duration::from_millis(1_500)));
        assert_eq!(
            steps,
            vec![SlintClockStep::MockElapsedTime { ms: 1_500 }],
            "one step per driver step (stepping lives in the clock service)"
        );
    }

    #[test]
    fn install_is_a_marker_pause_resume_are_gates() {
        assert_eq!(
            plan(ClockCmd::InstallVirtual),
            vec![SlintClockStep::MarkInstalled]
        );
        assert_eq!(
            plan(ClockCmd::Pause),
            vec![SlintClockStep::MarkPaused { paused: true }]
        );
        assert_eq!(
            plan(ClockCmd::Resume),
            vec![SlintClockStep::MarkPaused { paused: false }]
        );
    }

    #[test]
    fn probe_needs_no_slint_action() {
        assert!(plan(ClockCmd::Probe).is_empty());
    }
}
