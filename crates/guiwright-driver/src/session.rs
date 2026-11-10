//! The session: adapter + tree service + event-driven wait loop + action
//! scheduler (architecture §5.1 "driver thread(s)" and §15.3 zero-polling).
//!
//! The session owns the adapter on one thread (the UI thread in embedded
//! mode). It pumps the adapter, feeds every resulting event (tree diffs,
//! presented frames) into the active waiter, and performs actions with
//! full actionability checking (§8.3). Checks run **exactly when the world
//! changes** — between frames the session does no CPU work.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::NodeId;
use guiwright_core::actionability::{Check, action_checks};
use guiwright_core::expect::DEFAULT_TIMEOUT;
use guiwright_core::tree::{Resolved, TreeService};
use guiwright_core::wait::{ChecksWaiter, WaitOutcome, Waiter, WaiterEvent};
use guiwright_core::{CoreError, NodeKey, Selector};

use crate::adapter::{ActionOutcome, FrameDeadline, FrameworkAdapter, InputEvent, SemAction};
use crate::clock::{ClockReport, ClockService, LedgerCommand};
use crate::gesture::{Gesture, codegen_call, codegen_event_call, codegen_line, synthesize};
use crate::seams::SeamRegistry;
use guiwright_trace::video::{VideoOptions, VideoRecorder, VideoSummary};

/// Options for actions (§8.3 "Timeout & retries" + force mode).
#[derive(Clone, Debug)]
pub struct ActionOpts {
    pub timeout: Duration,
    /// Skip stability + hit-test checks (Playwright `force: true`).
    pub force: bool,
}

impl Default for ActionOpts {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            force: false,
        }
    }
}

/// Result of a synthesized gesture (§9, M7) — the `GestureDone` reply's
/// payload in embedded form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GestureOutcome {
    /// The frame the batch was applied at (rendezvous, §15.4).
    pub applied_at_frame: u64,
    /// How many raw events the synthesis produced.
    pub events: usize,
    /// Human-readable summary (positions, key names, event count).
    pub detail: String,
}

/// The `GestureDone` detail line: positions and event count, matching the
/// golden-vector wording ("clicked at (12.5, 30) — 3 events").
fn gesture_detail(gesture: &Gesture, events: usize) -> String {
    let n = if events == 1 {
        "1 event".to_string()
    } else {
        format!("{events} events")
    };
    match gesture {
        Gesture::Click { x, y, count: 1, .. } => format!("clicked at ({x}, {y}) — {n}"),
        Gesture::Click { x, y, count: 2, .. } => format!("double-clicked at ({x}, {y}) — {n}"),
        Gesture::Click { x, y, count, .. } => {
            format!("clicked ×{count} at ({x}, {y}) — {n}")
        }
        Gesture::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            ..
        } => format!("dragged ({from_x}, {from_y}) → ({to_x}, {to_y}) — {n}"),
        Gesture::Wheel { dx, dy } => format!("wheeled ({dx}, {dy}) — {n}"),
        Gesture::Key { key, modifiers } if modifiers.is_empty() => {
            format!("pressed {key} — {n}")
        }
        Gesture::Key { key, modifiers } => {
            format!("pressed {}+{key} — {n}", modifiers.join("+"))
        }
        Gesture::Type { text } => format!("typed {} chars — {n}", text.chars().count()),
    }
}

/// What one pump produced (for traces and tests).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpSummary {
    pub frames_presented: usize,
    pub tree_changed: bool,
    pub quiescent: bool,
}

/// The outcome of a completed wait: either a resolved node or
/// satisfied-by-absence (only possible for pure-negation assertions).
#[derive(Clone, Debug)]
pub enum WaitResolved {
    Node(Resolved),
    /// `expect(...).to_be_hidden()` on a node that never existed
    /// (Playwright parity). No node to act on.
    Absent,
}

impl WaitResolved {
    /// Unwrap for action paths — absence is an error there.
    pub fn into_node(self) -> Result<Resolved, CoreError> {
        match self {
            WaitResolved::Node(r) => Ok(r),
            WaitResolved::Absent => Err(CoreError::Other(
                "action target vanished before the action could run".into(),
            )),
        }
    }
}

/// Session state machine around an adapter (§5.1).
pub struct Session<A: FrameworkAdapter> {
    adapter: A,
    tree: TreeService,
    tree_seq: u64,
    frame_seq: u64,
    /// Last frame seq fed to a waiter — events are fed exactly once.
    fed_frame_seq: u64,
    /// Virtual clock service (§14): stepping plans + `clock.bin` ledger.
    clock_service: ClockService,
    /// App-registered state seams (§14 mock domain); executed via
    /// `adapter.run_in` inside the runner context.
    seams: SeamRegistry,
    /// Mock-domain overrides: seam id → canned response (`None` = run the
    /// real hook). Registered via GWP `Mock::InstallSeam`.
    seam_overrides: BTreeMap<String, Option<String>>,
    /// The seam `reset_state(scope)` runs (§14 fixture parity).
    reset_seam: String,
    /// Video recording service (§10, M3): present-frame capture riding
    /// the pump loop. `None` = not recording.
    video: Option<VideoRecorder>,
    /// Remote codegen recorder (§13, M4): active while a GWP client has
    /// issued `Inspect::StartRecording`. Embedded sessions never touch it
    /// — the kit records through its own locator taps (`App::codegen`).
    codegen: Option<guiwright_inspect::codegen::CodegenRecorder>,
}

impl<A: FrameworkAdapter> Session<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            tree: TreeService::new(),
            tree_seq: 0,
            frame_seq: 0,
            fed_frame_seq: 0,
            clock_service: ClockService::new(),
            seams: SeamRegistry::new(),
            seam_overrides: BTreeMap::new(),
            reset_seam: "reset_state".into(),
            video: None,
            codegen: None,
        }
    }

    pub fn tree(&self) -> &TreeService {
        &self.tree
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    /// Pump the adapter once and integrate the outcome: tree diffs feed the
    /// tree service, frame stamps feed stability bookkeeping. This is the
    /// frame-boundary rendezvous (§15.4): **one rendezvous per action**,
    /// no "spin the loop N times" heuristics.
    pub fn pump(&mut self) -> PumpSummary {
        // Pump the framework loop with a short frame deadline; the session
        // re-checks outer deadlines between pumps.
        let outcome = self
            .adapter
            .pump(FrameDeadline::within(Duration::from_millis(30)));
        let mut summary = PumpSummary::default();

        if let Some(update) = self.adapter.tree_update() {
            let _diff = self.tree.apply_update(update);
            self.tree_seq += 1;
            summary.tree_changed = true;
        }
        if let Some(diff) = self.adapter.logical_snapshot() {
            // Level-2 adapters synthesize an AccessKit-shaped update
            // (§6.1): selectors, actionability and expect run unchanged on
            // the consumer tree; actions come back with the adapter-minted
            // NodeIds (session.rs `node_id`).
            let _diff = self.tree.apply_update(diff.update);
            for (node, test_id) in diff.test_ids {
                self.tree.register_test_id(
                    guiwright_core::NodeKey::AccessKit {
                        tree: accesskit::TreeId::ROOT,
                        node: NodeId(node),
                    },
                    test_id,
                );
            }
            self.tree_seq += 1;
            summary.tree_changed = true;
        }

        for stamp in &outcome.frames {
            self.frame_seq = self.frame_seq.max(stamp.seq);
            let _info = self.tree.frame_presented(stamp.hash);
            summary.frames_presented += 1;
        }

        // Video capture (§10, M3): every presented frame is offered to the
        // recorder — hash-equality dedup drops static spans, the ring is
        // bounded, and encoding happens on the writer thread (§15.4: the
        // UI thread only hands over an `Arc` clone).
        if self.video.is_some() {
            for stamp in &outcome.frames {
                let frame = self.adapter.last_frame();
                if let (Some(rec), Some(frame)) = (self.video.as_mut(), frame) {
                    rec.push_frame(
                        stamp.seq,
                        stamp.hash,
                        frame.size.0,
                        frame.size.1,
                        frame.pixels.as_ref(),
                    );
                }
            }
        }

        summary.quiescent = outcome.is_quiescent();
        summary
    }

    // --- Video service (§10, M3) ------------------------------------------

    /// Start recording presented frames into `dir` (§10). Requires the
    /// adapter's §6.2 pixel capability row (`last_frame` or
    /// `render_frame`); refuses when a recording is already running.
    /// `fps = 0` selects the driver default (25).
    pub fn video_start_in(&mut self, dir: &Path, fps: u32) -> Result<(), CoreError> {
        let caps = self.adapter.meta().caps;
        if !caps.last_frame && !caps.render_frame {
            return Err(CoreError::Unsupported(
                "video (adapter exposes no pixel path, §6.2)",
            ));
        }
        if self.video.is_some() {
            return Err(CoreError::Other(
                "a video recording is already running".into(),
            ));
        }
        let path = dir.join(format!(
            "guiwright-video-{}-{}.webm",
            std::process::id(),
            guiwright_trace::video::epoch_ms()
        ));
        let recorder = VideoRecorder::start(path, &VideoOptions::new(fps))
            .map_err(|e| CoreError::Other(format!("video start: {e}")))?;
        self.video = Some(recorder);
        Ok(())
    }

    /// Start recording into the default directory (`GUIWRIGHT_VIDEO_DIR`
    /// or the OS temp dir) — the remote-mode default, since the client
    /// learns the finalized path from the `VideoFile` reply (§7).
    pub fn video_start(&mut self, fps: u32) -> Result<(), CoreError> {
        let dir = std::env::var_os("GUIWRIGHT_VIDEO_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        self.video_start_in(&dir, fps)
    }

    /// Stop and finalize the recording; one final pump lands the last
    /// requested frame before the encoder closes (§15.4). Idempotence via
    /// error: stopping twice is an error, mirroring `Clock` domain rules.
    pub fn video_stop(&mut self) -> Result<VideoSummary, CoreError> {
        if self.video.is_none() {
            return Err(CoreError::Other("no video recording is running".into()));
        }
        self.pump();
        let recorder = self.video.take().expect("checked above");
        recorder
            .stop()
            .map_err(|e| CoreError::Other(format!("video stop: {e}")))
    }

    /// Is a recording currently running?
    pub fn recording(&self) -> bool {
        self.video.is_some()
    }

    // --- Remote codegen (§13, M4) -----------------------------------------

    /// Start the interaction → kit-code recorder (§7 `Inspect::StartRecording`).
    /// Refuses while a recording is already active (clean `Unsupported`-style
    /// error, mirroring the video domain's idempotence rules).
    pub fn codegen_start(&mut self) -> Result<(), CoreError> {
        if self.codegen.is_some() {
            return Err(CoreError::Other(
                "codegen recording is already active".into(),
            ));
        }
        self.codegen = Some(guiwright_inspect::codegen::CodegenRecorder::new());
        Ok(())
    }

    /// Stop the recorder and fetch the generated kit code (§7
    /// `Inspect::StopRecording` → `Codegen { code, warnings }`).
    pub fn codegen_stop(&mut self) -> Result<(String, Vec<String>), CoreError> {
        let recorder = self
            .codegen
            .take()
            .ok_or_else(|| CoreError::Other("no codegen recording is active".into()))?;
        Ok(recorder.finish())
    }

    /// Is a codegen recording currently active?
    pub fn codegen_active(&self) -> bool {
        self.codegen.is_some()
    }

    /// Record one applied GWP action into the codegen recorder (§13
    /// "captured at the adapter tap"): facts come from the session's tree
    /// service, the §13 cost model picks the minimal stable selector.
    /// `Scroll` is a raw-input gesture (§9) with no target node — the
    /// gesture vocabulary records through `perform_gesture`/`send_input`
    /// instead (the §13-over-§9 path).
    ///
    /// Called with the *pre-action* tree state: the node is exactly what
    /// the client targeted, before the action's own side effects land.
    fn codegen_action_facts(
        &self,
        key: &NodeKey,
        action: &SemAction,
    ) -> Option<(
        &'static str,
        Option<String>,
        guiwright_inspect::codegen::NodeFacts,
    )> {
        self.codegen.as_ref()?;
        // (action-name, argument) in the kit's locator vocabulary.
        let (kind, argument): (&str, Option<String>) = match action {
            SemAction::Click => ("click", None),
            SemAction::DoubleClick => ("dblclick", None),
            SemAction::SetValue { text } => ("fill", Some(text.clone())),
            SemAction::Focus => ("focus", None),
            SemAction::Blur => ("blur", None),
            SemAction::Hover => ("hover", None),
            SemAction::Press { key, .. } => ("press", Some(key.clone())),
            SemAction::Scroll { .. } => return None,
            SemAction::Check => ("check", None),
            SemAction::Uncheck => ("uncheck", None),
        };
        let node = self.tree.node(key);
        let facts = guiwright_inspect::codegen::NodeFacts {
            key: *key,
            role: node.as_ref().map(|n| format!("{:?}", n.role())),
            name: node.as_ref().and_then(|n| n.label()),
            label: node.as_ref().and_then(|n| n.label()),
            value: node.as_ref().and_then(|n| n.value()),
            test_id: self.tree.test_id_of(key).map(str::to_string),
            bounds: None,
        };
        Some((kind, argument, facts))
    }

    /// Land a prepared codegen line (see [`Session::codegen_action_facts`]).
    fn codegen_record(
        &mut self,
        kind: &str,
        argument: Option<String>,
        facts: &guiwright_inspect::codegen::NodeFacts,
    ) {
        let Some(mut recorder) = self.codegen.take() else {
            return;
        };
        if let Err(err) = recorder.record(kind, argument.as_deref(), facts, &self.tree) {
            tracing::debug!(%err, action = %kind, "codegen: no stable selector");
        }
        self.codegen = Some(recorder);
    }

    /// Pump until the app is quiescent or the deadline hits (§6.1 "pump
    /// until quiescent"). Used at session start and after clock jumps.
    pub fn settle(&mut self, deadline: Instant) -> PumpSummary {
        let mut total = PumpSummary::default();
        loop {
            if Instant::now() >= deadline {
                break;
            }
            let s = self.pump();
            total.frames_presented += s.frames_presented;
            total.tree_changed |= s.tree_changed;
            total.quiescent = s.quiescent;
            if s.quiescent {
                break;
            }
        }
        total
    }

    /// Resolve a selector against the current tree (strict mode). A pump is
    /// performed first so freshly-queued diffs are visible — resolve stays
    /// consistent with the adapter's latest state.
    pub fn resolve(&mut self, selector: &Selector) -> Result<Resolved, CoreError> {
        self.pump();
        self.tree.resolve(selector)
    }

    /// Resolve returning all matches (no strict check).
    pub fn resolve_all(&mut self, selector: &Selector) -> Result<Vec<Resolved>, CoreError> {
        self.pump();
        self.tree.resolve_all(selector)
    }

    // --- Window targets (§16 v0.4 multi-window, M6) -----------------------

    /// The session's window targets (§16 v0.4): adapter-reported metadata
    /// (handle / title / focus). Single-window adapters report exactly one
    /// `main` target.
    pub fn windows(&mut self) -> Vec<crate::adapter::WindowInfo> {
        self.adapter.windows()
    }

    /// Validate a window handle and map it to its scope root (§16 v0.4
    /// "targets per window"): `Ok(None)` = the whole tree (the `main`
    /// target of a graft-free app, or an empty handle); `Ok(Some(root))` =
    /// resolve only inside that window's subtree; `Err(UnknownWindow)` =
    /// the handle is not a live target. The **tree** is the source of
    /// truth — an adapter metadata entry without tree content is not a
    /// usable target.
    pub fn window_scope(&self, handle: &str) -> Result<Option<NodeKey>, CoreError> {
        self.tree.window_scope(handle)
    }

    /// Narrow every resolution path (resolve, waiters, `expect`) to a
    /// window subtree until [`Session::clear_resolve_scope`]. The GWP
    /// server sets this around a scoped request and clears it after — the
    /// rendezvous model keeps the session single-threaded, so no request
    /// can observe a half-applied scope.
    pub fn set_resolve_scope(&mut self, scope: Option<NodeKey>) {
        self.tree.set_scope(scope);
    }

    /// Drop the window scope (back to the unscoped, whole-tree surface).
    pub fn clear_resolve_scope(&mut self) {
        self.tree.set_scope(None);
    }

    // --- Raw input & gestures (§9 pipeline, M7) -------------------------

    /// The session's default window target (§9): the first target the
    /// adapter reports — a single-window adapter's `main`, a multi-window
    /// adapter's primary window. Unscoped injections land here.
    pub fn default_window(&mut self) -> Result<String, CoreError> {
        self.windows()
            .into_iter()
            .next()
            .map(|w| w.handle)
            .ok_or_else(|| CoreError::Other("session has no window targets".into()))
    }

    /// Resolve the input target (§9 coordinates are window-relative):
    /// an explicit handle is validated against the tree (the source of
    /// truth, §16 v0.4); `None` (or empty) selects the session default.
    fn input_target(&mut self, window: Option<&str>) -> Result<String, CoreError> {
        match window {
            Some(handle) if !handle.is_empty() => {
                self.window_scope(handle)?;
                Ok(handle.to_string())
            }
            _ => self.default_window(),
        }
    }

    /// Inject a batch of raw input events into a window's pipeline (§9).
    /// Coordinates are logical px relative to that window; the batch
    /// applies inside the next frame (one rendezvous pump, §15.4) and the
    /// returned frame seq is the `InputQueued` reply's. An empty batch is
    /// an honest no-op that still pumps.
    pub fn send_input(
        &mut self,
        events: Vec<InputEvent>,
        window: Option<&str>,
    ) -> Result<u64, CoreError> {
        let target = self.input_target(window)?;
        // §13 over §9: raw pointer primitives record while a codegen
        // recording runs (keyboard primitives ride the gesture vocabulary,
        // see `codegen_event_call`). Rendered before the batch moves into
        // the adapter, landed after the rendezvous pump confirms it.
        let recorded: Vec<_> = if self.codegen.is_some() {
            events.iter().filter_map(codegen_event_call).collect()
        } else {
            Vec::new()
        };
        self.inject(events, &target)?;
        self.pump();
        for (surface, call) in recorded {
            let line = codegen_line(surface, &call, window);
            self.codegen_input_line(line, None);
        }
        Ok(self.frame_seq)
    }

    /// Synthesize and apply a high-level gesture (§9): the adapter's
    /// pacing table drives the synthesis (§18 "per-adapter gesture pacing
    /// tables"), the batch rides the same per-window injection as
    /// [`Session::send_input`], one rendezvous pump applies it.
    pub fn perform_gesture(
        &mut self,
        gesture: Gesture,
        window: Option<&str>,
    ) -> Result<GestureOutcome, CoreError> {
        let pacing = self.adapter.gesture_pacing();
        let events = synthesize(&gesture, &pacing);
        if events.is_empty() {
            return Err(CoreError::Other(
                "gesture synthesizes no events (click count 0?)".into(),
            ));
        }
        let target = self.input_target(window)?;
        let count = events.len();
        self.inject(events, &target)?;
        self.pump();
        // §13 over §9: the applied gesture records as kit code (the
        // promise the semantic-action path carried since M0 — raw input is
        // recorded in the gesture milestone). Recorded only after the
        // batch actually applied; a failed gesture records nothing.
        if self.codegen.is_some() {
            let (surface, call, warning) = codegen_call(&gesture);
            let line = codegen_line(surface, &call, window);
            self.codegen_input_line(line, warning);
        }
        Ok(GestureOutcome {
            applied_at_frame: self.frame_seq,
            events: count,
            detail: gesture_detail(&gesture, count),
        })
    }

    /// Land one raw-input codegen line (§13): pre-rendered by the
    /// [`crate::gesture`] renderer, no selector to validate.
    fn codegen_input_line(&mut self, line: String, warning: Option<String>) {
        let Some(mut recorder) = self.codegen.take() else {
            return;
        };
        if let Some(warning) = warning {
            recorder.warn(warning);
        }
        recorder.record_raw(line);
        self.codegen = Some(recorder);
    }

    /// Forward a synthesized/raw batch through the adapter's window-aware
    /// hook (§9 fidelity rule: synthetic events enter at the same choke
    /// point as OS events).
    fn inject(&mut self, events: Vec<InputEvent>, target: &str) -> Result<(), CoreError> {
        for ev in &events {
            self.adapter
                .input_window(target, ev.clone())
                .map_err(|e| CoreError::Other(e.to_string()))?;
        }
        Ok(())
    }

    /// The event-driven wait loop (§15.3): pump → feed events → re-check.
    ///
    /// Waiters only progress on events; each pump feeds at most one event
    /// (the newest world change). Nothing is fed twice, and when the pump
    /// produced nothing new the waiter is not consulted at all — that is
    /// the zero-polling contract in practice.
    pub fn wait_for(
        &mut self,
        selector: &Selector,
        checks: Vec<Check>,
        deadline: Instant,
    ) -> Result<WaitResolved, CoreError> {
        let started = Instant::now();
        let configured = deadline.saturating_duration_since(started);
        let mut waiter = ChecksWaiter::new(selector.clone(), checks);

        // Initial evaluation against the current tree — many waits finish
        // without pumping at all.
        match waiter.on_event(WaiterEvent::Initial, &self.tree) {
            WaitOutcome::Satisfied(resolved) => {
                return Ok(resolved.map_or(WaitResolved::Absent, WaitResolved::Node));
            }
            WaitOutcome::Failed(failure) => {
                return Err(CoreError::Other(format!(
                    "wait for `{}` failed: {} ({})",
                    selector.render(),
                    failure.check,
                    failure.reason
                )));
            }
            WaitOutcome::Pending => {}
        }

        loop {
            if Instant::now() >= deadline {
                return Err(CoreError::Timeout {
                    operation: waiter.describe(),
                    timeout: configured,
                    last: Box::new(self.tree.summary()),
                });
            }
            let last_diff = self.tree.last_diff().clone();
            let last_frame = self.tree.last_frame();
            self.pump();

            // Feed the newest event produced by this pump, if any.
            let diff = self.tree.last_diff();
            let event = if !diff.is_empty() && diff != &last_diff {
                Some(WaiterEvent::TreeChanged(diff))
            } else if let Some(info) = self.tree.last_frame() {
                let frame_is_new = last_frame.is_none_or(|prev| info.seq > prev.seq);
                frame_is_new.then_some(WaiterEvent::FramePresented {
                    seq: info.seq,
                    hash: info.hash,
                })
            } else {
                None
            };

            if let Some(ev) = event {
                match waiter.on_event(ev, &self.tree) {
                    WaitOutcome::Satisfied(resolved) => {
                        self.fed_frame_seq = self.frame_seq;
                        return Ok(resolved.map_or(WaitResolved::Absent, WaitResolved::Node));
                    }
                    WaitOutcome::Failed(failure) => {
                        return Err(CoreError::Other(format!(
                            "wait for `{}` failed: {} ({})",
                            selector.render(),
                            failure.check,
                            failure.reason
                        )));
                    }
                    WaitOutcome::Pending => {}
                }
            }
        }
    }

    /// expect()-style assertion with retry-until-timeout semantics (§16).
    pub fn expect(&mut self, selector: Selector) -> ExpectBuilder<'_, A> {
        ExpectBuilder {
            session: self,
            selector,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Run a semantic action with full actionability checking (§8.3–§8.4).
    /// Force mode skips stability + hit-test checks.
    pub fn perform_action(
        &mut self,
        selector: &Selector,
        action: SemAction,
        opts: &ActionOpts,
    ) -> Result<ActionOutcome, CoreError> {
        let deadline = Instant::now() + opts.timeout;
        let checks = action_checks(opts.force);
        let resolved = self.wait_for(selector, checks, deadline)?.into_node()?;
        // Generation guard (§8.2): the identity must still be live at act
        // time — no ABA through id recycling.
        self.tree.check_generation(&resolved)?;

        // §8.4: hover/move/drag/scroll are NOT semantic actions — they ride
        // the real input pipeline with coordinates from the resolved bounds.
        let outcome = match (&action, resolved.bounds) {
            (SemAction::Hover, Some(rect)) => {
                self.adapter
                    .input(InputEvent::PointerMove {
                        x: (rect.x0 + rect.x1) / 2.0,
                        y: (rect.y0 + rect.y1) / 2.0,
                    })
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                ActionOutcome {
                    applied_at_frame: self.frame_seq,
                    detail: "hovered".into(),
                }
            }
            (SemAction::Scroll { dx, dy }, Some(_rect)) => {
                self.adapter
                    .input(InputEvent::Wheel { dx: *dx, dy: *dy })
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                ActionOutcome {
                    applied_at_frame: self.frame_seq,
                    detail: format!("scrolled ({dx},{dy})"),
                }
            }
            _ => self
                .adapter
                .act(node_id(&resolved.key), action)
                .map_err(|e| CoreError::Other(e.to_string()))?,
        };
        // Apply the action inside the next frame (rendezvous, §15.4): one
        // pump, then done — never spin.
        self.pump();
        Ok(outcome)
    }

    /// Apply a pre-resolved action (the GWP `Action` path, §7): the caller
    /// already holds the node handle + generation from a `Resolve`; the
    /// session validates the generation and applies inside the next frame.
    pub fn apply_action(
        &mut self,
        key: NodeKey,
        action: SemAction,
        force: bool,
        timeout_ms: u64,
    ) -> Result<ActionOutcome, CoreError> {
        let _ = (force, timeout_ms);
        // Remote codegen (§13, M4): facts come from the *pre-action* tree —
        // the node is exactly what the client targeted before the action's
        // own side effects land. The embedded path (`perform_action`)
        // records through the kit's own locator taps instead — no double
        // recording.
        let codegen_pending = self.codegen_action_facts(&key, &action);
        let outcome = self
            .adapter
            .act(node_id(&key), action)
            .map_err(|e| CoreError::Other(e.to_string()))?;
        // Frame-boundary rendezvous (§15.4): one pump so the action is
        // applied at a frame before the reply is sent.
        self.pump();
        if let Some((kind, argument, facts)) = codegen_pending {
            self.codegen_record(kind, argument, &facts);
        }
        Ok(outcome)
    }

    /// `locator.click()`.
    pub fn click(&mut self, selector: &Selector) -> Result<ActionOutcome, CoreError> {
        self.perform_action(selector, SemAction::Click, &ActionOpts::default())
    }

    /// `locator.fill(text)`.
    pub fn fill(&mut self, selector: &Selector, text: &str) -> Result<ActionOutcome, CoreError> {
        self.perform_action(
            selector,
            SemAction::SetValue {
                text: text.to_string(),
            },
            &ActionOpts::default(),
        )
    }

    /// `locator.hover()`.
    pub fn hover(&mut self, selector: &Selector) -> Result<ActionOutcome, CoreError> {
        self.perform_action(selector, SemAction::Hover, &ActionOpts::default())
    }

    /// `app.clock().fast_forward(dt)` (§14) — through the clock service:
    /// virtual time advances in frame-sized steps until drained, settling
    /// between steps (timers + animations + subscriptions).
    pub fn clock_fast_forward(&mut self, dt: Duration) -> Result<(), CoreError> {
        self.clock(crate::adapter::ClockCmd::FastForward(dt))
            .map(|_| ())
    }

    /// The full clock surface (§14): install / fast-forward / run-for /
    /// pause / resume / set-time / probe, recorded into the ledger so
    /// traces replay timing (§12 `clock.bin`).
    pub fn clock(&mut self, cmd: crate::adapter::ClockCmd) -> Result<ClockReport, CoreError> {
        use crate::adapter::ClockCmd as Cmd;
        let report = match cmd {
            Cmd::InstallVirtual => {
                let state = self
                    .adapter
                    .clock(Cmd::InstallVirtual)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                self.clock_service
                    .record(LedgerCommand::InstallVirtual, state, self.frame_seq, 0);
                ClockReport { state, steps: 0 }
            }
            Cmd::FastForward(dt) => {
                // §14: advance in frame-sized steps until drained; each
                // step pumps a frame so timers/animations settle.
                let plan = ClockService::plan_steps(dt);
                let mut state = self
                    .adapter
                    .clock(Cmd::Probe)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                for step in &plan {
                    state = self
                        .adapter
                        .clock(Cmd::FastForward(*step))
                        .map_err(|e| CoreError::Other(e.to_string()))?;
                    self.pump();
                    self.clock_service
                        .record(LedgerCommand::FastForward, state, self.frame_seq, 1);
                }
                ClockReport {
                    state,
                    steps: plan.len() as u64,
                }
            }
            Cmd::RunFor(dt) => {
                if self.clock_service.paused() {
                    return Err(CoreError::Other(
                        "clock is paused: run_for cannot pace real time (§14)".into(),
                    ));
                }
                let state = self
                    .adapter
                    .clock(Cmd::RunFor(dt))
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                self.pump();
                self.clock_service
                    .record(LedgerCommand::RunFor, state, self.frame_seq, 0);
                ClockReport { state, steps: 0 }
            }
            Cmd::Pause => {
                let state = self
                    .adapter
                    .clock(Cmd::Pause)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                self.clock_service
                    .record(LedgerCommand::Pause, state, self.frame_seq, 0);
                ClockReport { state, steps: 0 }
            }
            Cmd::Resume => {
                let state = self
                    .adapter
                    .clock(Cmd::Resume)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                self.clock_service
                    .record(LedgerCommand::Resume, state, self.frame_seq, 0);
                ClockReport { state, steps: 0 }
            }
            Cmd::SetTime(t) => {
                let state = self
                    .adapter
                    .clock(Cmd::SetTime(t))
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                self.clock_service
                    .record(LedgerCommand::SetTime, state, self.frame_seq, 0);
                ClockReport { state, steps: 0 }
            }
            Cmd::Probe => {
                let state = self
                    .adapter
                    .clock(Cmd::Probe)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                ClockReport { state, steps: 0 }
            }
        };
        Ok(report)
    }

    /// The clock service ledger (§12 `clock.bin` payload for the trace).
    pub fn clock_ledger(&self) -> &[crate::clock::ClockLedgerEntry] {
        self.clock_service.ledger()
    }

    // ------------------------------------------------------------------
    // State seams (§14 mock domain)

    /// Register app-boot seams (the `#[seam]` + `register_seams!` output).
    pub fn register_seams(&mut self, registry: &SeamRegistry) {
        self.seams.merge(registry);
    }

    /// The registered seam ids (diagnostics; sorted).
    pub fn seam_ids(&self) -> Vec<&str> {
        self.seams.ids()
    }

    /// Choose which registered seam `reset_state(scope)` runs (default
    /// `"reset_state"`).
    pub fn with_reset_seam(&mut self, id: impl Into<String>) -> &mut Self {
        self.reset_seam = id.into();
        self
    }

    /// Mock-domain override (§7 `Mock::InstallSeam`): preset a canned
    /// response for a seam. `None` = clear the canned response so the real
    /// hook runs again.
    pub fn set_seam_response(&mut self, id: impl Into<String>, response: Option<String>) {
        self.seam_overrides.insert(id.into(), response);
    }

    /// Drop the override for a seam (§7 `Mock::Clear`).
    pub fn clear_seam(&mut self, id: &str) {
        self.seam_overrides.remove(id);
    }

    /// Run a registered seam once: the hook executes **inside the
    /// adapter's runner context** via `run_in` (§14 "state writes need the
    /// runner's context"), then the session pumps a frame so any UI effect
    /// settles. Canned overrides win over the real hook.
    pub fn run_seam(&mut self, id: &str, scope: &str) -> Result<String, CoreError> {
        if let Some(Some(canned)) = self.seam_overrides.get(id) {
            return Ok(canned.clone());
        }
        let hook = self.seams.get(id).ok_or_else(|| {
            CoreError::Other(format!(
                "no seam registered under `{id}` (registered: {:?})",
                self.seams.ids()
            ))
        })?;
        let task = crate::seams::compile_task(hook, scope.to_string());
        let detail = self
            .adapter
            .run_in(task)
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.pump();
        Ok(detail)
    }

    /// Reset app state to a fixture scope (§14 "fixture parity with
    /// Playwright contexts"). Runs the configured reset seam.
    pub fn reset_state(&mut self, scope: &str) -> Result<String, CoreError> {
        let id = self.reset_seam.clone();
        self.run_seam(&id, scope)
    }

    /// Register an explicit test id for a node (`getByTestId`, §8.1) —
    /// the kit exposes it for apps that mint ids outside the adapter.
    pub fn register_test_id(&mut self, key: NodeKey, id: impl Into<String>) {
        self.tree.register_test_id(key, id);
    }

    pub fn tree_seq(&self) -> u64 {
        self.tree_seq
    }

    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }
}

fn node_id(key: &NodeKey) -> NodeId {
    match key {
        NodeKey::AccessKit { node, .. } => *node,
        NodeKey::Logical { adapter, id } => {
            // Level-2 identities are delivered to adapters by their numeric
            // handle; AccessKit NodeId carries it losslessly for the
            // adapter-side translation (§6.1 NodeKey normalization).
            tracing::debug!(adapter, id, "logical identity routed through NodeId");
            NodeId(*id)
        }
    }
}

/// Builder for retry-until-timeout assertions on a session (§16).
pub struct ExpectBuilder<'s, A: FrameworkAdapter> {
    session: &'s mut Session<A>,
    selector: Selector,
    timeout: Duration,
}

impl<'s, A: FrameworkAdapter> ExpectBuilder<'s, A> {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn run(self, checks: Vec<Check>, name: &str) -> Result<(), CoreError> {
        let deadline = Instant::now() + self.timeout;
        self.session
            .wait_for(&self.selector, checks, deadline)
            .map(|_| ())
            .map_err(|e| match e {
                CoreError::Timeout { last, .. } => CoreError::Timeout {
                    operation: name.to_string(),
                    timeout: self.timeout,
                    last,
                },
                other => other,
            })
    }

    pub fn to_be_visible(self) -> Result<(), CoreError> {
        self.run(vec![Check::Visible], "visible")
    }

    pub fn to_be_hidden(self) -> Result<(), CoreError> {
        self.run(vec![Check::Hidden], "hidden")
    }

    pub fn to_be_enabled(self) -> Result<(), CoreError> {
        self.run(vec![Check::Enabled], "enabled")
    }

    pub fn to_be_disabled(self) -> Result<(), CoreError> {
        self.run(vec![Check::Disabled], "disabled")
    }

    pub fn to_be_focused(self) -> Result<(), CoreError> {
        self.run(vec![Check::Focused], "focused")
    }

    pub fn to_have_text(self, text: impl Into<guiwright_core::TextMatch>) -> Result<(), CoreError> {
        self.run(vec![Check::HasText(text.into())], "text")
    }

    pub fn to_have_value(
        self,
        value: impl Into<guiwright_core::TextMatch>,
    ) -> Result<(), CoreError> {
        self.run(vec![Check::HasText(value.into())], "value")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAdapter;
    use guiwright_core::test_util::Fixture;

    fn save_selector() -> Selector {
        Selector::role_named(accesskit::Role::Button, "Save")
    }

    /// A settled mock with two presented frames (stability baseline +
    /// confirmation), mirroring what a real adapter streams while idle.
    fn settled_session(fixture: &Fixture) -> Session<MockAdapter> {
        let mut adapter = MockAdapter::with_fixture(fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        session.settle(Instant::now() + Duration::from_secs(1));
        session
    }

    #[test]
    fn click_performs_actionability_then_act() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let outcome = session.click(&save_selector()).expect("click");
        assert_eq!(outcome.detail, "clicked");
        let actions = session.adapter().actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].1, SemAction::Click);
        assert_eq!(actions[0].0, fixture.save_button, "acts on the Save node");
    }

    #[test]
    fn click_waits_for_late_appearing_button() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        adapter.push_update(fixture.add_button_update("Export"));
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        let outcome = session
            .click(&Selector::role_named(accesskit::Role::Button, "Export"))
            .expect("export clicked after it appeared");
        assert_eq!(outcome.detail, "clicked");
    }

    #[test]
    fn click_on_disabled_times_out() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        let err = session
            .perform_action(
                &Selector::role_named(accesskit::Role::Button, "Confirm"),
                SemAction::Click,
                &ActionOpts {
                    timeout: Duration::from_millis(200),
                    force: false,
                },
            )
            .expect_err("disabled button cannot be clicked");
        assert!(
            matches!(err, CoreError::Timeout { timeout, .. } if timeout >= Duration::from_millis(199)),
            "{err}"
        );
        assert!(
            session.adapter().actions().is_empty(),
            "no action must reach the adapter"
        );
    }

    #[test]
    fn force_skips_stability_but_not_enabled() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .perform_action(
                &save_selector(),
                SemAction::Click,
                &ActionOpts {
                    timeout: Duration::from_secs(2),
                    force: true,
                },
            )
            .expect("force click skips stability");
    }

    #[test]
    fn assertions_run_on_session() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .expect(save_selector())
            .to_be_visible()
            .expect("save visible");
        session
            .expect(Selector::role_named(accesskit::Role::Button, "Confirm"))
            .to_be_disabled()
            .expect("confirm disabled");
        session
            .expect(Selector::role(accesskit::Role::TextInput))
            .to_have_value("ada@example.dev")
            .expect("email value");
        session
            .expect(Selector::role_named(accesskit::Role::Button, "Ghost"))
            .to_be_hidden()
            .expect("ghost hidden");
    }

    #[test]
    fn fill_sets_value_through_action() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .fill(&Selector::role(accesskit::Role::TextInput), "hello@dev")
            .expect("fill");
        let actions = session.adapter().actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].1,
            SemAction::SetValue {
                text: "hello@dev".into()
            }
        );
    }

    #[test]
    fn settle_drains_scripted_frames() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        let summary = session.settle(Instant::now() + Duration::from_secs(1));
        assert!(summary.quiescent);
        // 3 scripted passes + 1 trailing pass (the pump that discovers
        // quiescence still presents one pass — harness-driven adapters do).
        assert_eq!(summary.frames_presented, 4);
    }

    #[test]
    fn generation_guard_blocks_stale_action() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let resolved = session
            .tree()
            .resolve(&save_selector())
            .expect("save resolves");
        session
            .adapter_mut()
            .push_update(fixture.remove_save_update());
        session.pump();
        let err = session.tree.check_generation(&resolved).expect_err("stale");
        assert!(matches!(err, CoreError::StaleNode { .. }), "{err}");
    }

    #[test]
    fn strict_mode_violation_surfaces() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let err = session
            .click(&Selector::role(accesskit::Role::Button))
            .expect_err("four buttons");
        assert!(matches!(err, CoreError::Other(_)), "{err}");
        assert!(err.to_string().contains("resolved to"), "{err}");
    }

    /// Level-2 (M1): an iced-shaped adapter synthesizes an AccessKit
    /// update through `logical_snapshot`; selectors, actionability and
    /// actions must run unchanged, and the adapter-minted NodeId routes
    /// back to the adapter (§6.1).
    #[test]
    fn logical_snapshot_resolves_and_routes_actions() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::bare().as_logical();
        adapter.push_logical(crate::adapter::LogicalTreeDiff {
            adapter: "mock-logical",
            update: fixture.initial_update(),
            removed: vec![],
            test_ids: vec![],
            changed: true,
        });
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        session.settle(Instant::now() + Duration::from_secs(1));

        let outcome = session.click(&save_selector()).expect("logical click");
        assert_eq!(outcome.detail, "clicked");
        let actions = session.adapter().actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].1, SemAction::Click);
        assert_eq!(
            actions[0].0, fixture.save_button,
            "the adapter-minted id routes back unchanged"
        );

        // ...and expect() assertions work on the synthesized tree too.
        session
            .expect(Selector::role_named(accesskit::Role::Button, "Confirm"))
            .to_be_disabled()
            .expect("logical tree carries semantics");
    }

    #[test]
    fn clock_fast_forward_reaches_mock() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .clock_fast_forward(Duration::from_secs(30))
            .expect("mock supports clock");
        assert_eq!(
            session.adapter().virtual_time(),
            Some(Duration::from_secs(30))
        );
    }

    // --- M2: clock service (§14) ---------------------------------------

    #[test]
    fn clock_service_steps_fast_forward_and_ledgers() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let report = session
            .clock(crate::adapter::ClockCmd::FastForward(Duration::from_secs(
                1,
            )))
            .expect("stepped fast forward");
        // 1 s at 60 Hz = 60 steps.
        assert_eq!(report.steps, 60);
        assert_eq!(report.state.virtual_time, Some(Duration::from_secs(1)));
        // Every step is a ledger row with cumulative virtual time.
        let ledger = session.clock_ledger();
        assert_eq!(ledger.len(), 60);
        let last = ledger.last().expect("ledger");
        assert_eq!(last.virtual_time_us, 1_000_000);
        assert_eq!(last.command, LedgerCommand::FastForward);
    }

    #[test]
    fn clock_install_pause_resume_and_run_for_gate() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let report = session
            .clock(crate::adapter::ClockCmd::InstallVirtual)
            .expect("install");
        assert_eq!(report.state.virtual_time, Some(Duration::ZERO));

        let report = session
            .clock(crate::adapter::ClockCmd::Pause)
            .expect("pause");
        assert!(report.state.paused);

        let err = session
            .clock(crate::adapter::ClockCmd::RunFor(Duration::from_millis(100)))
            .expect_err("paused clock refuses real-time pacing");
        assert!(err.to_string().contains("paused"), "{err}");

        // Virtual jumps stay allowed while paused (determinism, §14).
        session
            .clock(crate::adapter::ClockCmd::FastForward(
                Duration::from_millis(50),
            ))
            .expect("virtual jump while paused");

        let report = session
            .clock(crate::adapter::ClockCmd::Resume)
            .expect("resume");
        assert!(!report.state.paused);
        session
            .clock(crate::adapter::ClockCmd::RunFor(Duration::from_millis(100)))
            .expect("run_for works after resume");
    }

    #[test]
    fn clock_set_time_is_absolute() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let report = session
            .clock(crate::adapter::ClockCmd::SetTime(Duration::from_millis(
                120_000,
            )))
            .expect("set time");
        assert_eq!(report.state.virtual_time, Some(Duration::from_secs(120)));
        let probed = session
            .clock(crate::adapter::ClockCmd::Probe)
            .expect("probe");
        assert_eq!(probed.state, report.state);
        assert_eq!(probed.steps, 0, "probe applies no steps");
    }

    // --- M2: state seams (§14 mock domain) ------------------------------

    #[derive(Debug, Default)]
    struct AppDb {
        rows: usize,
        fixture: Option<String>,
    }

    fn app_seams() -> SeamRegistry {
        let mut reg = SeamRegistry::new();
        reg.register("reset_state", |ctx, scope| {
            let db = ctx
                .downcast_mut::<AppDb>()
                .ok_or("wrong runner context type")?;
            db.rows = 0;
            db.fixture = Some(scope.to_string());
            Ok(format!("db reset to {scope}"))
        });
        reg.register("seed_users", |ctx, scope| {
            let db = ctx
                .downcast_mut::<AppDb>()
                .ok_or("wrong runner context type")?;
            db.rows = 3;
            db.fixture = Some(scope.to_string());
            Ok("3 users seeded".into())
        });
        reg
    }

    fn seam_session() -> Session<crate::mock::MockAdapter> {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session.register_seams(&app_seams());
        session
    }

    fn seam_session_with_state() -> Session<crate::mock::MockAdapter> {
        let fixture = Fixture::buttons();
        let mut adapter = crate::mock::MockAdapter::with_fixture(&fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        adapter = adapter.with_state(AppDb {
            rows: 42,
            fixture: None,
        });
        let mut session = Session::new(adapter);
        session.settle(Instant::now() + Duration::from_secs(1));
        session.register_seams(&app_seams());
        session
    }

    #[test]
    fn seam_writes_the_runner_context_via_run_in() {
        let mut session = seam_session_with_state();
        let detail = session.reset_state("seed-42").expect("reset seam");
        assert_eq!(detail, "db reset to seed-42");
        let db = session
            .adapter()
            .state::<AppDb>()
            .expect("runner context preserved");
        assert_eq!(db.rows, 0, "the seam reset the app state");
        assert_eq!(db.fixture.as_deref(), Some("seed-42"));
    }

    #[test]
    fn seam_scope_is_passed_through() {
        let mut session = seam_session_with_state();
        let detail = session.run_seam("seed_users", "smoke").expect("seed");
        assert_eq!(detail, "3 users seeded");
        let db = session.adapter().state::<AppDb>().expect("context");
        assert_eq!(db.rows, 3);
        assert_eq!(db.fixture.as_deref(), Some("smoke"));
    }

    #[test]
    fn canned_override_wins_over_the_real_hook() {
        let mut session = seam_session_with_state();
        session.set_seam_response("reset_state", Some("canned-ok".into()));
        let detail = session.reset_state("seed-42").expect("override");
        assert_eq!(detail, "canned-ok");
        // The real hook never ran — state untouched.
        let db = session.adapter().state::<AppDb>().expect("context");
        assert_eq!(db.rows, 42);
        // Clear restores the real hook.
        session.clear_seam("reset_state");
        let detail = session.reset_state("seed-42").expect("real hook");
        assert_eq!(detail, "db reset to seed-42");
    }

    #[test]
    fn unknown_seam_is_a_clean_error() {
        let mut session = seam_session();
        let err = session
            .run_seam("missing", "")
            .expect_err("unregistered seam");
        assert!(err.to_string().contains("no seam registered"), "{err}");
        assert!(err.to_string().contains("reset_state"), "names what exists");
    }

    #[test]
    fn reset_seam_is_configurable() {
        let mut session = seam_session_with_state();
        session.with_reset_seam("seed_users");
        let detail = session.reset_state("baseline").expect("seeds");
        assert_eq!(detail, "3 users seeded");
        assert_eq!(session.seam_ids(), vec!["reset_state", "seed_users"]);
    }

    // --- Video service (§10, M3) ------------------------------------------

    fn pixel_adapter() -> MockAdapter {
        MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
            16,
            8,
            vec![96; 16 * 8 * 4],
        )
    }

    #[test]
    fn video_records_presented_frames_and_dedups_static_spans() {
        let mut session = Session::new(pixel_adapter());
        session.settle(Instant::now() + Duration::from_secs(1));
        let dir = tempfile::tempdir().expect("tmpdir");
        session.video_start_in(dir.path(), 25).expect("start");
        assert!(session.recording());

        // Two distinct frames + one static span land during the window;
        // the final stop pump presents one more (queue-empty hash 0).
        session.adapter_mut().push_frame(0xA1);
        session.pump();
        session.adapter_mut().push_frame(0xA1);
        session.pump();
        session.adapter_mut().push_frame(0xB2);
        session.pump();

        let summary = session.video_stop().expect("stop");
        assert!(!session.recording());
        assert_eq!(
            summary.frames, 3,
            "0xA1 + 0xB2 + the final pump frame (hash 0)"
        );
        assert_eq!(summary.dropped, 1, "the static span collapsed");
        assert!(
            summary.path.exists(),
            "finalized artifact exists at {}",
            summary.path.display()
        );
    }

    #[test]
    fn video_requires_the_pixel_capability_row() {
        let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
        let dir = tempfile::tempdir().expect("tmpdir");
        let err = session
            .video_start_in(dir.path(), 25)
            .expect_err("no pixel capability");
        assert!(
            matches!(err, CoreError::Unsupported(_)),
            "clean §6.2 error: {err}"
        );
    }

    #[test]
    fn double_video_start_is_rejected_and_stop_without_start_too() {
        let mut session = Session::new(pixel_adapter());
        session.settle(Instant::now() + Duration::from_secs(1));
        let dir = tempfile::tempdir().expect("tmpdir");
        session.video_start_in(dir.path(), 25).expect("first start");
        let err = session
            .video_start_in(dir.path(), 25)
            .expect_err("second start");
        assert!(err.to_string().contains("already running"), "{err}");
        session.video_stop().expect("stop");
        let err = session.video_stop().expect_err("second stop");
        assert!(err.to_string().contains("no video recording"), "{err}");
    }
}
