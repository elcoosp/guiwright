//! The test-support dispatcher clock plan (§4.5, §6.2 GPUI row "⚠️
//! test-support dispatcher", §14).
//!
//! GPUI has no first-class virtual clock: `test-support` swaps in the
//! `TestPlatform` with a seeded foreground dispatcher
//! (`src/platform/test/…`). The plan is the pure mapping the glue
//! executes; stepping (frame-sized fast-forward) lives in the driver's
//! clock service (§14), so each step here is exactly one dispatch round:
//! the glue calls `dispatch_after(Duration::ZERO)` and drains the queued
//! timers up to the virtual deadline — the §5.1 waker row.

use guiwright_driver::adapter::ClockCmd;

/// One GPUI clock action (the glue executes these in order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuiClockStep {
    /// Swap the dispatcher to the seeded virtual scheduler
    /// (`Application::with_platform(TestPlatform…)`, §4.5 "precisely what
    /// `TestPlatform` does"). Marker: install happens at app build time;
    /// the trace records the driver-side install (§12).
    MarkInstalled,
    /// One dispatch round: run every timer/task scheduled before the
    /// (virtual) deadline, then flush a frame. The clock service issues
    /// one round per frame-sized step (§14).
    DispatchRound { advance_ms: u64 },
    /// Bookkeeping: pause/resume gate the driver's run_for, not GPUI.
    MarkPaused { paused: bool },
}

/// Pure plan for a clock command (§14 gpui row).
pub fn plan(cmd: ClockCmd) -> Vec<GpuiClockStep> {
    match cmd {
        ClockCmd::InstallVirtual => vec![GpuiClockStep::MarkInstalled],
        ClockCmd::FastForward(ms) | ClockCmd::RunFor(ms) => {
            vec![GpuiClockStep::DispatchRound {
                advance_ms: ms.as_millis() as u64,
            }]
        }
        ClockCmd::SetTime(t) => vec![GpuiClockStep::DispatchRound {
            advance_ms: t.as_millis() as u64,
        }],
        ClockCmd::Pause => vec![GpuiClockStep::MarkPaused { paused: true }],
        ClockCmd::Resume => vec![GpuiClockStep::MarkPaused { paused: false }],
        // Probes are driver-side bookkeeping; GPUI needs no action.
        ClockCmd::Probe => vec![],
    }
}

/// The clock precision the adapter reports in `Caps.clock` + traces (the
/// M2 spike §2 "a GPUI adapter reports its precision in `Caps.clock`").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPrecision {
    /// `test-support` dispatcher: deterministic rounds.
    VirtualRounds,
    /// Live mode: best-effort (real dispatcher, no seeding).
    BestEffort,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fast_forward_maps_to_one_dispatch_round_per_driver_step() {
        assert_eq!(
            plan(ClockCmd::FastForward(Duration::from_millis(16))),
            vec![GpuiClockStep::DispatchRound { advance_ms: 16 }],
            "the driver's clock service issues one step per frame (§14)"
        );
    }

    #[test]
    fn install_is_a_marker_pause_resume_are_gates() {
        assert_eq!(
            plan(ClockCmd::InstallVirtual),
            vec![GpuiClockStep::MarkInstalled]
        );
        assert_eq!(
            plan(ClockCmd::Pause),
            vec![GpuiClockStep::MarkPaused { paused: true }]
        );
        assert_eq!(
            plan(ClockCmd::Resume),
            vec![GpuiClockStep::MarkPaused { paused: false }]
        );
        assert!(plan(ClockCmd::Probe).is_empty());
    }

    #[test]
    fn set_time_is_an_absolute_round() {
        assert_eq!(
            plan(ClockCmd::SetTime(Duration::from_secs(5))),
            vec![GpuiClockStep::DispatchRound { advance_ms: 5_000 }]
        );
    }
}
