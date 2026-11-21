//! The downstream floem binding — [`FrameworkAdapter`] over the mirrored
//! floem harness, through the framework-free engine.
//!
//! This is the compilable form of `guiwright-adapter-floem`'s contract
//! (`contract.rs` "The downstream binding"): the engine calls are the
//! ones the real `glue/floem` harness makes, in the same order, against
//! the mirrored types — so the proof compiles and runs anywhere, no
//! floem graph required. Every §-reference and seam note is carried over
//! verbatim from the real glue's ride log.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hasher;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use accesskit::NodeId;
use guiwright_adapter_floem::clock::{FloemClockStep, plan as clock_plan};
use guiwright_adapter_floem::input::{FloemButton, FloemEvent, translate};
use guiwright_adapter_floem::paint::{Color, PaintOp, PaintState};
use guiwright_adapter_floem::{FloemAdapterContract, FloemLogicalTree, OverlayHost, TestIdRegistry, ViewReport};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PumpOutcome, SemAction,
};
use guiwright_inspect::overlay::OverlayPlan;
use twox_hash::XxHash3_64;

use crate::mirror::{self, Event, HeadlessHarness, MirrorView, TestRoot};

/// Facts the latest introspection pass recorded per view handle — the
/// action-time geometry (`act` resolves a minted `NodeId` back to the
/// view's laid-out rect, §8.4) and the editor length a fill would clear.
#[derive(Clone, Copy, Debug)]
struct ItemFacts {
    center: (f64, f64),
    value_len: usize,
}

/// The floem downstream adapter (§6.1): Level-2 — the engine folds mirror
/// `ViewId` walks into synthesized AccessKit diffs; input rides the real
/// (mirrored) harness event pipeline; pixels come from the engine's
/// tiny-skia `PaintState` over the laid-out view geometry; time is the
/// harness-timer plan.
pub struct FloemDownstreamGlue {
    meta: AdapterMeta,
    harness: HeadlessHarness,
    engine: FloemLogicalTree,
    test_ids: TestIdRegistry,
    paint: PaintState,
    size: (f64, f64),
    facts: HashMap<u64, ItemFacts>,
    /// The last introspection pass' diff, delivered to the session on the
    /// next `logical_snapshot` call (Level-2 polling, §6.1).
    pending_diff: Option<LogicalTreeDiff>,
    /// The §13 overlay binding (m86): the active plan + the mock overlay
    /// host — the present-path hook the real floem surface routes
    /// (`WindowHandle` post-paint → PaintOps, mirrored per contract.rs).
    /// `render_frame` extends its scene with the host's ops, so the
    /// overlay composites onto the presented pixels at present time.
    overlay_plan: Option<OverlayPlan>,
    overlay_host: OverlayHost,
    pointer: (f64, f64),
    pass_seq: u64,
    virtual_ms: u64,
    paused: bool,
}

impl FloemDownstreamGlue {
    /// Boot a harness around a freshly built view tree: claim the
    /// [`TestRoot`] (views created by `build` attach to it), run the
    /// factory, wrap the tree in the headless harness at `size`, and take
    /// the first introspection pass. The factory runs **after** the root
    /// is claimed — floem views attach to the current view.
    pub fn boot(
        size: (f64, f64),
        build: impl FnOnce() -> MirrorView,
    ) -> Result<Self, AdapterError> {
        let root = TestRoot::new();
        let view = build();
        let mut me = Self {
            meta: AdapterMeta {
                name: "floem",
                framework: "floem",
                // The mirror carries no framework version of its own; the
                // real glue reports the pinned revision here (§18 churn
                // mitigation: `AdapterMeta.framework_version` names the
                // platform crate versions).
                framework_version: "0.2.0 (downstream-glue mirror)".to_string(),
                // §6.2 floem column with the tiny-skia PaintState
                // contribution active — the contract is the data shape
                // the glue cannot silently drift from.
                caps: FloemAdapterContract::default().caps_with_paint_state_patch(),
            },
            harness: HeadlessHarness::new_with_size(root, view, size.0, size.1),
            engine: FloemLogicalTree::new(),
            test_ids: TestIdRegistry::new(),
            paint: PaintState::new(size.0 as u32, size.1 as u32)
                .map_err(|err| AdapterError::Internal(format!("floem paint state: {err}")))?,
            size,
            facts: HashMap::new(),
            pending_diff: None,
            overlay_plan: None,
            overlay_host: OverlayHost::new(),
            pointer: (size.0 / 2.0, size.1 / 2.0),
            pass_seq: 0,
            virtual_ms: 0,
            paused: false,
        };
        me.introspect_once();
        Ok(me)
    }

    /// The §13 draw-call evidence (the adapters' `take_paint_log` shape):
    /// one entry per overlay draw since the last drain.
    pub fn take_overlay_paint_log(&mut self) -> Vec<String> {
        self.overlay_host.take_paint_log()
    }

    /// One introspection pass: the `ViewId` walk folded into the engine.
    /// Also refreshes the per-handle facts the actions dispatch at and
    /// the test-id registry the O(1) hash reads.
    pub fn introspect_once(&mut self) -> u64 {
        let report = self.walk_view();
        let digest = xxh3_report_digest(&report);
        let diff = self.engine.rebuild(&report);
        self.pending_diff = Some(diff);
        digest
    }

    /// Walk the mirror view tree into a [`ViewReport`], refreshing the
    /// per-handle facts alongside (one pass serves both the engine fold
    /// and the action-time geometry). The kind key + test id parse is the
    /// real glue's `debug_name` grammar, unchanged.
    fn walk_view(&mut self) -> ViewReport {
        let root = self.harness.root_id();
        let mut facts = HashMap::new();
        let report = self.walk_inner(root, &mut facts);
        self.facts = facts;
        report
    }

    fn walk_inner(
        &mut self,
        id: mirror::ViewId,
        facts: &mut HashMap<u64, ItemFacts>,
    ) -> ViewReport {
        let handle = id.inner();
        let debug = id.debug_name();
        let kind = kind_key(&debug);
        let mut report = ViewReport::new(&kind, handle);

        if let Some(test_id) = test_id_of(&debug) {
            report = report.with_test_id(test_id.clone());
            // The upstream `test_id(...)` decorator is a thin patch over
            // this registry (§4.7 contribution 1a): duplicate ids are an
            // app bug and fail loud here.
            if let Err(err) = self.test_ids.register(test_id.clone(), handle) {
                panic!("floem downstream glue: {err}");
            }
        }

        let rect = id.get_visual_rect();
        report = report.with_rect((rect.x0, rect.y0), (rect.width(), rect.height()));

        let center = (rect.x0.midpoint(rect.x1), rect.y0.midpoint(rect.y1));
        let mut value_len = 0;
        // The text/value facts ride the app's shared state (the
        // `SemReader` seam: floem exposes no text getters at the pinned
        // revision — the mirror's shared `Rc<RefCell<String>>` is the
        // same signal the views render from).
        if let Some(text) = id.text() {
            report = report.with_text(text);
        }
        if let Some(value) = id.value() {
            value_len = value.chars().count();
            report = report.with_value(value);
        }
        facts.insert(handle, ItemFacts { center, value_len });

        if self.harness.is_focused(id.clone()) {
            report = report.focused();
        }

        for child in id.children() {
            if !child.is_valid() {
                continue;
            }
            let child_report = self.walk_inner(child, facts);
            report = report.with_child(child_report);
        }
        report
    }

    fn dispatch(&mut self, event: Event) {
        self.harness.dispatch_event(event);
    }

    /// Click at a point: move → press → release (the §9 gesture
    /// synthesis, executed through the harness's real pipeline).
    fn click_at(&mut self, x: f64, y: f64) {
        self.pointer_moved(x, y);
        self.harness.pointer_down(x, y);
        self.harness.pointer_up(x, y);
    }

    fn pointer_moved(&mut self, x: f64, y: f64) {
        self.pointer = (x, y);
        self.harness.pointer_move(x, y);
    }

    /// The engine's tiny-skia paint pass: the laid-out view geometry
    /// rasterized into [`PaintOp`]s (§4.7 contribution 2's driver-side
    /// half — the upstream patch routing floem's display list would make
    /// it bit-identical to the GPU scene).
    fn paint_ops(&self) -> Vec<PaintOp> {
        let mut ops = vec![PaintOp::SolidRect {
            x: 0.0,
            y: 0.0,
            w: self.size.0 as f32,
            h: self.size.1 as f32,
            color: Color::rgba(24, 24, 28, 255),
        }];
        let root = self.harness.root_id();
        Self::collect_ops(&root, &mut ops);
        ops
    }

    fn collect_ops(id: &mirror::ViewId, ops: &mut Vec<PaintOp>) {
        let rect = id.get_visual_rect();
        let color = match kind_key(&id.debug_name()).as_str() {
            "button" => Color::rgba(76, 119, 255, 255),
            "text_input" => Color::rgba(46, 46, 52, 255),
            "label" => Color::rgba(24, 24, 28, 255),
            _ => Color::rgba(34, 34, 40, 255),
        };
        ops.push(PaintOp::RoundedRect {
            x: rect.x0 as f32,
            y: rect.y0 as f32,
            w: rect.width() as f32,
            h: rect.height() as f32,
            radius: 4.0,
            color,
        });
        for child in id.children() {
            Self::collect_ops(&child, ops);
        }
    }
}

impl FrameworkAdapter for FloemDownstreamGlue {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // One harness pass per pump (harness-driven adapters run a full
        // pass per pump): introspect → fold → one frame stamp.
        let digest = self.introspect_once();
        self.pass_seq += 1;
        PumpOutcome {
            frames: vec![FrameStamp {
                seq: self.pass_seq,
                hash: digest,
            }],
            repaint_requested: false,
            quit_requested: false,
        }
    }

    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        // Level-2: no AccessKit of its own — the engine synthesizes the
        // update into `logical_snapshot` (§6.1).
        None
    }

    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        self.pending_diff.take()
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        let handle = self
            .engine
            .handle_of(node)
            .ok_or_else(|| AdapterError::Internal("act: unknown minted NodeId".into()))?;
        let Some(facts) = self.facts.get(&handle) else {
            return Err(AdapterError::Internal(
                "act: no facts for the resolved handle (stale pass?)".into(),
            ));
        };
        // §8.4 action routing: clicks are spatial input at the visual
        // rect; fills are GATED — the headless-IME fill blocker below —
        // and the rest of the vocabulary has no headless dispatch
        // surface at the pinned revision (honest `Unsupported`, §6.2).
        match action {
            SemAction::Click => {
                let (x, y) = facts.center;
                self.click_at(x, y);
                Ok(ActionOutcome {
                    applied_at_frame: self.pass_seq,
                    detail: "clicked".to_string(),
                })
            }
            SemAction::SetValue { text } => {
                let _ = (facts.value_len, text);
                Err(AdapterError::Unsupported(
                    "floem headless fill: text_input requests IME on focus \
                     (src/views/text_input.rs:1403), the headless MockWindow \
                     rejects every IME request (src/window/mock.rs:262) and \
                     handle.rs unwraps the panic — gated per contract.rs (m80)",
                ))
            }
            _other => Err(AdapterError::Unsupported(
                "floem headless act: no dispatch surface for this action \
                 (semantic vocabulary beyond click) at the pinned revision",
            )),
        }
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        // The input vocabulary through the real event pipeline (§4.7
        // "headless = `dispatch_event`"): one dispatch per translated
        // step, in order.
        for step in translate(ev) {
            let event = wrap_event(step)?;
            self.dispatch(event);
        }
        Ok(())
    }

    fn last_frame(&mut self) -> Option<FrameRef> {
        self.render_frame(None).ok()
    }

    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        let (w, h) = size.unwrap_or((self.size.0 as u32, self.size.1 as u32));
        if (w, h) != (self.paint.width(), self.paint.height()) {
            // A size override re-renders at the requested viewport (§6.1).
            self.paint = PaintState::new(w, h)
                .map_err(|err| AdapterError::Internal(format!("floem paint state: {err}")))?;
        }
        // One scene, app + overlay (§13 present-time composition, m86):
        // the overlay host's ops paint after the app's own, so the
        // outline + badges land on top of the presented pixels.
        let mut ops = self.paint_ops();
        if let Some(plan) = self.overlay_plan.as_ref() {
            ops.extend(self.overlay_host.present(plan));
        }
        self.paint.paint(&ops);
        Ok(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash: self.paint.strided_hash(),
            },
            size: (w, h),
            pixels: Some(Arc::from(self.paint.rgba())),
        })
    }

    /// Inspector overlay (§13, m86): store the plan — every
    /// `render_frame` composites it on the present path (the tiny-skia
    /// surface paints the host's ops into the same frame); `None`
    /// clears the scene (the `DisableOverlay` reset).
    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.overlay_plan = plan.cloned();
        if self.overlay_plan.is_none() {
            self.overlay_host.clear();
        }
        Ok(())
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        // The harness-timer plan (§4.7 "pumps UpdatePhase ticks with
        // virtual time"): each Tick advances the mirror's virtual clock;
        // the mirror schedules no timers of its own, so the tick is the
        // advance itself.
        for step in clock_plan(cmd) {
            match step {
                FloemClockStep::Tick { advance_ms } => {
                    self.virtual_ms += advance_ms;
                }
                FloemClockStep::MarkInstalled | FloemClockStep::MarkPaused { .. } => {}
            }
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_millis(self.virtual_ms)),
            paused: self.paused,
        })
    }
}

/// Wrap one engine input step into the mirrored floem event (the glue's
/// dispatch step: `translate` produces the vocabulary, the harness
/// executes it — one `dispatch_event` per step).
fn wrap_event(step: FloemEvent) -> Result<Event, AdapterError> {
    let event = match step {
        FloemEvent::PointerDown {
            x,
            y,
            button,
            click_count,
        } => Event::PointerDown(mirror::PointerDownEvent {
            x,
            y,
            button: wrap_button(button),
            click_count,
        }),
        FloemEvent::PointerUp {
            x,
            y,
            button,
            click_count,
        } => Event::PointerUp(mirror::PointerUpEvent {
            x,
            y,
            button: wrap_button(button),
            click_count,
        }),
        FloemEvent::PointerMove { x, y } => Event::PointerMove(mirror::PointerMoveEvent { x, y }),
        FloemEvent::PointerLeave => Event::PointerLeave,
        FloemEvent::Wheel { x, y, dx, dy } => Event::Wheel(mirror::WheelEvent { x, y, dx, dy }),
        FloemEvent::KeyDown { key, modifiers } => {
            Event::KeyDown(mirror::KeyEvent { key, modifiers })
        }
        FloemEvent::KeyUp { key, modifiers } => Event::KeyUp(mirror::KeyEvent { key, modifiers }),
        FloemEvent::ImeCommit { text } => Event::ImeCommit(mirror::ImeCommitEvent { text }),
    };
    Ok(event)
}

fn wrap_button(button: FloemButton) -> mirror::PointerButton {
    match button {
        FloemButton::Left => mirror::PointerButton::Left,
        FloemButton::Right => mirror::PointerButton::Right,
        FloemButton::Middle => mirror::PointerButton::Middle,
    }
}

/// The view type name the role registry keys on (the real glue's
/// `kind_key` grammar over floem's `debug_name`): truncate a `": "`
/// payload first, take the last `::` path segment, then `snake_case`
/// (`"Button"` → `"button"`).
fn kind_key(debug: &str) -> String {
    let last = debug.rsplit(" - ").next().unwrap_or(debug).trim();
    let stripped = match last.split_once(": ") {
        Some((type_name, _)) => type_name,
        None => last,
    };
    let type_name = stripped.rsplit("::").next().unwrap_or(stripped);
    let mut snake = String::with_capacity(type_name.len());
    for (i, c) in type_name.trim().chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    snake
}

/// The §8.1 test id: the first `debug_name` decorator segment (floem
/// chains them most-recent-first before the type name).
fn test_id_of(debug: &str) -> Option<String> {
    let first = debug.split(" - ").next()?;
    let type_name = debug.rsplit(" - ").next()?;
    (first != type_name && !first.is_empty()).then(|| first.to_string())
}

/// The pass digest (the frame stamp's hash): xxh3 over the report's
/// identity-relevant fields — the same pinned hashing stack the glue
/// rides (`twox-hash`, §15.5's cheap visual-change signal).
fn xxh3_report_digest(report: &ViewReport) -> u64 {
    let mut hasher = XxHash3_64::default();
    digest_view(report, &mut hasher);
    hasher.finish()
}

fn digest_view(report: &ViewReport, hasher: &mut XxHash3_64) {
    // The identity-relevant shape as one line (f64s are not `Hash`, so
    // the bounds ride their rendered form) — cheap, stable, and the same
    // fields the real glue's digest folds.
    let bounds = report
        .bounds
        .map(|b| format!("{} {} {} {}", b[0], b[1], b[2], b[3]))
        .unwrap_or_default();
    let line = format!(
        "{}|{}|{:?}|{:?}|{}|{}|{}|{}",
        report.kind,
        report.handle,
        report.text,
        report.value,
        bounds,
        report.enabled,
        report.focused,
        report.children.len(),
    );
    hasher.write(line.as_bytes());
    for child in &report.children {
        digest_view(child, hasher);
    }
}

/// The counter app the bind test drives (the shared-state seam the real
/// glue documents: the label's text IS the app's signal).
pub fn counter_app(
    count: Rc<std::cell::RefCell<u64>>,
    label_text: Rc<RefCell<String>>,
) -> MirrorView {
    let increment = Rc::clone(&count);
    let increment_text = Rc::clone(&label_text);
    MirrorView::v_stack()
        .at(0.0, 0.0, 480.0, 240.0)
        .with_child(
            MirrorView::button("increment", move || {
                *increment.borrow_mut() += 1;
                *increment_text.borrow_mut() = format!("Count: {}", increment.borrow());
            })
            .at(16.0, 16.0, 136.0, 48.0),
        )
        .with_child(MirrorView::label("count_text", label_text).at(16.0, 64.0, 136.0, 96.0))
}
