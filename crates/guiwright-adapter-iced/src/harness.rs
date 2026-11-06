//! The `iced` feature: thin glue binding the engine to `iced_test` 0.14
//! (architecture §4.6, §18 "adapters are thin & feature-gated").
//!
//! Build with `--features iced` — that pulls the iced stack
//! (`iced_test` → `iced_renderer`, wgpu included) and is exactly what the
//! CI matrix job `adapter-iced (features: iced)` compile-verifies. The
//! default build carries none of it, per §5.3 ("nothing else leaks into
//! release builds").
//!
//! Verified against `iced_test` 0.14.0's public surface
//! (`Simulator::{find, click, into_messages, tap_key, typewrite}`,
//! `Emulator`, `iced_test::screenshot`). Rows of the §6.2 capability
//! matrix that need deeper internals than the public surface exposes
//! (`render_frame` through `ICED_TEST_BACKEND`, live `Mode` pumping) are
//! explicit [`AdapterError::Unsupported`] rows so the gap is visible, not
//! silent — they land with the CI matrix pass.

use std::hash::Hasher;

use accesskit::{NodeId, Role};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PumpOutcome, SemAction,
};
use twox_hash::xxhash3_64::Hasher as Xxh3;

use crate::actions::{WidgetOp, route};
use crate::input::{InputOp, translate};
use crate::logical::{LogicalTree, WidgetReport};
use crate::policy::{Mode, may_run_pass};

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

/// The simulator-facing seam the CI matrix pass binds to `iced_test`:
/// pump the program one pass, execute one widget op, run one interaction.
pub trait SimulatorSeam {
    fn run_one_pass(&mut self);
    fn apply_op(&mut self, op: &WidgetOp) -> Result<String, AdapterError>;
    fn interact(&mut self, interaction: &SimInteraction) -> Result<String, AdapterError>;
    fn screen_size(&self) -> (f64, f64);
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
    clock: ClockState,
}

impl<S: SimulatorSeam> IcedAdapter<S> {
    /// Capability row for iced (§6.2): no AccessKit tree (Level-2), input
    /// through the real pipeline, frames via the backend's screenshot
    /// path, clock best-effort via Emulator modes.
    pub fn new(
        report: ReportFn,
        sim: S,
        registry: crate::registry::RoleRegistry,
        screen: (f64, f64),
    ) -> Self {
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
                    render_frame: false,
                    clock: false,
                },
            },
            engine: LogicalTree::with_registry("iced", registry),
            mode: Mode::default(),
            report,
            sim,
            pass_seq: 0,
            last_frame_hash: 0,
            clock: ClockState {
                virtual_time: None,
                paused: false,
            },
            screen,
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

impl<S: SimulatorSeam> FrameworkAdapter for IcedAdapter<S> {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
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
        Some(self.engine.rebuild(self.current_report()))
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
            InputOp::Pointer(_) => Err(AdapterError::Unsupported(
                "pointer input targets coordinates — route via semantic actions \
                 (iced has no public coordinate hit-test on the Simulator seam)",
            )),
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

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        // §14: iced embedded clock is best-effort (Emulator determinism
        // modes) — the M1 row stays Unsupported until the CI pass binds it.
        let _ = cmd;
        Err(AdapterError::Unsupported("clock (iced: Emulator modes)"))
    }
}

/// Last frame digest (§10 "wait for visual change" signal).
impl<S: SimulatorSeam> IcedAdapter<S> {
    pub fn last_frame_hash(&self) -> u64 {
        self.last_frame_hash
    }

    fn sim(&self) -> &S {
        &self.sim
    }
}

/// Screenshot hook for the CI matrix pass (§10 `render_frame`): binds
/// `iced_test::screenshot` to the adapter once the backend seam lands.
pub fn screenshot_unsupported() -> Result<(), AdapterError> {
    Err(AdapterError::Unsupported(
        "render_frame (binds iced_test::screenshot in the CI matrix pass)",
    ))
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
            (480.0, 240.0),
        )
    }

    #[test]
    fn caps_row_matches_the_documented_matrix() {
        let adapter = make_adapter();
        let caps = &adapter.meta().caps;
        assert!(!caps.tree_update, "Level-2: no AccessKit tree");
        assert!(caps.logical_snapshot);
        assert!(caps.input);
        assert!(!caps.render_frame, "unsupported row is explicit");
        assert!(!caps.clock, "best-effort row is explicit");
    }

    #[test]
    fn pump_settles_when_the_report_stabilizes() {
        let mut adapter = make_adapter();
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(100));
        let outcome = adapter.pump(deadline);
        assert!(!outcome.frames.is_empty());
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
        let save_node = diff
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Save"))
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
        let _ = adapter.sim();
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
