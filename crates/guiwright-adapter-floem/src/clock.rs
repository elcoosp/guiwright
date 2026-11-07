//! The floem clock plan (§4.7 + §14): harness-driven timers.
//!
//! Floem's timers live in the harness action queue (`action.rs`, §4.7
//! "harness-driven timers — Guiwright pumps `Event::Window(UpdatePhase)`
//! ticks with virtual time"). The plan is the pure mapping the glue
//! executes; stepping (frame-sized fast-forward) lives in the driver's
//! clock service (§14), so each step here is exactly one harness tick.

use guiwright_driver::adapter::ClockCmd;

/// One floem clock action (the glue executes these in order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloemClockStep {
    /// Advance virtual time by one tick: the glue injects
    /// `Event::Window(WindowEvent::UpdatePhase)` and the harness drains
    /// the timers scheduled before the (virtual) deadline.
    Tick { advance_ms: u64 },
    /// Bookkeeping: the harness clock is virtual from the first tick, so
    /// install is a marker (the trace records it, §12).
    MarkInstalled,
    /// Bookkeeping: pause/resume gate the driver's run_for, not floem.
    MarkPaused { paused: bool },
}

/// Pure plan for a clock command (§14 floem row).
pub fn plan(cmd: ClockCmd) -> Vec<FloemClockStep> {
    match cmd {
        ClockCmd::InstallVirtual => vec![FloemClockStep::MarkInstalled],
        ClockCmd::FastForward(ms) | ClockCmd::RunFor(ms) => {
            vec![FloemClockStep::Tick {
                advance_ms: ms.as_millis() as u64,
            }]
        }
        ClockCmd::SetTime(t) => vec![FloemClockStep::Tick {
            advance_ms: t.as_millis() as u64,
        }],
        ClockCmd::Pause => vec![FloemClockStep::MarkPaused { paused: true }],
        ClockCmd::Resume => vec![FloemClockStep::MarkPaused { paused: false }],
        // Probes are driver-side bookkeeping; floem needs no action.
        ClockCmd::Probe => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fast_forward_maps_to_one_harness_tick_per_driver_step() {
        assert_eq!(
            plan(ClockCmd::FastForward(Duration::from_millis(16))),
            vec![FloemClockStep::Tick { advance_ms: 16 }],
            "the driver's clock service issues one step per frame (§14)"
        );
    }

    #[test]
    fn set_time_is_an_absolute_tick() {
        assert_eq!(
            plan(ClockCmd::SetTime(Duration::from_secs(30))),
            vec![FloemClockStep::Tick { advance_ms: 30_000 }]
        );
    }

    #[test]
    fn install_is_a_marker_pause_resume_are_gates() {
        assert_eq!(
            plan(ClockCmd::InstallVirtual),
            vec![FloemClockStep::MarkInstalled]
        );
        assert_eq!(
            plan(ClockCmd::Pause),
            vec![FloemClockStep::MarkPaused { paused: true }]
        );
        assert_eq!(
            plan(ClockCmd::Resume),
            vec![FloemClockStep::MarkPaused { paused: false }]
        );
        assert!(plan(ClockCmd::Probe).is_empty());
    }
}
