//! The downstream gpui binding — [`FrameworkAdapter`] over the mirrored
//! gpui/test-support surface, through the framework-free engine.
//!
//! This is the compilable form of `guiwright-adapter-gpui`'s contract
//! (`contract.rs` "The adapter binding" table): the engine calls are the
//! ones the real `glue/gpui` harness makes, in the same order, against
//! the mirrored types — so the proof compiles and runs anywhere, no gpui
//! graph required. Every §-reference and seam note is carried over
//! verbatim from the real glue's ride log.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use accesskit::NodeId;
use guiwright_adapter_gpui::clock::{GpuiClockStep, plan as clock_plan};
use guiwright_adapter_gpui::frames::FrameDigest;
use guiwright_adapter_gpui::input::{GpuiButton, GpuiInputStep, translate};
use guiwright_adapter_gpui::{A11yHybrid, ActivationPolicy, GpuiAdapterContract, RunInPlan};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, PumpOutcome, SemAction,
};
use guiwright_driver::seams::SeamTask;

use crate::mirror::{
    App, CounterState, Entity, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, Point, Window,
};

/// The gpui downstream adapter (§6.1 hybrid): Level-1 — the app's
/// per-frame `TreeUpdate`s flow through the engine's activation model;
/// input rides the real (mirrored) `dispatch_event` pipeline; pixels are
/// the test-support-gated readback with the §15.5 strided digest; time is
/// the seeded virtual dispatcher; seams run on the foreground executor.
pub struct GpuiDownstreamGlue {
    meta: AdapterMeta,
    /// The engine's a11y activation model (§18 "ActivationHandler with
    /// immediate activation for tests"): force-enabled at driver attach,
    /// so the per-frame updates flow from the first pump.
    hybrid: A11yHybrid,
    window: Rc<RefCell<Window>>,
    cx: App,
    entity: Entity<CounterState>,
    /// The scripted test-support raster (the `rendered_frame` stand-in).
    pixels: Vec<u8>,
    pass_seq: u64,
}

impl GpuiDownstreamGlue {
    /// Boot the mirror app and attach the driver: the window is built
    /// around the counter entity, a11y is force-activated immediately
    /// (§18), and the first a11y restatement is taken. `test_support` is
    /// modeled ON (the caps row the §11 GPUI line promises).
    pub fn boot(entity: Entity<CounterState>) -> Result<Self, AdapterError> {
        let (window, entity) = crate::mirror::counter_window(&entity);
        let mut hybrid = A11yHybrid::new(ActivationPolicy::Immediate);
        // Force-enable at driver attach (§18): the driver never waits for
        // a screen reader.
        hybrid.activate();
        let mut me = Self {
            meta: AdapterMeta {
                name: "gpui",
                framework: "gpui",
                // §18 churn mitigation: `AdapterMeta.framework_version`
                // reports the GPUI platform crate versions — the mirror
                // names itself instead.
                framework_version: "0.2.2 (downstream-glue mirror)".to_string(),
                caps: GpuiAdapterContract::default().caps_for(true),
            },
            hybrid,
            window,
            cx: App::new(),
            entity,
            pixels: vec![0; 480 * 240 * 4],
            pass_seq: 0,
        };
        me.hybrid.apply(&me.window.borrow_mut().a11y_tree_update());
        Ok(me)
    }

    /// The entity handle (the §14 runner context and the app state the
    /// reads close over).
    pub fn entity(&self) -> &Entity<CounterState> {
        &self.entity
    }

    /// Wrap one engine input step into `PlatformInput` (the glue's
    /// dispatch step: `translate` produces the vocabulary, the window
    /// executes it — one `dispatch_event` per step).
    fn wrap_step(step: GpuiInputStep) -> PlatformInput {
        match step {
            GpuiInputStep::MouseMove { x, y } => PlatformInput::MouseMove(MouseMoveEvent {
                position: Point { x, y },
            }),
            GpuiInputStep::MouseDown { x, y, button } => PlatformInput::MouseDown(MouseDownEvent {
                position: Point { x, y },
                button: wrap_button(button),
                click_count: 1,
            }),
            GpuiInputStep::MouseUp { x, y, button } => PlatformInput::MouseUp(MouseUpEvent {
                position: Point { x, y },
                button: wrap_button(button),
                click_count: 1,
            }),
            GpuiInputStep::Scroll { dx: _, dy: _ } => {
                // gpui scroll events ride `PlatformInput::Scroll` (the
                // wheel vocabulary); the mirror's single window has no
                // scroll surface — the step collapses to a move (the
                // mirror's documented divergence).
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: Point { x: 0.0, y: 0.0 },
                })
            }
            GpuiInputStep::KeyDown { keystroke } => PlatformInput::KeyDown(KeyDownEvent {
                keystroke: crate::mirror::Keystroke {
                    modifiers: keystroke
                        .modifiers
                        .iter()
                        .map(|m| format!("{m:?}"))
                        .collect(),
                    key: keystroke.key,
                },
            }),
            GpuiInputStep::KeyUp { keystroke } => PlatformInput::KeyDown(KeyDownEvent {
                keystroke: crate::mirror::Keystroke {
                    modifiers: keystroke
                        .modifiers
                        .iter()
                        .map(|m| format!("{m:?}"))
                        .collect(),
                    key: keystroke.key,
                },
            }),
            GpuiInputStep::ImeCommit { text } => {
                // The IME commit rides the keystroke path in the mirror
                // (the REAL glue gates fills before this point anyway —
                // the scheduler blocker in `contract.rs`).
                PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: crate::mirror::Keystroke {
                        modifiers: vec![],
                        key: text,
                    },
                })
            }
        }
    }

    /// The test-support raster (the `rendered_frame` stand-in): the
    /// restated a11y geometry filled into a 480×240 RGBA8 buffer — the
    /// strided xxh3 over this is the frame hash (§15.5 "no scene digest —
    /// use the §15.5 strided-pixel xxh3").
    fn render_pixels(&mut self) {
        let (w, h) = (480_u32, 240_u32);
        for chunk in self.pixels.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[28, 28, 32, 255]);
        }
        let bounds = self.window.borrow_mut().a11y_tree_update();
        for (id, node) in &bounds.nodes {
            let Some(rect) = node.bounds() else { continue };
            let (color, border) = match node.role() {
                accesskit::Role::Button => ([70, 110, 235, 255], true),
                _ => ([40, 40, 46, 255], false),
            };
            let _ = id;
            fill_rect(&mut self.pixels, w, h, rect, color, border);
        }
    }
}

fn fill_rect(
    pixels: &mut [u8],
    w: u32,
    h: u32,
    rect: accesskit::Rect,
    color: [u8; 4],
    border: bool,
) {
    let x0 = rect.x0.max(0.0) as u32;
    let y0 = rect.y0.max(0.0) as u32;
    let x1 = (rect.x1.min(f64::from(w))) as u32;
    let y1 = (rect.y1.min(f64::from(h))) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let edge = border && (x == x0 || x + 1 == x1 || y == y0 || y + 1 == y1);
            let px = if edge { [220, 220, 230, 255] } else { color };
            let i = ((y * w + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&px);
        }
    }
}

fn wrap_button(button: GpuiButton) -> MouseButton {
    match button {
        GpuiButton::Left => MouseButton::Left,
        GpuiButton::Right => MouseButton::Right,
        GpuiButton::Middle => MouseButton::Middle,
    }
}

impl FrameworkAdapter for GpuiDownstreamGlue {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // The foreground-executor round (§5.1 waker row): the dispatcher
        // round runs scheduled work, then the frame flush restates the
        // a11y tree and presents the frame stamp (§15.5 digest).
        self.cx.dispatch_round(0);
        let update = self.window.borrow_mut().a11y_tree_update();
        let stats = self.hybrid.apply(&update);
        self.pass_seq += 1;
        self.render_pixels();
        let hash = FrameDigest::of(&self.pixels).value();
        let _ = stats;
        PumpOutcome {
            frames: vec![FrameStamp {
                seq: self.pass_seq,
                hash,
            }],
            repaint_requested: false,
            quit_requested: false,
        }
    }

    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        // Level-1 while active (§4.5 "AccessKit is unconditional"): the
        // app restates; the hybrid's activation model gates the flow.
        let update = self.window.borrow_mut().a11y_tree_update();
        self.hybrid.apply(&update)?;
        Some(update)
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        // §8.4: a11y `ActionRequest` → synthesized mouse events at the
        // node's restated bounds (the same route the real glue takes).
        let Some(rect) = self.hybrid.bounds_of(node) else {
            return Err(AdapterError::Internal(
                "act: no restated bounds for the a11y node".into(),
            ));
        };
        let (x, y) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
        match action {
            SemAction::Click => {
                for input in [
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position: Point { x, y },
                    }),
                    PlatformInput::MouseDown(MouseDownEvent {
                        position: Point { x, y },
                        button: MouseButton::Left,
                        click_count: 1,
                    }),
                    PlatformInput::MouseUp(MouseUpEvent {
                        position: Point { x, y },
                        button: MouseButton::Left,
                        click_count: 1,
                    }),
                ] {
                    self.window.borrow_mut().dispatch_event(input);
                }
                Ok(ActionOutcome {
                    applied_at_frame: self.pass_seq,
                    detail: "clicked".to_string(),
                })
            }
            SemAction::SetValue { text } => {
                let _ = text;
                Err(AdapterError::Unsupported(
                    "gpui fill: the real KeyDown path panics inside zed's test \
                     scheduler (scheduler/src/test_scheduler.rs:232, \
                     next_session_id overflow after three Keystroke dispatches) \
                     — gated per contract.rs (m80)",
                ))
            }
            _other => Err(AdapterError::Unsupported(
                "gpui act: no ActionRequest synthesis for this action (§9 table row)",
            )),
        }
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        // The input vocabulary through the real event pipeline (§4.5
        // "the canonical seam — synthetic events are bit-identical to OS
        // input"): one dispatch per translated step, in order.
        for step in translate(ev) {
            self.window
                .borrow_mut()
                .dispatch_event(Self::wrap_step(step));
        }
        Ok(())
    }

    fn last_frame(&mut self) -> Option<FrameRef> {
        self.render_frame(None).ok()
    }

    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        // test-support `rendered_frame` (§4.5): the mirror models the
        // feature ON (the caps row), the raster is the scripted stand-in.
        let (w, h) = size.unwrap_or((480, 240));
        if (w, h) != (480, 240) {
            return Err(AdapterError::Unsupported(
                "gpui render_frame: the mirror rasterizes the 480×240 fixture only",
            ));
        }
        self.render_pixels();
        Ok(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash: FrameDigest::of(&self.pixels).value(),
            },
            size: (w, h),
            pixels: Some(Arc::from(self.pixels.clone())),
        })
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        // The seeded virtual dispatcher rounds (§6.2 GPUI row): each
        // DispatchRound runs the scheduled work before the (virtual)
        // deadline — the mirror schedules nothing of its own, so the
        // round is the advance itself.
        for step in clock_plan(cmd) {
            match step {
                GpuiClockStep::DispatchRound { advance_ms } => {
                    self.cx.dispatch_round(advance_ms);
                }
                GpuiClockStep::MarkInstalled => {}
                GpuiClockStep::MarkPaused { paused } => self.cx.set_paused(paused),
            }
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_millis(self.cx.virtual_ms())),
            paused: self.cx.paused(),
        })
    }

    fn run_in(&mut self, task: SeamTask) -> Result<String, AdapterError> {
        // The §14 state-seam contract: the task dispatches on the
        // foreground executor with the app's root entity as the runner
        // context (§4.5 "main-thread confinement" — the mirror runs
        // single-threaded, so the hop is already foreground).
        let plan = RunInPlan::new("gpui-run-in");
        let entity = self.entity.clone();
        let mut context = entity.any_mut();
        let context_dyn: &mut dyn std::any::Any = &mut *context;
        plan.execute(task, |task, context| task(context), context_dyn)
            .map_err(AdapterError::Internal)
    }
}
