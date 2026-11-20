//! Guiwright `FrameworkAdapter` for `egui` (architecture §4.1, M0 scope).
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
//!   "scene-digest where available"); the pixel path (m77) rides
//!   alongside — headless wgpu render + readback keyed by the
//!   cumulative pass number (`pixels`).

#![forbid(unsafe_code)]

mod frame_digest;
mod input;
pub mod overlay;
mod pixels;

pub use input::translate_input;

use std::sync::Arc;
use std::time::Duration;

use egui::{Context, RawInput};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, PumpOutcome, SemAction,
};
use guiwright_inspect::overlay::OverlayPlan;

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
    /// Full [`accesskit::TreeUpdate`] from the last pass, drained once by the driver.
    pending_tree_update: Option<accesskit::TreeUpdate>,
    frame_seq: u64,
    /// Frame deadline for settle loops (one pass per pump).
    frame_step: f64,
    /// The inspector overlay plan (§13, M8): composited at the end of
    /// every pass while a session is on (painter layer after the app
    /// closure — the §13 "egui: painter layer after the pass" path).
    overlay: Option<OverlayPlan>,
    /// The §4.1 pixel path (m77): the headless wgpu backend, probed once.
    pixels: PixelProbe,
    /// The last presented frame's stamp (the pump's, or a fresh render's).
    last_stamp: Option<FrameStamp>,
}

/// One cached pixel frame: its presented stamp, physical size, buffer.
type CapturedFrame = (FrameStamp, (u32, u32), Arc<[u8]>);

/// The lazily probed pixel backend. A GPU-less host caches the probe
/// failure and every pixel row answers [`AdapterError::Unsupported`] (the
/// masonry precedent); a GPU host renders once per pass (§15.4).
enum PixelProbe {
    Unprobed,
    Failed(String),
    Ready(Box<pixels::PixelBackend>),
}

impl std::fmt::Debug for EguiAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EguiAdapter")
            .field("meta", &self.meta)
            .field("frame_seq", &self.frame_seq)
            .finish_non_exhaustive()
    }
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
                    // m77: the §4.1 pixel path — headless wgpu offscreen
                    // render + readback; GPU-less hosts answer the rows
                    // with AdapterError::Unsupported at runtime.
                    last_frame: true,
                    render_frame: true,
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
            overlay: None,
            pixels: PixelProbe::Unprobed,
            last_stamp: None,
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

    /// The overlay plan currently composited (§13, M8): `None` when no
    /// overlay session is on.
    pub fn overlay(&self) -> Option<&OverlayPlan> {
        self.overlay.as_ref()
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
        let overlay = &self.overlay;
        // egui 0.37: `run_pass` wraps begin/end pass and hands the app the
        // Context (apps build panels via `ctx.root_ui`). The inspector
        // overlay paints inside the same pass, after the app closure —
        // the §13 "painter layer after the pass" composition, no extra
        // frame (§15.4 render-once invariant).
        self.ctx.run_pass(input, |ctx| {
            app(ctx);
            if let Some(plan) = overlay {
                crate::overlay::paint_overlay(ctx, plan);
            }
        })
    }
}

impl EguiAdapter {
    /// Physical pixel size of the window (logical rect at scale 1.0).
    /// The guarded conversion is the session's documented pattern: clamp
    /// into range, then `u32::try_from` proves it.
    fn physical_size(&self) -> (u32, u32) {
        let px = |v: f32| u32::try_from(v.round().clamp(0.0, u32::MAX as f32) as i64).unwrap_or(0);
        (px(self.screen_rect.width()), px(self.screen_rect.height()))
    }

    /// Probe the pixel backend once (the outcome is cached: a GPU-less
    /// host never retries, §11).
    fn ensure_pixels_probed(&mut self) {
        if matches!(self.pixels, PixelProbe::Unprobed) {
            self.pixels = match pixels::PixelBackend::probe() {
                Ok(backend) => PixelProbe::Ready(Box::new(backend)),
                Err(e) => {
                    tracing::info!("egui headless pixels unavailable: {e}");
                    PixelProbe::Failed(e)
                }
            };
        }
    }

    /// The pixel cache for the current pass, ready to hand out.
    fn cached_pixels(&self) -> Option<CapturedFrame> {
        let stamp = self.last_stamp?;
        let PixelProbe::Ready(backend) = &self.pixels else {
            return None;
        };
        let physical = self.physical_size();
        let rgba = backend.cached(self.ctx.cumulative_pass_nr(), physical)?;
        Some((stamp, physical, rgba))
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
        // visual-change signal (the scene-digest row: egui has the scene,
        // so the hash semantics stay unchanged with pixels around).
        let shapes = std::mem::take(&mut output.shapes);
        let prims = self.ctx.tessellate(shapes, output.pixels_per_point);
        let hash = frame_digest::digest(&prims);
        let stamp = FrameStamp {
            seq: self.frame_seq,
            hash,
        };
        self.last_stamp = Some(stamp);

        // The §4.1 pixel path (m77): render this pass once, keyed by the
        // cumulative pass number, so `last_frame` (video capture, §10) and
        // `render_frame` (screenshots) hand out the frame's own pixels.
        // GPU-less hosts probe once and stay hash-only — the masonry
        // precedent for a graceful row.
        let physical = self.physical_size();
        let pass_nr = self.ctx.cumulative_pass_nr();
        self.ensure_pixels_probed();
        if let PixelProbe::Ready(backend) = &mut self.pixels {
            match backend.render(
                &output.textures_delta,
                &prims,
                physical,
                output.pixels_per_point,
            ) {
                Ok(rgba) => backend.remember(pass_nr, physical, rgba),
                Err(e) => tracing::warn!("egui pixel render failed: {e}"),
            }
        }
        // Textures are either applied by the pixel path (above) or dropped
        // when no pixel backend is present (hash-only hosts never apply
        // deltas — there is no atlas to feed).
        output.drop_without_applying_deltas();

        PumpOutcome {
            frames: vec![stamp],
            repaint_requested,
            quit_requested: false,
        }
    }

    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        self.pending_tree_update.take()
    }

    /// Last presented frame (§6.1): the pump already rendered this pass's
    /// pixels into the pass-keyed cache — no re-render (the masonry shape).
    /// `None` before the first pump or on a GPU-less host.
    fn last_frame(&mut self) -> Option<FrameRef> {
        let (stamp, size, rgba) = self.cached_pixels()?;
        Some(FrameRef {
            stamp,
            size,
            pixels: Some(rgba),
        })
    }

    /// Headless render on demand (§11): render-once keyed by the egui
    /// cumulative pass number — a cache hit returns the pass's own pixels;
    /// a miss runs one pass (a caller-requested size swaps the window rect
    /// for that pass, the app relays out) and renders it. GPU-less hosts
    /// answer `Unsupported` (the cached probe failure, the masonry
    /// precedent).
    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        self.ensure_pixels_probed();
        if let PixelProbe::Failed(reason) = &self.pixels {
            tracing::debug!("egui render_frame unsupported: {reason}");
            return Err(AdapterError::Unsupported(
                "render_frame (no wgpu adapter on this host — install lavapipe, §11)",
            ));
        }

        let physical = size.unwrap_or_else(|| (self.physical_size().0, self.physical_size().1));
        let pass_nr = self.ctx.cumulative_pass_nr();
        if let PixelProbe::Ready(backend) = &self.pixels {
            if let Some(rgba) = backend.cached(pass_nr, physical) {
                let stamp = self.last_stamp.unwrap_or(FrameStamp {
                    seq: self.frame_seq,
                    hash: 0,
                });
                return Ok(FrameRef {
                    stamp,
                    size: physical,
                    pixels: Some(rgba),
                });
            }
        }

        // A fresh pass for the cache miss. A caller-requested size runs the
        // pass with the matching window rect; the adapter's own rect is
        // restored afterwards (the pump keeps its size).
        let mut output = match size {
            Some(_) => {
                let restored = self.screen_rect;
                self.screen_rect = egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(
                        f32::from(u16::try_from(physical.0).unwrap_or(u16::MAX)),
                        f32::from(u16::try_from(physical.1).unwrap_or(u16::MAX)),
                    ),
                );
                let output = self.run_pass();
                self.screen_rect = restored;
                output
            }
            None => self.run_pass(),
        };
        let shapes = std::mem::take(&mut output.shapes);
        let prims = self.ctx.tessellate(shapes, output.pixels_per_point);
        let hash = frame_digest::digest(&prims);
        let pass_nr = self.ctx.cumulative_pass_nr();
        let mut rgba = None;
        if let PixelProbe::Ready(backend) = &mut self.pixels {
            let rendered = backend
                .render(
                    &output.textures_delta,
                    &prims,
                    physical,
                    output.pixels_per_point,
                )
                .map_err(AdapterError::Internal)?;
            backend.remember(pass_nr, physical, rendered.clone());
            rgba = Some(rendered);
        }
        let stamp = FrameStamp {
            seq: self.frame_seq,
            hash,
        };
        self.last_stamp = Some(stamp);
        Ok(FrameRef {
            stamp,
            size: physical,
            pixels: rgba,
        })
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

    /// The egui adapter has an overlay surface (§13, M8): the plan rides
    /// the painter layer inside the next pass.
    fn show_overlay(&mut self, plan: Option<&OverlayPlan>) -> Result<(), AdapterError> {
        self.overlay = plan.cloned();
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
            ClockCmd::SetTime(t) => {
                self.time = t.as_secs_f64();
            }
            ClockCmd::Pause => self.clock_paused = true,
            ClockCmd::Resume => self.clock_paused = false,
            ClockCmd::Probe => {}
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
