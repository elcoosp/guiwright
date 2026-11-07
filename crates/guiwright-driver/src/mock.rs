//! The scriptable mock adapter — the TDD workhorse for driver, kit and
//! bench tests (feature `mock`).
//!
//! Model (mirrors harness-driven Level-1 adapters like egui/masonry):
//! every `pump` presents exactly one pass (frame stamp), scripted tree
//! diffs are delivered one per pump, and the app requests repaints while
//! scripted work remains. Tests script the frame/update stream explicitly.

use std::collections::VecDeque;
use std::time::Duration;

use accesskit::{NodeId, TreeId};

use crate::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameworkAdapter, LogicalTreeDiff, PumpOutcome, SemAction, WindowInfo,
};
use guiwright_core::MAIN_WINDOW;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};

/// A scriptable in-process adapter.
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
    /// The runner context handed to seam tasks by `run_in` (§14 state
    /// seams) — the app state the "UI thread" owns.
    state: Option<Box<dyn std::any::Any + Send>>,
    /// Scripted window targets (§16 v0.4 multi-window, M6): the metadata
    /// `windows()` reports. Empty = single-window persona (the default
    /// `main` target).
    windows: Vec<WindowScript>,
    /// Shared script channel (e2e tests): other threads queue tree updates
    /// while the session runs — the next pump delivers them, exactly like
    /// app-side changes arriving mid-session.
    shared: Option<std::sync::Arc<std::sync::Mutex<Vec<accesskit::TreeUpdate>>>>,
}

/// A scripted window target: the handle is the subtree [`TreeId`] the
/// adapter pushes updates for — the same identity the tree scopes with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowScript {
    pub id: TreeId,
    pub title: String,
    pub focused: bool,
}

impl std::fmt::Debug for MockAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockAdapter")
            .field("meta", &self.meta)
            .field("pending_updates", &self.updates.len())
            .field("pending_logical", &self.logical.len())
            .field("pending_frames", &self.frames.len())
            .field("seq", &self.seq)
            .field("actions", &self.actions.len())
            .field("clock", &self.clock)
            .field("has_state", &self.state.is_some())
            .finish_non_exhaustive()
    }
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
            state: None,
            windows: Vec::new(),
            shared: None,
        }
    }

    /// Attach the runner context for state seams (§14): the `&mut dyn Any`
    /// handed to `run_in` tasks. Must be `Send` because the mock may host
    /// remote sessions (the server spawns a session thread).
    pub fn with_state(mut self, state: impl std::any::Any + Send) -> Self {
        self.state = Some(Box::new(state));
        self
    }

    /// The multi-window persona (§16 v0.4, M6): scripts the grafted
    /// two-window fixture (root tree + one subtree per window, delivered
    /// over successive pumps) and the matching `windows()` metadata — the
    /// handles are the fixture's subtree TreeIds, the same identity the
    /// tree scopes resolution with.
    pub fn with_multi_window(fixture: &MultiWindowFixture) -> Self {
        let mut me = Self::bare();
        for update in fixture.initial_updates() {
            me.push_update(update);
        }
        me.windows = vec![
            WindowScript {
                id: fixture.w1,
                title: "Editor".into(),
                focused: true,
            },
            WindowScript {
                id: fixture.w2,
                title: "Preview".into(),
                focused: false,
            },
        ];
        me
    }

    /// Queue an incremental update for ONE window's subtree (§16 v0.4):
    /// per-window diffs ride the same pump stream as single-window ones.
    pub fn push_subtree_update(&mut self, id: TreeId, update: accesskit::TreeUpdate) {
        assert_eq!(
            update.tree_id, id,
            "subtree update must target its window's TreeId"
        );
        self.push_update(update);
    }

    /// Script a focus flip on one window target (reflected in `windows()`).
    pub fn set_window_focus(&mut self, id: TreeId, focused: bool) {
        for w in &mut self.windows {
            if w.id == id {
                w.focused = focused;
            }
        }
    }

    /// Attach a shared script channel: updates pushed into the vec from
    /// any thread are delivered on the next pump (the e2e tests emulate
    /// app-side changes arriving mid-session — window-local diffs land
    /// through the same pump stream).
    pub fn with_shared_scripts(
        mut self,
        scripts: std::sync::Arc<std::sync::Mutex<Vec<accesskit::TreeUpdate>>>,
    ) -> Self {
        self.shared = Some(scripts);
        self
    }

    /// Read back the runner context (test-side assertions).
    pub fn state<T: 'static>(&self) -> Option<&T> {
        self.state.as_ref().and_then(|s| s.downcast_ref::<T>())
    }

    /// Mutate the runner context (test-side scripting).
    pub fn state_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.state.as_mut().and_then(|s| s.downcast_mut::<T>())
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

    /// Window targets (§16 v0.4): the scripted list, or the default single
    /// `main` target for the single-window persona.
    fn windows(&mut self) -> Vec<WindowInfo> {
        if self.windows.is_empty() {
            return vec![WindowInfo {
                handle: MAIN_WINDOW.to_string(),
                title: self.meta.framework.to_string(),
                focused: true,
            }];
        }
        self.windows
            .iter()
            .map(|w| WindowInfo {
                handle: w.id.0.to_string(),
                title: w.title.clone(),
                focused: w.focused,
            })
            .collect()
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // Drain the shared script channel first: queued updates ride this
        // pump's diff stream like any app-side change.
        if let Some(shared) = self.shared.clone() {
            if let Ok(mut queue) = shared.lock() {
                for update in queue.drain(..) {
                    self.updates.push_back(update);
                    self.repaints_left += 1;
                }
            }
        }
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
            ClockCmd::SetTime(t) => self.clock.virtual_time = Some(t),
            ClockCmd::Pause => self.clock.paused = true,
            ClockCmd::Resume => self.clock.paused = false,
            ClockCmd::Probe => {}
        }
        Ok(self.clock)
    }

    fn run_in(&mut self, task: crate::seams::SeamTask) -> Result<String, AdapterError> {
        let mut state = self.state.take().ok_or_else(|| {
            AdapterError::Internal("no runner context attached (with_state missing)".into())
        })?;
        let result = task(state.as_mut_any());
        self.state = Some(state);
        result.map_err(AdapterError::Internal)
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

/// `Box<dyn Any + Send>` → `&mut dyn Any` (the `run_in` context shape).
trait AsMutAny {
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any;
}
impl AsMutAny for Box<dyn std::any::Any + Send> {
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        &mut **self
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

    #[test]
    fn set_time_and_probe_are_lossless() {
        let mut adapter = MockAdapter::bare();
        let state = adapter
            .clock(ClockCmd::SetTime(Duration::from_millis(1_500)))
            .unwrap();
        assert_eq!(state.virtual_time, Some(Duration::from_millis(1_500)));
        let probed = adapter.clock(ClockCmd::Probe).unwrap();
        assert_eq!(probed, state, "probe mutates nothing");
        adapter.clock(ClockCmd::Pause).unwrap();
        assert!(adapter.clock(ClockCmd::Probe).unwrap().paused);
    }

    /// §14 state seams: the seam writes app state through the runner
    /// context (`run_in`), never through the session.
    #[test]
    fn run_in_writes_the_runner_context() {
        #[derive(Debug, Default)]
        struct AppDb {
            rows: usize,
            fixture: Option<String>,
        }
        let mut adapter = MockAdapter::bare().with_state(AppDb {
            rows: 12,
            fixture: None,
        });
        let task: crate::seams::SeamTask = Box::new(|ctx: &mut dyn std::any::Any| {
            let db = ctx
                .downcast_mut::<AppDb>()
                .ok_or("wrong runner context type")?;
            db.rows = 0;
            db.fixture = Some("seed-42".into());
            Ok("db reset to seed-42".into())
        });
        let detail = adapter.run_in(task).expect("seam executed");
        assert_eq!(detail, "db reset to seed-42");
        let db = adapter.state::<AppDb>().expect("context preserved");
        assert_eq!(db.rows, 0);
        assert_eq!(db.fixture.as_deref(), Some("seed-42"));
    }

    #[test]
    fn run_in_without_context_is_loud() {
        let mut adapter = MockAdapter::bare();
        let task: crate::seams::SeamTask = Box::new(|_ctx: &mut dyn std::any::Any| Ok("x".into()));
        let err = adapter.run_in(task).unwrap_err();
        assert!(err.to_string().contains("with_state"), "{err}");
    }
}
