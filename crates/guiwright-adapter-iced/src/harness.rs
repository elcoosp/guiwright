//! The `iced` feature: thin glue binding the engine to `iced_test` 0.14
//! (architecture §4.6, §18 "adapters are thin & feature-gated").
//!
//! Build with `--features iced` — that pulls the iced stack
//! (`iced_test` + the `tiny-skia` CPU backend) and is exactly what the CI
//! matrix job `adapter-iced (features: iced)` runtime-tests. The default
//! build carries none of it, per §5.3 ("nothing else leaks into release
//! builds").
//!
//! The real seam lives in [`crate::emulator`]: `EmulatorHarness` binds
//! `iced_test::Emulator` to a real `Program` and implements
//! [`SimulatorSeam`] over the instruction vocabulary (`Mouse::Click` /
//! `Move` / `Press` / `Release`, `Keyboard::Typewrite` / `Type`) plus the
//! §4.6 `Task::widget` tree read-back. Rows of the §6.2 capability matrix
//! the instruction surface can't express (`render_frame`, wheel, blur)
//! stay explicit [`AdapterError::Unsupported`] rows so the gap is visible,
//! not silent.
//!
//! [`AdapterError`]: guiwright_driver::adapter::AdapterError

use std::hash::Hasher;
use std::time::Duration;

use accesskit::{NodeId, Role};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PumpOutcome, SemAction,
};
use twox_hash::xxhash3_64::Hasher as Xxh3;

use crate::actions::{WidgetOp, route};
use crate::input::{InputOp, PointerOp, translate};
use crate::logical::{LogicalTree, WidgetReport};
use crate::policy::{Mode, may_run_pass};

/// The frame step the §14 fast-forward advances in (the §14 pacing
/// default; the Emulator's Zen drain settles each step).
const FRAME_MS: u64 = 16;

/// Frame digest (§10): xxh3 over the report's structural summary — the
/// cheap visual-change signal when the backend produces no pixels.
fn digest_report(report: &WidgetReport) -> u64 {
    fn walk(hasher: &mut Xxh3, r: &WidgetReport) {
        hasher.write(r.kind.as_bytes());
        hasher.write_u64(r.handle);
        if let Some(text) = &r.text {
            hasher.write(text.as_bytes());
        }
        if let Some([x0, y0, x1, y1]) = r.bounds {
            hasher.write_u64(x0.to_bits());
            hasher.write_u64(y0.to_bits());
            hasher.write_u64(x1.to_bits());
            hasher.write_u64(y1.to_bits());
        }
        hasher.write_u8(u8::from(r.enabled));
        hasher.write_u8(u8::from(r.focused));
        hasher.write_u32(r.children.len() as u32);
        for child in &r.children {
            walk(hasher, child);
        }
    }
    let mut hasher = Xxh3::default();
    walk(&mut hasher, report);
    hasher.finish()
}

/// The app-facing view report callback: the harness asks the app for its
/// current widget tree once per pass (iced rebuilds the widget tree every
/// update — §4.6 — so a full report is the natural shape).
pub type ReportFn = Box<dyn FnMut() -> WidgetReport + 'static>;

/// One simulated interaction the `Simulator` vocabulary can express
/// (`click` by text selector; `typewrite`/`tap_key`).
#[derive(Clone, Debug, PartialEq)]
pub enum SimInteraction {
    /// `Simulator::click` on the accessible text of a widget.
    Click(String),
    /// `Simulator::typewrite` into the focused widget.
    Typewrite(String),
    /// `Simulator::tap_key` on the focused widget.
    TapKey(String),
}

/// The simulator-facing seam: pump the program one pass, execute one
/// widget op, run one interaction or pointer op.
pub trait SimulatorSeam {
    fn run_one_pass(&mut self);
    fn apply_op(&mut self, op: &WidgetOp) -> Result<String, AdapterError>;
    fn interact(&mut self, interaction: &SimInteraction) -> Result<String, AdapterError>;
    /// Pointer ops through the real event pipeline (the Emulator
    /// translates them to `Mouse::Move` / `Press` / `Release`
    /// instructions, §9).
    fn pointer(&mut self, op: &PointerOp) -> Result<String, AdapterError>;
    fn screen_size(&self) -> (f64, f64);
    /// Install/clear the §13 overlay plan (M83: the wrapper program
    /// composites it inside the app's own view build). Default: no
    /// overlay surface — the honest Unsupported row.
    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        let _ = plan;
        Err(AdapterError::Unsupported("inspect overlay (§13)"))
    }
    /// One headless render pass (§10/§11): the Emulator's `tiny-skia`
    /// screenshot — real pixels at the boot viewport. Default: no pixel
    /// path.
    fn screenshot(&mut self) -> Result<(u32, u32, Vec<u8>), AdapterError> {
        Err(AdapterError::Unsupported("render_frame"))
    }
}

/// The iced adapter (§6.1 trait implementation over the engine).
pub struct IcedAdapter<S: SimulatorSeam> {
    meta: AdapterMeta,
    engine: LogicalTree,
    mode: Mode,
    report: ReportFn,
    sim: S,
    pass_seq: u64,
    last_frame_hash: u64,
    /// The §14 virtual-time ledger (M84): the Emulator's determinism
    /// machinery is the framework's clock primitive.
    clock_ledger: Option<Duration>,
    clock_paused: bool,
}

impl<S: SimulatorSeam> IcedAdapter<S> {
    /// Capability row for iced (§6.2): no AccessKit tree (Level-2), input
    /// through the real pipeline, frames via the Emulator's headless
    /// screenshot (§10/§11 — the same pass the instructions ride),
    /// clock best-effort via Emulator modes.
    pub fn new(report: ReportFn, sim: S, registry: crate::registry::RoleRegistry) -> Self {
        Self {
            meta: AdapterMeta {
                name: "iced",
                framework: "iced",
                framework_version: env!("CARGO_PKG_VERSION").to_string(),
                caps: Caps {
                    tree_update: false,
                    logical_snapshot: true,
                    input: true,
                    last_frame: false,
                    render_frame: true,
                    clock: true,
                },
            },
            engine: LogicalTree::with_registry("iced", registry),
            mode: Mode::default(),
            report,
            sim,
            pass_seq: 0,
            last_frame_hash: 0,
            clock_ledger: None,
            clock_paused: false,
        }
    }

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    fn current_report(&mut self) -> WidgetReport {
        (self.report)()
    }
}

impl<S: SimulatorSeam + 'static> FrameworkAdapter for IcedAdapter<S> {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    /// Level-2 adapter: no AccessKit tree of its own (§6.2 row).
    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome {
        let mut outcome = PumpOutcome::default();
        let mut passes = 0;
        while may_run_pass(self.mode, passes, &deadline) {
            self.sim.run_one_pass();
            self.pass_seq += 1;
            passes += 1;
            // iced rebuilds per update: every pass produces a report; the
            // engine folds it into the next `logical_snapshot` (§4.6).
            let report = self.current_report();
            let digest = digest_report(&report);
            self.last_frame_hash = digest;
            outcome.frames.push(FrameStamp {
                seq: self.pass_seq,
                hash: digest,
            });
            // Zen settles to quiescence: a stable digest means the app
            // stopped producing changes — stop pumping (§4.6 Zen).
            if passes > 1 && digest == outcome.frames[outcome.frames.len() - 2].hash {
                break;
            }
        }
        outcome
    }

    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        let report = self.current_report();
        Some(self.engine.rebuild(&report))
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        let handle = self
            .engine
            .handle_of(node)
            .ok_or_else(|| AdapterError::Internal(format!("unknown node {node:?}")))?;
        let role = self.engine.role_of(node).unwrap_or(Role::Unknown);
        let op = route(role, &action, handle)?;
        let detail = self.sim.apply_op(&op)?;
        self.sim.run_one_pass();
        Ok(ActionOutcome {
            applied_at_frame: self.pass_seq,
            detail,
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        match translate(&ev)? {
            // Pointer ops ride the real event pipeline (§9): the seam
            // translates them to `Mouse::Move` / `Press` / `Release`
            // instructions resolved against the laid-out UI.
            InputOp::Pointer(op) => {
                self.sim.pointer(&op)?;
                self.sim.run_one_pass();
                Ok(())
            }
            InputOp::Key(key) => {
                let interaction = match key {
                    crate::input::KeyOp::Text { text } => SimInteraction::Typewrite(text),
                    crate::input::KeyOp::Down { key, .. } => SimInteraction::TapKey(key),
                    crate::input::KeyOp::Up { .. } => {
                        return Ok(()); // tap_key is down+up in one step
                    }
                };
                self.sim.interact(&interaction)?;
                self.sim.run_one_pass();
                Ok(())
            }
        }
    }

    /// Virtual clock (§14, M84): the binding rides the Emulator's own
    /// determinism machinery — `Mode::Zen` settles every spawned task and
    /// every due subscription tick. `iced_test` exposes no virtual timer
    /// (subscriptions fire on real time), so `fast_forward` advances the
    /// ledger in frame-sized steps, draining the runtime each step — the
    /// due timers land, the app state advances. The §14 iced row is
    /// documented best-effort determinism (§14 "per-framework injection
    /// table").
    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        match cmd {
            ClockCmd::InstallVirtual => self.clock_ledger = Some(Duration::ZERO),
            ClockCmd::Probe => {}
            ClockCmd::FastForward(dt) | ClockCmd::RunFor(dt) => {
                let ms = dt.as_millis() as u64;
                for _ in 0..ms.div_ceil(FRAME_MS) {
                    std::thread::sleep(Duration::from_millis(FRAME_MS));
                    self.sim.run_one_pass();
                }
                self.clock_ledger = Some(self.clock_ledger.unwrap_or_default().saturating_add(dt));
            }
            ClockCmd::Pause => self.clock_paused = true,
            ClockCmd::Resume => self.clock_paused = false,
            ClockCmd::SetTime(t) => self.clock_ledger = Some(t),
        }
        Ok(ClockState {
            virtual_time: self.clock_ledger,
            paused: self.clock_paused,
        })
    }

    /// Inspector overlay (§13, M83): the plan rides the seam into the
    /// wrapper program — the next view build composites it (the
    /// Emulator's own pass paints it, no extra pass, §15.4).
    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.sim.show_overlay(plan)
    }

    /// Render on demand (§10/§11, M83): the Emulator's headless
    /// `tiny-skia` screenshot — one build+draw of the current view, the
    /// overlay composited when the inspect session is on.
    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        // The Emulator rasterizes at its boot viewport (the headless
        // window size is fixed at construction — §4.6).
        let _ = size;
        let (width, height, rgba) = self.sim.screenshot()?;
        let pixels: std::sync::Arc<[u8]> = std::sync::Arc::from(rgba);
        let hash = {
            use std::hash::Hasher;
            use std::time::Duration;
            use twox_hash::xxhash3_64::Hasher as Xxh3;
            let mut hasher = Xxh3::default();
            hasher.write(&pixels);
            hasher.finish()
        };
        self.pass_seq += 1;
        Ok(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash,
            },
            size: (width, height),
            pixels: Some(pixels),
        })
    }
}

/// Last frame digest (§10 "wait for visual change" signal).
impl<S: SimulatorSeam> IcedAdapter<S> {
    pub fn last_frame_hash(&self) -> u64 {
        self.last_frame_hash
    }
}

impl<S: SimulatorSeam> std::fmt::Debug for IcedAdapter<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcedAdapter")
            .field("meta", &self.meta)
            .field("mode", &self.mode)
            .field("pass_seq", &self.pass_seq)
            .field("last_frame_hash", &self.last_frame_hash)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RoleRegistry;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A scripted seam: no iced involved — the engine + adapter contract
    /// is what's under test here; the real seam lands with the CI pass.
    struct ScriptedSim {
        log: Vec<String>,
        screen: (f64, f64),
    }

    impl SimulatorSeam for ScriptedSim {
        fn run_one_pass(&mut self) {}
        fn apply_op(&mut self, op: &WidgetOp) -> Result<String, AdapterError> {
            let detail = format!("{op:?}");
            self.log.push(detail.clone());
            Ok(match op {
                WidgetOp::Press(_) => "pressed".into(),
                WidgetOp::SetContent(_, t) => format!("set {t}"),
                WidgetOp::None => "noop".into(),
                _ => detail,
            })
        }
        fn interact(&mut self, i: &SimInteraction) -> Result<String, AdapterError> {
            self.log.push(format!("{i:?}"));
            Ok("interacted".into())
        }
        fn pointer(&mut self, op: &PointerOp) -> Result<String, AdapterError> {
            self.log.push(format!("{op:?}"));
            Ok("pointered".into())
        }
        fn screen_size(&self) -> (f64, f64) {
            self.screen
        }
    }

    fn make_adapter() -> IcedAdapter<ScriptedSim> {
        let counter = Rc::new(RefCell::new(0u32));
        let c = Rc::clone(&counter);
        let report: ReportFn = Box::new(move || {
            *c.borrow_mut() += 1;
            let pass = *c.borrow();
            WidgetReport::new("window", 100)
                .with_text("Demo")
                .with_child(
                    WidgetReport::new("button", 1)
                        .with_text(if pass <= 2 { "Save" } else { "Saved!" })
                        .with_bounds(16.0, 16.0, 136.0, 48.0),
                )
        });
        IcedAdapter::new(
            report,
            ScriptedSim {
                log: Vec::new(),
                screen: (480.0, 240.0),
            },
            RoleRegistry::new(),
        )
    }

    #[test]
    fn caps_row_matches_the_documented_matrix() {
        let adapter = make_adapter();
        let caps = &adapter.meta().caps;
        assert!(!caps.tree_update, "Level-2: no AccessKit tree");
        assert!(caps.logical_snapshot);
        assert!(caps.input);
        assert!(caps.render_frame, "the Emulator screenshot path (§11)");
        assert!(caps.clock, "the §14 determinism-drain binding (M84)");
    }

    #[test]
    fn pump_settles_when_the_report_stabilizes() {
        let mut adapter = make_adapter();
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(100));
        let outcome = adapter.pump(deadline);
        assert_ne!(outcome.frames.len(), 0);
        // The report changes once ("Save" → "Saved!") then stabilizes: the
        // adapter must not run away to the Zen budget.
        assert!(outcome.frames.len() < 16, "{} passes", outcome.frames.len());
    }

    #[test]
    fn actions_route_through_the_seam() {
        let mut adapter = make_adapter();
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(50));
        let _ = adapter.pump(deadline);
        let diff = adapter.logical_snapshot().expect("logical diff");
        // The report settles "Save" → "Saved!" at the frame boundary (the
        // pump test pins that); route by role — the label churn is not
        // what's under test here.
        let save_node = diff
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Button)
            .map(|(id, _)| *id)
            .expect("save button");
        let outcome = adapter.act(save_node, SemAction::Click).expect("click");
        assert_eq!(outcome.detail, "pressed");
    }

    #[test]
    fn text_input_rides_typewrite() {
        let mut adapter = make_adapter();
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(50));
        let _ = adapter.pump(deadline);
        adapter
            .input(InputEvent::TextInput { text: "ada".into() })
            .expect("typewrite");
    }

    #[test]
    fn digest_tracks_report_changes() {
        let mut adapter = make_adapter();
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(50));
        let first = adapter.pump(deadline);
        let h1 = adapter.last_frame_hash();
        assert_eq!(h1, first.frames.last().expect("frame").hash);
    }
}
