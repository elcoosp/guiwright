//! The `iced` feature's real seam: [`EmulatorHarness`] binds the engine to
//! `iced_test`'s [`Emulator`] (architecture §4.6, §17 MVP trio).
//!
//! `iced_test::Emulator` drives a **real iced `Program`** headlessly —
//! boot, update, subscriptions and tasks run through iced's own runtime
//! with the `Zen` determinism mode (settle-to-quiescence), and interactions
//! inject real `core::Event`s through `UserInterface::update` (§4.6
//! "injection = `UserInterface::update(&[core::Event])`"). One renderer
//! lives for the harness's lifetime; the UI rebuilds from the program's
//! view per instruction, restoring widget state through the layout cache.
//!
//! The widget tree comes from the §4.6 read-back ([`dump::HarnessApp`]):
//! the harness routes one hook message into the app, the app returns
//! `Task::widget(TreeDumpOp)` (the default [`dump::HarnessApp::run_hook`]),
//! and the operation ships the laid-out walk over an mpsc channel —
//! "operations run on the loop thread, the channel is the safe boundary".
//!
//! Bounds are layout rectangles at scale factor 1.0 (the Emulator's
//! headless window): logical px == physical px (the [`WidgetReport`]
//! vocabulary, §9 "adapters own any scale conversion").
//!
//! Known quirks encoded here (§9 "each adapter's known quirks are encoded
//! here"): `instruction::Key` covers Enter/Escape/Tab/Backspace only;
//! fills focus by clicking the field's right edge (iced places the cursor
//! at the click point) and clears with backspaces before retyping —
//! backspaces on an empty editor are no-ops, so the count can overshoot.
//! Wheel input has no instruction vocabulary and stays an explicit
//! [`AdapterError::Unsupported`] row.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced_test::Instruction;
use iced_test::core::mouse::Button;
use iced_test::core::{Point, Size};
use iced_test::emulator::{Emulator, Event, Mode};
use iced_test::futures::Executor as FuturesExecutor;
use iced_test::futures::futures::channel::mpsc;
use iced_test::instruction::{Interaction, Key, Keyboard, Mouse, Target};
use iced_test::program::Program as IcedProgram;

use guiwright_driver::adapter::AdapterError;

use crate::dump::{DumpResult, WINDOW_HANDLE};
// The app-facing read-back contract rides the emulator's surface: apps
// implement `HarnessApp` and pass `TreeDumpTx` through their hook message.
pub use crate::dump::{HarnessApp, TreeDumpTx};
use crate::harness::{IcedAdapter, ReportFn, SimInteraction, SimulatorSeam};
use crate::input::PointerOp;
use crate::logical::WidgetReport;
use crate::overlay::{OverlayProgram, OverlayState};
use crate::registry::RoleRegistry;

/// How long an interaction may wait for the Emulator's `Zen` settle (the
/// §6.1 rendezvous: the instruction has applied once `Ready` arrives).
const SETTLE: Duration = Duration::from_secs(1);

/// How long a quiesce drain spins for in-flight task events after the
/// channel goes quiet (thread-pool tasks complete off-thread).
const QUIESCE_GRACE: Duration = Duration::from_millis(16);

/// Map a [`guiwright_driver::adapter::PointerButton`] onto iced's.
fn mouse_button(button: guiwright_driver::adapter::PointerButton) -> Button {
    match button {
        guiwright_driver::adapter::PointerButton::Primary => Button::Left,
        guiwright_driver::adapter::PointerButton::Secondary => Button::Right,
        guiwright_driver::adapter::PointerButton::Middle => Button::Middle,
    }
}

/// The four keys `iced_test::instruction::Key` can express (§9 quirk row).
fn instruction_key(key: &str) -> Option<Key> {
    match key.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(Key::Enter),
        "escape" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        _ => None,
    }
}

/// The real harness seam: owns the [`Emulator`], the program and the
/// read-back receiver, and answers the [`SimulatorSeam`] vocabulary by
/// running real instructions through iced's pipeline. The program rides
/// the [`OverlayProgram`] wrapper: the §13 overlay composites inside the
/// app's own view build (the screenshot pass paints it, M83).
pub struct EmulatorHarness<P: HarnessApp> {
    program: OverlayProgram<P>,
    emulator: Emulator<OverlayProgram<P>>,
    events: mpsc::Receiver<Event<OverlayProgram<P>>>,
    last: Option<WidgetReport>,
    size: Size,
}

impl<P: HarnessApp> EmulatorHarness<P> {
    /// Boot the headless runtime for `program` at `size` (logical px) and
    /// take the first tree read.
    pub fn new(program: P, size: (f32, f32)) -> Result<Self, AdapterError> {
        let size = Size::new(size.0, size.1);
        // The runtime sends boot/instruction/task events through this
        // channel; the harness performs each action until `Ready` (Zen).
        let (sender, events) = mpsc::channel(1024);
        let wrapped = OverlayProgram::new(program, OverlayState::default());
        let mut harness = Self {
            emulator: Emulator::new(sender, &wrapped, Mode::Zen, size),
            program: wrapped,
            events,
            last: None,
            size,
        };
        harness.drain_until_ready()?;
        harness.read_tree()?;
        Ok(harness)
    }

    /// The last tree read, window-rooted — the adapter's report source.
    pub fn last_report(&self) -> Option<WidgetReport> {
        self.last.clone()
    }

    /// Install/clear the §13 overlay plan: the wrapper program composites
    /// it inside the app's own view build — the next UI build (any
    /// instruction pass or screenshot) paints it (M83).
    pub fn set_overlay_plan(&mut self, plan: Option<&guiwright_inspect::overlay::OverlayPlan>) {
        self.program.overlay_state().set_plan(plan);
    }

    /// Drain the overlay paint log — one entry per draw call the layer
    /// performed (the §13 draw-call evidence).
    pub fn take_overlay_paint_log(&mut self) -> Vec<String> {
        self.program.overlay_state().take_paint_log()
    }

    /// One headless render pass (§10/§11): build + draw the current view
    /// through a dedicated `tiny-skia` headless renderer — the same pass
    /// shape the instructions ride, overlay composited when active.
    ///
    /// This does NOT reuse `Emulator::screenshot`: that path consumes the
    /// Emulator's layout cache (`take().unwrap()` in `iced_test`'s
    /// operation/instruction branches would panic on the next pass). A
    /// mid-session render must be side-effect-free (§15.4's render-once
    /// invariant), so the UI builds over a fresh cache and the Emulator's
    /// cache stays untouched (the §9 quirk row).
    pub fn screenshot_pass(&mut self) -> Result<(u32, u32, Vec<u8>), AdapterError> {
        use iced_test::core::renderer::Headless;
        use iced_test::runtime::user_interface::{Cache, UserInterface};

        let executor = <P::Executor as FuturesExecutor>::new()
            .map_err(|err| AdapterError::Internal(format!("iced screenshot executor: {err}")))?;
        let settings = self.program.settings();
        let mut renderer = executor
            .block_on(P::Renderer::new(
                settings.default_font,
                settings.default_text_size,
                None,
            ))
            .ok_or_else(|| AdapterError::Internal("iced headless renderer unavailable".into()))?;

        let theme = self.emulator.theme(&self.program).unwrap_or_else(|| {
            <P::Theme as iced_test::core::theme::Base>::default(iced_test::core::theme::Mode::Light)
        });
        let style = <P::Theme as iced_test::core::theme::Base>::base(&theme);
        let element = self.emulator.view(&self.program);
        let mut ui = UserInterface::build(element, self.size, Cache::default(), &mut renderer);
        ui.draw(
            &mut renderer,
            &theme,
            &iced_test::core::renderer::Style {
                text_color: style.text_color,
            },
            iced_test::core::mouse::Cursor::Unavailable,
        );
        // The guarded conversion is the session's documented pattern (the
        // egui adapter's `physical_size`): clamp into range, then
        // `u32::try_from` proves it — negative or absurd viewport values
        // cannot reach the raster as sign-truncated garbage.
        let px = |v: f32| u32::try_from(v.round().clamp(0.0, u32::MAX as f32) as i64).unwrap_or(0);
        let physical = Size::new(px(self.size.width), px(self.size.height));
        let rgba = renderer.screenshot(physical, 1.0, style.background_color);
        Ok((physical.width, physical.height, rgba))
    }

    /// Receive and perform runtime events until the channel settles. An
    /// idle pass returns at once (nothing in flight); after observed
    /// events a short grace window covers tasks completing off-thread.
    fn drain(&mut self, grace: Duration) -> Result<(), AdapterError> {
        let mut quiet_at: Option<Instant> = None;
        loop {
            match self.events.try_recv() {
                Ok(Event::Action(action)) => {
                    self.emulator.perform(&self.program, action);
                    quiet_at.get_or_insert(Instant::now() + grace);
                }
                Ok(Event::Ready) | Err(mpsc::TryRecvError::Closed) => return Ok(()),
                Ok(Event::Failed(instruction)) => {
                    return Err(AdapterError::Internal(format!(
                        "iced instruction failed: {instruction}"
                    )));
                }
                Err(mpsc::TryRecvError::Empty) => match quiet_at {
                    // Nothing was in flight: an idle pump must not burn
                    // the grace window per pass.
                    None => return Ok(()),
                    Some(at) if Instant::now() >= at => return Ok(()),
                    Some(_) => std::thread::sleep(Duration::from_millis(1)),
                },
            }
        }
    }

    /// Wait for the `Zen` settle (`Event::Ready`) — the contract `run`
    /// fulfills per instruction.
    fn drain_until_ready(&mut self) -> Result<(), AdapterError> {
        let deadline = Instant::now() + SETTLE;
        loop {
            match self.events.try_recv() {
                Ok(Event::Action(action)) => {
                    self.emulator.perform(&self.program, action);
                }
                Ok(Event::Ready) => return Ok(()),
                Ok(Event::Failed(instruction)) => {
                    return Err(AdapterError::Internal(format!(
                        "iced instruction failed: {instruction}"
                    )));
                }
                Err(mpsc::TryRecvError::Closed) => {
                    return Err(AdapterError::Internal(
                        "iced emulator event channel closed".into(),
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return Err(AdapterError::Internal(
                            "iced emulator did not settle in time".into(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }

    /// Run one instruction (a real interaction or keyboard event) and wait
    /// for the settle that proves it applied.
    fn run_instruction(&mut self, instruction: Instruction) -> Result<String, AdapterError> {
        let detail = format!("{instruction}");
        self.emulator.run(&self.program, instruction);
        self.drain_until_ready()?;
        Ok(detail)
    }

    /// One §4.6 read-back: route the hook message into the app (the
    /// default hook arm returns `Task::widget(TreeDumpOp)`), pump the
    /// runtime until the walk result lands (the dump task completes on the
    /// executor, off-thread), and receive it.
    fn read_tree(&mut self) -> Result<(), AdapterError> {
        let (tx, rx) = std::sync::mpsc::channel::<DumpResult>();
        self.emulator
            .update(&self.program, P::hook(TreeDumpTx::new(tx)));
        let deadline = Instant::now() + SETTLE;
        loop {
            // Perform pending runtime events (the dump task's Widget
            // action among them); a Zen `Ready` only means the app's own
            // task set settled — the dump result is the real signal.
            loop {
                match self.events.try_recv() {
                    Ok(Event::Action(action)) => {
                        self.emulator.perform(&self.program, action);
                    }
                    Ok(Event::Ready) | Err(mpsc::TryRecvError::Empty) => break,
                    Ok(Event::Failed(instruction)) => {
                        return Err(AdapterError::Internal(format!(
                            "iced instruction failed: {instruction}"
                        )));
                    }
                    Err(mpsc::TryRecvError::Closed) => {
                        return Err(AdapterError::Internal(
                            "iced emulator event channel closed".into(),
                        ));
                    }
                }
            }
            if let Ok(DumpResult { mut root }) = rx.try_recv() {
                crate::dump::adopt_names(&mut root);
                self.last = Some(WidgetReport::new("window", WINDOW_HANDLE).with_child(root));
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(AdapterError::Internal(
                    "iced tree read-back returned no result — the app must route \
                     HarnessApp::hook into HarnessApp::run_hook"
                        .into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The bounds center of a widget from the last tree read.
    fn center_of(&self, handle: u64) -> Result<Point, AdapterError> {
        fn walk(report: &WidgetReport, handle: u64) -> Option<Point> {
            if report.handle == handle {
                if let Some([x0, y0, x1, y1]) = report.bounds {
                    return Some(Point::new(
                        (f64::midpoint(x0, x1)) as f32,
                        (f64::midpoint(y0, y1)) as f32,
                    ));
                }
            }
            report.children.iter().find_map(|c| walk(c, handle))
        }
        self.last
            .as_ref()
            .and_then(|root| walk(root, handle))
            .ok_or_else(|| {
                AdapterError::Internal(format!("widget handle {handle} has no laid-out bounds"))
            })
    }

    /// The right edge of a widget's bounds — iced places the text cursor at
    /// the click point, so fills click here before clearing.
    fn right_edge_of(&self, handle: u64) -> Result<Point, AdapterError> {
        fn walk(report: &WidgetReport, handle: u64) -> Option<[f64; 4]> {
            if report.handle == handle {
                return report.bounds;
            }
            report.children.iter().find_map(|c| walk(c, handle))
        }
        let [x0, y0, x1, y1] = self
            .last
            .as_ref()
            .and_then(|root| walk(root, handle))
            .ok_or_else(|| {
                AdapterError::Internal(format!("widget handle {handle} has no laid-out bounds"))
            })?;
        Ok(Point::new(
            ((x1 - 2.0).max(x0)) as f32,
            (f64::midpoint(y0, y1)) as f32,
        ))
    }

    /// The current content length of a text input (the clear count).
    fn value_len_of(&self, handle: u64) -> usize {
        fn walk(report: &WidgetReport, handle: u64) -> Option<usize> {
            if report.handle == handle {
                return report.value.as_deref().map(str::len);
            }
            report.children.iter().find_map(|c| walk(c, handle))
        }
        self.last
            .as_ref()
            .and_then(|root| walk(root, handle))
            .unwrap_or(0)
    }
}

impl<P: HarnessApp> SimulatorSeam for EmulatorHarness<P> {
    fn run_one_pass(&mut self) {
        // A pass pumps pending runtime work and refreshes the tree read;
        // a failed read keeps the previous tree (resolve/act surface the
        // staleness at action time).
        let _ = self.drain(QUIESCE_GRACE);
        let _ = self.read_tree();
    }

    fn apply_op(&mut self, op: &crate::actions::WidgetOp) -> Result<String, AdapterError> {
        use crate::actions::WidgetOp;
        match op {
            WidgetOp::Press(handle) => {
                let center = self.center_of(*handle)?;
                self.run_instruction(Instruction::Interact(Interaction::Mouse(Mouse::Click {
                    button: Button::Left,
                    target: Some(Target::Point(center)),
                })))
            }
            WidgetOp::SetContent(handle, text) => {
                // Focus (iced focuses a text input on click), clear through
                // the real editor pipeline, retype. Backspaces on an empty
                // editor are no-ops, so the count can overshoot.
                let edge = self.right_edge_of(*handle)?;
                self.run_instruction(Instruction::Interact(Interaction::Mouse(Mouse::Click {
                    button: Button::Left,
                    target: Some(Target::Point(edge)),
                })))?;
                let clear = self.value_len_of(*handle) + 2;
                for _ in 0..clear {
                    self.run_instruction(Instruction::Interact(Interaction::Keyboard(
                        Keyboard::Press(Key::Backspace),
                    )))?;
                }
                self.run_instruction(Instruction::Interact(Interaction::Keyboard(
                    Keyboard::Typewrite(text.clone()),
                )))
            }
            WidgetOp::Focus(handle) => {
                // iced focuses focusable widgets on click; there is no
                // focus-without-activate instruction.
                let center = self.center_of(*handle)?;
                self.run_instruction(Instruction::Interact(Interaction::Mouse(Mouse::Click {
                    button: Button::Left,
                    target: Some(Target::Point(center)),
                })))?;
                Ok("focused via click".into())
            }
            WidgetOp::Blur(_) => Err(AdapterError::Unsupported(
                "blur (iced has no unfocus instruction; focus moves by clicking elsewhere)",
            )),
            WidgetOp::None => Ok("noop".into()),
        }
    }

    fn interact(&mut self, interaction: &SimInteraction) -> Result<String, AdapterError> {
        match interaction {
            SimInteraction::Click(text) => {
                // The Emulator resolves text targets through iced's own
                // selector (`Selector::find` over the laid-out UI).
                self.run_instruction(Instruction::Interact(Interaction::Mouse(Mouse::Click {
                    button: Button::Left,
                    target: Some(Target::Text(text.clone())),
                })))
            }
            SimInteraction::Typewrite(text) => self.run_instruction(Instruction::Interact(
                Interaction::Keyboard(Keyboard::Typewrite(text.clone())),
            )),
            SimInteraction::TapKey(key) => {
                let key = instruction_key(key).ok_or(AdapterError::Unsupported(
                    "key not in the iced instruction surface (Enter/Escape/Tab/Backspace)",
                ))?;
                self.run_instruction(Instruction::Interact(Interaction::Keyboard(
                    Keyboard::Type(key),
                )))
            }
        }
    }

    fn pointer(&mut self, op: &PointerOp) -> Result<String, AdapterError> {
        match op {
            PointerOp::Move { x, y } => self.run_instruction(Instruction::Interact(
                Interaction::Mouse(Mouse::Move(Target::Point(Point::new(*x as f32, *y as f32)))),
            )),
            PointerOp::Down { x, y, button } => {
                self.run_instruction(Instruction::Interact(Interaction::Mouse(Mouse::Press {
                    button: mouse_button(*button),
                    target: Some(Target::Point(Point::new(*x as f32, *y as f32))),
                })))
            }
            PointerOp::Up { x, y, button } => {
                self.run_instruction(Instruction::Interact(Interaction::Mouse(Mouse::Release {
                    button: mouse_button(*button),
                    target: Some(Target::Point(Point::new(*x as f32, *y as f32))),
                })))
            }
            PointerOp::Wheel { .. } => Err(AdapterError::Unsupported(
                "wheel (the iced instruction surface has no wheel vocabulary)",
            )),
        }
    }

    fn screen_size(&self) -> (f64, f64) {
        (f64::from(self.size.width), f64::from(self.size.height))
    }

    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.set_overlay_plan(plan);
        Ok(())
    }

    fn screenshot(&mut self) -> Result<(u32, u32, Vec<u8>), AdapterError> {
        self.screenshot_pass()
    }
}

impl<P: HarnessApp> std::fmt::Debug for EmulatorHarness<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmulatorHarness")
            .field("size", &self.size)
            .field("has_tree", &self.last.is_some())
            .finish_non_exhaustive()
    }
}

/// The shared seam handed to [`IcedAdapter::new`]: the adapter reads the
/// report through one handle and drives the seam through the other (the
/// same shape the egui adapter's app-state closure rides).
pub struct SharedHarness<P: HarnessApp>(Rc<RefCell<EmulatorHarness<P>>>);

impl<P: HarnessApp> SimulatorSeam for SharedHarness<P> {
    fn run_one_pass(&mut self) {
        self.0.borrow_mut().run_one_pass();
    }

    fn apply_op(&mut self, op: &crate::actions::WidgetOp) -> Result<String, AdapterError> {
        self.0.borrow_mut().apply_op(op)
    }

    fn interact(&mut self, interaction: &SimInteraction) -> Result<String, AdapterError> {
        self.0.borrow_mut().interact(interaction)
    }

    fn pointer(&mut self, op: &PointerOp) -> Result<String, AdapterError> {
        self.0.borrow_mut().pointer(op)
    }

    fn screen_size(&self) -> (f64, f64) {
        self.0.borrow().screen_size()
    }

    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.0.borrow_mut().show_overlay(plan)
    }

    fn screenshot(&mut self) -> Result<(u32, u32, Vec<u8>), AdapterError> {
        self.0.borrow_mut().screenshot()
    }
}

impl<P: HarnessApp> std::fmt::Debug for SharedHarness<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedHarness").finish_non_exhaustive()
    }
}

/// Build the embedded adapter over a real [`EmulatorHarness`]: the report
/// closure reads the harness's last tree read, the seam drives the same
/// harness (the egui adapter's shared-app-state shape).
pub fn embedded_adapter<P: HarnessApp>(
    harness: EmulatorHarness<P>,
    registry: RoleRegistry,
) -> IcedAdapter<SharedHarness<P>> {
    let shared = Rc::new(RefCell::new(harness));
    let seam = SharedHarness(Rc::clone(&shared));
    let report: ReportFn = Box::new(move || {
        shared
            .borrow()
            .last_report()
            .unwrap_or_else(|| WidgetReport::new("window", WINDOW_HANDLE))
    });
    IcedAdapter::new(report, seam, registry)
}
