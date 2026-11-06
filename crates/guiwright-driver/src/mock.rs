//! The scriptable mock adapter — the TDD workhorse for driver, kit and
//! bench tests (feature `mock`).
//!
//! Model (mirrors harness-driven Level-1 adapters like egui/masonry):
//! every `pump` presents exactly one pass (frame stamp), scripted tree
//! diffs are delivered one per pump, and the app requests repaints while
//! scripted work remains. Tests script the frame/update stream explicitly.

use std::collections::VecDeque;
use std::time::Duration;

use accesskit::NodeId;

use crate::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameworkAdapter, LogicalTreeDiff, PumpOutcome, SemAction,
};
use guiwright_core::test_util::Fixture;

/// A scriptable in-process adapter.
#[derive(Debug)]
pub struct MockAdapter {
    meta: AdapterMeta,
    updates: VecDeque<accesskit::TreeUpdate>,
    logical: VecDeque<LogicalTreeDiff>,
    frames: VecDeque<u64>,
    seq: u64,
    /// Actions recorded for assertions.
    actions: Vec<(NodeId, SemAction)>,
    /// Scripted action results (popped in order; default = success).
    action_results: VecDeque<Result<(u64, String), AdapterError>>,
    /// Scripted pixels for `render_frame` (M1 capture pipeline tests, §10).
    pixels: Option<(u32, u32, std::sync::Arc<[u8]>)>,
    clock: ClockState,
    /// Number of pumps before the adapter reports no repaint requested
    /// (quiescence model). Zero = settled immediately.
    repaints_left: u32,
}

impl MockAdapter {
    /// Fresh adapter with a scripted initial tree (the fixture's full
    /// update) — mirrors what a Level-1 adapter emits on attach.
    pub fn with_fixture(fixture: &Fixture) -> Self {
        let mut me = Self::bare();
        me.updates.push_back(fixture.initial_update());
        me
    }

    pub fn bare() -> Self {
        Self {
            meta: AdapterMeta {
                name: "mock",
                framework: "fixture",
                framework_version: env!("CARGO_PKG_VERSION").to_string(),
                caps: Caps {
                    tree_update: true,
                    logical_snapshot: false,
                    input: true,
                    last_frame: false,
                    render_frame: false,
                    clock: true,
                },
            },
            updates: VecDeque::new(),
            logical: VecDeque::new(),
            frames: VecDeque::new(),
            seq: 0,
            actions: Vec::new(),
            action_results: VecDeque::new(),
            pixels: None,
            clock: ClockState {
                virtual_time: None,
                paused: false,
            },
            repaints_left: 0,
        }
    }

    /// Queue a tree diff to be delivered on the next pump.
    pub fn push_update(&mut self, update: accesskit::TreeUpdate) {
        self.updates.push_back(update);
        self.repaints_left += 1;
    }

    /// Switch the mock to the Level-2 persona (an iced/floem-shaped
    /// adapter: no AccessKit tree of its own, synthesized logical diffs).
    pub fn as_logical(mut self) -> Self {
        self.meta.name = "mock-logical";
        self.meta.caps.logical_snapshot = true;
        self.meta.caps.tree_update = false;
        self
    }

    /// Queue a logical diff (Level-2 persona) for the next pump.
    pub fn push_logical(&mut self, diff: LogicalTreeDiff) {
        self.logical.push_back(diff);
        self.repaints_left += 1;
    }

    /// Queue a presented frame (hash) to be delivered on the next pump.
    pub fn push_frame(&mut self, hash: u64) {
        self.frames.push_back(hash);
        self.repaints_left += 1;
    }

    /// Script the result of the next `act` call.
    pub fn push_action_result(&mut self, result: Result<(u64, String), AdapterError>) {
        self.action_results.push_back(result);
    }

    /// Script RGBA8 pixels for `render_frame`/`last_frame` — also flips
    /// the capability row, so capture-pipeline tests exercise the same
    /// contract real adapters do (§10, M1).
    pub fn with_frame_pixels(mut self, width: u32, height: u32, rgba: Vec<u8>) -> Self {
        self.meta.caps.render_frame = true;
        self.meta.caps.last_frame = true;
        self.pixels = Some((width, height, rgba.into()));
        self
    }

    pub fn actions(&self) -> &[(NodeId, SemAction)] {
        &self.actions
    }

    pub fn pump_count(&self) -> u64 {
        self.seq
    }

    pub fn virtual_time(&self) -> Option<Duration> {
        self.clock.virtual_time
    }
}

impl FrameworkAdapter for MockAdapter {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        self.seq += 1;
        // Repaint requested while scripted work remains.
        let repaint_requested = !self.updates.is_empty()
            || !self.logical.is_empty()
            || !self.frames.is_empty()
            || self.repaints_left > 0;
        // Every pump presents one pass (harness-driven adapters run a full
        // pass per pump).
        let hash = self.frames.pop_front().unwrap_or(0);
        self.repaints_left = self.repaints_left.saturating_sub(1);
        PumpOutcome {
            frames: vec![crate::adapter::FrameStamp {
                seq: self.seq,
                hash,
            }],
            repaint_requested,
            quit_requested: false,
        }
    }

    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        self.updates.pop_front()
    }

    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        self.logical.pop_front()
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        self.actions.push((node, action.clone()));
        match self.action_results.pop_front() {
            Some(Err(e)) => Err(e),
            Some(Ok((frame, detail))) => Ok(ActionOutcome {
                applied_at_frame: frame,
                detail,
            }),
            None => Ok(ActionOutcome {
                applied_at_frame: self.seq,
                detail: default_detail(&action),
            }),
        }
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        match cmd {
            ClockCmd::InstallVirtual => {
                self.clock.virtual_time = Some(Duration::ZERO);
            }
            ClockCmd::FastForward(dt) | ClockCmd::RunFor(dt) => {
                self.clock.virtual_time = Some(
                    self.clock
                        .virtual_time
                        .unwrap_or(Duration::ZERO)
                        .saturating_add(dt),
                );
            }
            ClockCmd::Pause => self.clock.paused = true,
            ClockCmd::Resume => self.clock.paused = false,
        }
        Ok(self.clock)
    }

    fn last_frame(&mut self) -> Option<crate::adapter::FrameRef> {
        self.render_frame(None).ok()
    }

    fn render_frame(
        &mut self,
        size: Option<(u32, u32)>,
    ) -> Result<crate::adapter::FrameRef, AdapterError> {
        let (w, h, pixels) = self.pixels.as_ref().ok_or(AdapterError::Unsupported(
            "render_frame (script pixels via with_frame_pixels)",
        ))?;
        // A size override re-renders at the requested viewport (§6.1) — the
        // mock stretches by repeating (tests use matching sizes).
        let (w, h, pixels) = match size {
            Some((sw, sh)) if (sw, sh) != (*w, *h) => {
                let count = (sw as usize) * (sh as usize) * 4;
                let mut scaled = Vec::with_capacity(count);
                while scaled.len() < count {
                    let take = (count - scaled.len()).min(pixels.len());
                    scaled.extend_from_slice(&pixels[..take]);
                }
                (sw, sh, std::sync::Arc::from(scaled.into_boxed_slice()))
            }
            _ => (*w, *h, std::sync::Arc::clone(pixels)),
        };
        Ok(crate::adapter::FrameRef {
            stamp: crate::adapter::FrameStamp {
                seq: self.seq,
                hash: 0,
            },
            size: (w, h),
            pixels: Some(pixels),
        })
    }
}

fn default_detail(action: &SemAction) -> String {
    match action {
        SemAction::Click => "clicked".to_string(),
        other => other.describe(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_core::test_util::Fixture;

    #[test]
    fn fixture_adapter_delivers_initial_update_on_first_pump() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        let outcome = adapter.pump(FrameDeadline::within(Duration::from_millis(10)));
        assert_eq!(outcome.frames.len(), 1);
        assert!(outcome.repaint_requested, "an update is pending");
        let update = adapter.tree_update().expect("initial update");
        assert!(update.tree.is_some());
    }

    #[test]
    fn quiescence_after_scripted_content_drains() {
        let mut adapter = MockAdapter::bare();
        adapter.push_frame(1);
        let o1 = adapter.pump(FrameDeadline::within(Duration::from_millis(1)));
        assert_eq!(o1.frames.len(), 1);
        assert_eq!(o1.frames[0].hash, 1);
        assert!(o1.repaint_requested, "the scripted frame was pending");
        let o2 = adapter.pump(FrameDeadline::within(Duration::from_millis(1)));
        assert!(o2.is_quiescent(), "drained after the scripted frame");
    }

    #[test]
    fn actions_are_recorded() {
        let mut adapter = MockAdapter::bare();
        let outcome = adapter
            .act(NodeId(2), SemAction::Click)
            .expect("default action result");
        assert_eq!(outcome.detail, "clicked");
        assert_eq!(adapter.actions().len(), 1);
        adapter.push_action_result(Err(AdapterError::Internal("boom".into())));
        let err = adapter.act(NodeId(3), SemAction::Click).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn virtual_clock_advances() {
        let mut adapter = MockAdapter::bare();
        adapter.clock(ClockCmd::InstallVirtual).unwrap();
        let state = adapter
            .clock(ClockCmd::FastForward(Duration::from_secs(30)))
            .unwrap();
        assert_eq!(state.virtual_time, Some(Duration::from_secs(30)));
        assert!(!state.paused);
    }
}
