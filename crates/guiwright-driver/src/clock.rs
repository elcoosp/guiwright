//! The virtual clock service (architecture §14 "Clock service" + §15.1
//! frame-boundary discipline).
//!
//! Semantics (§14):
//! * `fast_forward(dt)` advances virtual time **in frame-sized steps until
//!   drained** — timers, animations and subscriptions settle between steps
//!   because the session pumps a frame after every step.
//! * `run_for(dt)` paces real-time (one adapter jump, no stepping).
//! * `pause`/`resume` gate `run_for` (a paused clock refuses real-time
//!   pacing; virtual jumps remain allowed — tests stay deterministic).
//! * every command is recorded into a **ledger** so traces replay timing
//!   (§12 `clock.bin`, §14 "clock.bin in traces replays timing").
//!
//! All clocks are per-session (§14 "All clocks are per-session").

use std::time::Duration;

use crate::adapter::ClockState;

/// One frame at 60 Hz — the step size for fast-forward (§14 "frame-sized
/// steps", §15.1 "1 frame (≤16.7 ms)").
pub const FRAME_STEP: Duration = Duration::from_nanos(16_666_667);

/// One ledger entry: a clock command applied by the service, with the
/// virtual time **after** it. Shape-compatible with the `gwtrace/v1`
/// `ClockJump` record (the kit maps it into the trace archive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockLedgerEntry {
    /// Adapter frame the command settled at (best effort — session frame
    /// counter after the pump).
    pub frame_seq: u64,
    pub command: LedgerCommand,
    /// Virtual time after the command, microseconds.
    pub virtual_time_us: u64,
}

/// Ledger command vocabulary (mirrors `ClockCmd` minus the probe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerCommand {
    InstallVirtual,
    FastForward,
    RunFor,
    Pause,
    Resume,
    SetTime,
}

/// What a servicing command produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockReport {
    /// Adapter clock state after the command.
    pub state: ClockState,
    /// Frame-sized jumps applied by *this* command (fast-forward steps).
    pub steps: u64,
}

/// Per-session clock bookkeeping. The adapter owns the actual time source
/// (§14 per-framework injection table); the service plans the steps,
/// records the ledger and mirrors the state.
#[derive(Debug, Default)]
pub struct ClockService {
    ledger: Vec<ClockLedgerEntry>,
    installed: bool,
    paused: bool,
    /// Mirrored adapter virtual time (None until installed).
    virtual_time: Option<Duration>,
    /// Total frame-sized steps applied (lifetime of the session).
    total_steps: u64,
}

impl ClockService {
    pub fn new() -> Self {
        Self::default()
    }

    /// The virtual-time ledger (§12 `clock.bin` payload).
    pub fn ledger(&self) -> &[ClockLedgerEntry] {
        &self.ledger
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn total_steps(&self) -> u64 {
        self.total_steps
    }

    /// Plan `dt` as frame-sized steps (§14). The last step carries the
    /// remainder, so the total is exact.
    pub fn plan_steps(dt: Duration) -> Vec<Duration> {
        let nanos = dt.as_nanos();
        if nanos == 0 {
            return Vec::new();
        }
        let step_nanos = FRAME_STEP.as_nanos();
        let full = nanos / step_nanos;
        let rem = nanos % step_nanos;
        // Plan stays bounded: a tail beyond 1023 full steps folds into one
        // remainder jump (huge jumps still step, just without 60 k pumps).
        const MAX_FULL_STEPS: u128 = 1023;
        let folded_full = full.min(MAX_FULL_STEPS);
        let mut steps = Vec::with_capacity((folded_full + u128::from(rem > 0)) as usize);
        steps.extend(std::iter::repeat_n(FRAME_STEP, folded_full as usize));
        let tail_nanos = if full > MAX_FULL_STEPS {
            nanos - step_nanos * folded_full
        } else {
            rem
        };
        if tail_nanos > 0 {
            steps.push(Duration::from_nanos(tail_nanos as u64));
        }
        steps
    }

    /// Mirror the adapter's state after a command and record the ledger
    /// entry. `steps` counts the frame-sized jumps applied.
    pub fn record(
        &mut self,
        command: LedgerCommand,
        state: ClockState,
        frame_seq: u64,
        steps: u64,
    ) {
        self.installed = self.installed || state.virtual_time.is_some();
        self.paused = state.paused;
        self.virtual_time = state.virtual_time;
        self.total_steps += steps;
        let virtual_time_us = state
            .virtual_time
            .map(|t| t.as_micros() as u64)
            .unwrap_or(0);
        self.ledger.push(ClockLedgerEntry {
            frame_seq,
            command,
            virtual_time_us,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_forward_plans_exact_frame_steps() {
        // 3 full frames + remainder.
        let dt = Duration::from_nanos(FRAME_STEP.as_nanos() as u64 * 3 + 5_000_000);
        let steps = ClockService::plan_steps(dt);
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0], FRAME_STEP);
        assert_eq!(steps[3], Duration::from_millis(5));
        let total: u128 = steps.iter().map(|s| s.as_nanos()).sum();
        assert_eq!(total, dt.as_nanos(), "the plan is exact");
    }

    #[test]
    fn sub_frame_jump_is_one_step() {
        let steps = ClockService::plan_steps(Duration::from_millis(5));
        assert_eq!(steps, vec![Duration::from_millis(5)]);
    }

    #[test]
    fn zero_jump_plans_nothing() {
        assert!(ClockService::plan_steps(Duration::ZERO).is_empty());
    }

    #[test]
    fn huge_jump_folds_into_bounded_plan() {
        let steps = ClockService::plan_steps(Duration::from_secs(3600));
        assert_eq!(steps.len(), 1024, "plan stays bounded");
        let total: u128 = steps.iter().map(|s| s.as_nanos()).sum();
        assert_eq!(total, Duration::from_secs(3600).as_nanos());
    }

    #[test]
    fn ledger_records_virtual_time_after_command() {
        let mut svc = ClockService::new();
        svc.record(
            LedgerCommand::InstallVirtual,
            ClockState {
                virtual_time: Some(Duration::ZERO),
                paused: false,
            },
            1,
            0,
        );
        svc.record(
            LedgerCommand::FastForward,
            ClockState {
                virtual_time: Some(Duration::from_secs(30)),
                paused: false,
            },
            4,
            2,
        );
        assert!(svc.installed());
        assert_eq!(svc.total_steps(), 2);
        let ledger = svc.ledger();
        assert_eq!(ledger.len(), 2);
        assert_eq!(ledger[1].virtual_time_us, 30_000_000);
        assert_eq!(ledger[1].frame_seq, 4);
    }
}
