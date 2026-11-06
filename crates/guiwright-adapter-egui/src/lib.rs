//! Guiwright FrameworkAdapter for egui (architecture §4.1, M0 scope).
//!
//! Embeds a [`egui::Context`] and drives it with [`egui::RawInput`] — no
//! window, no backend (the kittest-style windowless path, §11). Every pump
//! runs one full pass (`begin_pass` → app → `end_pass`), which is also one
//! presented frame for stability purposes.
//!
//! Level-1 facts this adapter relies on (all verified against egui 0.37):
//! * `Context::enable_accesskit()` — tree updates flow through
//!   `FullOutput::platform_output.accesskit_update` as **full** updates
//!   with `TreeInfo::new(root)` and `TreeId::ROOT` every pass; the driver's
//!   consumer tree diffs them at O(changes) (§15.2).
//! * Inbound semantic actions arrive as
//!   `egui::Event::AccessKitActionRequest` inside the next `RawInput` —
//!   stock widgets honor `Click` (and text edits honor `SetValue`).
//! * The virtual clock is `RawInput.time` — egui has no internal
//!   wall-clock in its animation paths (§14), so Guiwright owns time
//!   completely.
//! * The frame digest is the xxh3 of the tessellated scene (§10
//!   "scene-digest where available") — no GPU readback at M0.

#![forbid(unsafe_code)]

mod frame_digest;
mod input;

pub use input::translate_input;

use std::time::Duration;

use egui::{Context, RawInput};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameworkAdapter, InputEvent, PumpOutcome, SemAction,
};

/// The embedded app: receives the `Context` once per pass, exactly like an
/// `eframe` update callback.
pub type EguiApp = Box<dyn FnMut(&Context) + 'static>;

/// Windowless egui adapter (§4.1 "Embed").
pub struct EguiAdapter {
    ctx: Context,
    app: EguiApp,
    meta: AdapterMeta,
    screen_rect: egui::Rect,
    /// Virtual time in seconds (RawInput.time, §14).
    time: f64,
    clock_installed: bool,
    clock_paused: bool,
    /// Input events queued for the next pass (§9: synthetic input enters the
    /// same choke point as OS input).
    pending_input: Vec<egui::Event>,
    /// Full TreeUpdate from the last pass, drained once by the driver.
    pending_tree_update: Option<accesskit::TreeUpdate>,
    frame_seq: u64,
    /// Frame deadline for settle loops (one pass per pump).
    frame_step: f64,
}

impl EguiAdapter {
    /// Create an adapter around an app closure. AccessKit tree generation
    /// is enabled immediately (§18: adapters force-enable at driver attach).
    pub fn new(app: impl FnMut(&Context) + 'static) -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        Self {
            ctx,
            app: Box::new(app),
            meta: AdapterMeta {
                name: "egui",
                framework: "egui",
                framework_version: egui_version(),
                caps: Caps {
                    tree_update: true,
                    logical_snapshot: false,
                    input: true,
                    last_frame: false,
                    render_frame: false,
                    clock: true,
                },
            },
            screen_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(800.0, 600.0)),
            time: 0.0,
            clock_installed: false,
            clock_paused: false,
            pending_input: Vec::new(),
            pending_tree_update: None,
            frame_seq: 0,
            frame_step: 1.0 / 60.0,
        }
    }

    /// Logical window size in points (default 800×600).
    pub fn with_screen_size(mut self, width: f32, height: f32) -> Self {
        self.screen_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(width, height));
        self
    }

    /// Current virtual time (§14 clock service state).
    pub fn virtual_time(&self) -> Duration {
        Duration::from_secs_f64(self.time.max(0.0))
    }

    fn run_pass(&mut self) -> egui::FullOutput {
        // The backend must describe the current viewport in RawInput
        // (egui 0.37 requirement): one ROOT viewport with our logical size.
        let mut viewports = egui::ViewportIdMap::default();
        viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                inner_rect: Some(self.screen_rect),
                ..Default::default()
            },
        );
        let input = RawInput {
            viewport_id: egui::ViewportId::ROOT,
            screen_rect: Some(self.screen_rect),
            time: Some(self.time),
            predicted_dt: self.frame_step as f32,
            events: std::mem::take(&mut self.pending_input),
            viewports,
            ..Default::default()
        };
        let app = &mut self.app;
        // egui 0.37: `run_pass` wraps begin/end pass and hands the app the
        // Context (apps build panels via `ctx.root_ui`).
        self.ctx.run_pass(input, |ctx| app(ctx))
    }
}

impl FrameworkAdapter for EguiAdapter {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // One pass per pump; time advances by one frame step when the clock
        // runs (pause freezes animations, §14).
        if !self.clock_paused {
            self.time += self.frame_step;
        }
        let mut output = self.run_pass();
        self.frame_seq += 1;
        self.pending_tree_update = output.platform_output.accesskit_update.take();

        // egui asks for more frames only while animations/interactions run;
        // a static app reports repaint_delay > 0 and settles.
        let repaint_requested = output
            .viewport_output
            .values()
            .any(|v| v.repaint_delay.is_zero());

        // Scene digest (§10): xxh3 over tessellated geometry — a cheap
        // visual-change signal without any pixel readback.
        let shapes = std::mem::take(&mut output.shapes);
        let prims = self.ctx.tessellate(shapes, output.pixels_per_point);
        // Hash-only pipeline at M0: textures are not applied to any atlas,
        // so the delta is dropped explicitly (pixel capture lands in M1).
        output.drop_without_applying_deltas();
        let hash = frame_digest::digest(&prims);

        PumpOutcome {
            frames: vec![guiwright_driver::adapter::FrameStamp {
                seq: self.frame_seq,
                hash,
            }],
            repaint_requested,
            quit_requested: false,
        }
    }

    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        self.pending_tree_update.take()
    }

    fn act(
        &mut self,
        node: accesskit::NodeId,
        action: SemAction,
    ) -> Result<ActionOutcome, AdapterError> {
        // §8.4: prefer the framework's own semantics — egui translates
        // AccessKit ActionRequests into widget behavior on stock widgets.
        let request = match &action {
            SemAction::Click | SemAction::Check | SemAction::Uncheck => accesskit::ActionRequest {
                action: accesskit::Action::Click,
                target_node: node,
                target_tree: accesskit::TreeId::ROOT,
                data: None,
            },
            SemAction::SetValue { text } => accesskit::ActionRequest {
                action: accesskit::Action::SetValue,
                target_node: node,
                target_tree: accesskit::TreeId::ROOT,
                data: Some(accesskit::ActionData::Value(text.clone().into_boxed_str())),
            },
            SemAction::Focus => accesskit::ActionRequest {
                action: accesskit::Action::Focus,
                target_node: node,
                target_tree: accesskit::TreeId::ROOT,
                data: None,
            },
            SemAction::DoubleClick => accesskit::ActionRequest {
                action: accesskit::Action::Click,
                target_node: node,
                target_tree: accesskit::TreeId::ROOT,
                data: Some(accesskit::ActionData::CustomAction(1)),
            },
            SemAction::Blur => accesskit::ActionRequest {
                action: accesskit::Action::Blur,
                target_node: node,
                target_tree: accesskit::TreeId::ROOT,
                data: None,
            },
            // Hover/Scroll are real-input actions (§8.4): the session routes
            // them to `input()` with coordinates; reaching `act` means the
            // session has no bounds — report clearly.
            SemAction::Hover | SemAction::Scroll { .. } => {
                return Err(AdapterError::Internal(
                    "hover/scroll need bounds; the session routes them through input()".into(),
                ));
            }
            SemAction::Press { key, modifiers } => {
                // Keyboard actions ride the real pipeline (§9).
                return match input::named_key(key, modifiers) {
                    Some(event) => {
                        self.pending_input.push(event);
                        Ok(ActionOutcome {
                            applied_at_frame: self.frame_seq,
                            detail: format!("pressed {key}"),
                        })
                    }
                    None => Err(AdapterError::Internal(format!("unknown key {key}"))),
                };
            }
        };
        self.pending_input
            .push(egui::Event::AccessKitActionRequest(request));
        Ok(ActionOutcome {
            applied_at_frame: self.frame_seq,
            detail: format!("queued {}", action.describe()),
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        self.pending_input
            .extend(translate_input(ev, self.screen_rect));
        Ok(())
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        match cmd {
            ClockCmd::InstallVirtual => {
                self.clock_installed = true;
            }
            ClockCmd::FastForward(dt) => {
                // Jump time: egui interpolates animations from RawInput.time,
                // so the next pass lands at the target state (§14).
                self.time += dt.as_secs_f64();
            }
            ClockCmd::RunFor(dt) => {
                self.time += dt.as_secs_f64();
            }
            ClockCmd::Pause => self.clock_paused = true,
            ClockCmd::Resume => self.clock_paused = false,
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_secs_f64(self.time.max(0.0))),
            paused: self.clock_paused,
        })
    }
}

fn egui_version() -> String {
    // egui doesn't expose a version constant; read it from the crate graph
    // at compile time so traces report the truth (§18 AdapterMeta).
    env!("CARGO_PKG_VERSION").to_string()
}
