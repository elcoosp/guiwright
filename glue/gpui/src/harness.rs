//! The gpui glue (architecture §4.5, §17 row 7, m80):
//! [`FrameworkAdapter`] over the **real** gpui `test-support` platform.
//!
//! The engine (`guiwright-adapter-gpui`, main workspace) stays
//! framework-free; this crate is the framework half, scoped to its own
//! workspace by the recorded gpui constraint (crate docs of
//! `guiwright-adapter-gpui`): `test-support` is git-only and gpui's
//! Linux platform stack needs system libraries (xkbcommon, wayland) the
//! main workspace must never require. Everything here rides the
//! verified public surface of gpui at the fetched zed revision:
//!
//! * **Boot** — [`TestAppContext::build`] over a seeded
//!   [`TestDispatcher`] (the §4.5 virtual scheduler), then
//!   `TestAppContext::open_window` on the real `TestPlatform`: a real
//!   window running the real counter+form entity ([`crate::app`]).
//! * **Input** — `Window::dispatch_event(PlatformInput)` — the
//!   canonical §4.5 seam: synthetic events are bit-identical to OS
//!   input. Clicks hit-test through the real focus/click machinery;
//!   fills focus the field through a real click and type real
//!   `Keystroke`s through the focused element's `on_key_down`.
//! * **Tree** — the §6.1 hybrid's fallback: the glue synthesizes an
//!   AccessKit-shaped [`LogicalTreeDiff`] from the app's declared
//!   bindings + the framework's **real laid-out geometry** (the
//!   `debug_selector` bounds gpui records at paint) and feeds it
//!   through the engine's [`A11yHybrid`] bookkeeping. The a11y tree
//!   itself never flows on the test platform — the platform never
//!   calls `a11y_init`'s activation callback
//!   (`platform/test/window.rs` overrides nothing), so
//!   `ActivationPolicy::Immediate` can only be modeled, not forced;
//!   the synthesized-diff fallback is the honest row.
//! * **Pixels** — the test-support painted-quad scene (m87): one real
//!   draw per fold (`Window::draw` — the same call the framework's own
//!   visual tests make), the readback `Window::painted_quads()` (§4.5
//!   "the quads in the most recently rendered frame's scene"), each quad
//!   clipped to its content mask, divided out of the window scale and
//!   rasterized driver-side (the engine's source-over quad raster — the
//!   §4.7-style driver-side half: scene data real, raster deterministic
//!   CPU). `last_frame` serves the most recent presented raster from a
//!   per-pass cache; `render_frame` re-rasters on demand — the §10
//!   video + baseline rows ride real pixels. The glyph row (text
//!   primitives) waits on an upstream `PlatformHeadlessRenderer` over
//!   lavapipe: `TestPlatform::new` ships no factory (`platform/test/
//!   platform.rs` passes `None`), so `Window::render_to_image` stays the
//!   documented upstream gap (§4.5).
//! * **Clock** — the test-support dispatcher plan: each
//!   [`GpuiClockStep::DispatchRound`] is
//!   `TestDispatcher::advance_clock` + `run_until_parked` (§5.1's
//!   seeded-scheduler rounds).
//! * **Seams** — `run_in` dispatches the [`SeamTask`] on the app's root
//!   entity (the §14 runner context) inside `App` update — the
//!   foreground thread the entity lives on.

use std::rc::Rc;
use std::time::Duration;

use accesskit::{Node, NodeId, Rect, TreeId, TreeInfo, TreeUpdate};
use gpui::{
    AnyWindowHandle, AppContext, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton,
    Pixels, PlatformInput, Point, ScrollDelta, ScrollEvent, ScrollSource, TestAppContext,
    TestDispatcher, TouchPhase, point, px, size,
};
use guiwright_adapter_gpui::clock::{GpuiClockStep, plan as clock_plan};
use guiwright_adapter_gpui::frames::FrameDigest;
use guiwright_adapter_gpui::input::{GpuiButton, GpuiInputStep, KeyStroke, Modifier, translate};
use guiwright_adapter_gpui::overlay::{PaintedQuad, composite_quads};
use guiwright_adapter_gpui::{A11yHybrid, ActivationPolicy, OverlayHost};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PumpOutcome, SemAction,
};
use guiwright_driver::seams::SeamTask;

use crate::app::CounterForm;

/// `px` is f32-native; the adapter's vocabulary is f64 — one guarded
/// widening at the boundary.
fn px64(v: f64) -> Pixels {
    px(v as f32)
}

/// One app-declared semantic binding (the §4.5 seam): gpui's element
/// DSL carries roles (`role(accesskit::Role)`) and the framework
/// records each named element's real laid-out bounds at paint
/// (`debug_selector`), but no `TreeUpdate` flows on the test platform
/// (see the crate docs) and names have no query surface — so the glue
/// pairs the framework-real geometry with app-declared role/name facts.
/// The reader closures run inside `App` update and read the entity's
/// **real state** (the same fields the views render from — the
/// documented upstream seam; a flowing a11y tree would retire it).
/// One app-state fact reader (labels/values over the root entity).
pub type FactReader = Rc<dyn Fn(&gpui::App) -> String>;

pub struct SemBinding {
    /// The `debug_selector` name (§8.1 test id).
    pub test_id: &'static str,
    /// The AccessKit role the element carries via gpui's `role(...)`.
    pub role: accesskit::Role,
    /// The accessible-name fact (labels).
    pub text: Option<FactReader>,
    /// The editable-content fact (text fields).
    pub value: Option<FactReader>,
}

impl std::fmt::Debug for SemBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemBinding")
            .field("test_id", &self.test_id)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl SemBinding {
    /// A text-carrying binding.
    #[must_use]
    pub fn labeled(
        test_id: &'static str,
        role: accesskit::Role,
        text: impl Fn(&gpui::App) -> String + 'static,
    ) -> Self {
        Self {
            test_id,
            role,
            text: Some(Rc::new(text)),
            value: None,
        }
    }

    /// A value-carrying binding (editable content).
    #[must_use]
    pub fn valued(
        test_id: &'static str,
        role: accesskit::Role,
        value: impl Fn(&gpui::App) -> String + 'static,
    ) -> Self {
        Self {
            test_id,
            role,
            text: None,
            value: Some(Rc::new(value)),
        }
    }

    /// A bare interactive binding (buttons carry static names here).
    #[must_use]
    pub fn interactive(test_id: &'static str, role: accesskit::Role, name: &str) -> Self {
        let name = name.to_string();
        Self {
            test_id,
            role,
            text: Some(Rc::new(move |_| name.clone())),
            value: None,
        }
    }
}

/// The app-declared bindings for [`crate::app::CounterForm`] — the same
/// vocabulary the element tree declares (`debug_selector` names match
/// 1:1).
#[must_use]
pub fn counter_bindings(entity: &gpui::Entity<CounterForm>) -> Vec<SemBinding> {
    let count = entity.clone();
    let name_value = entity.clone();
    let greeting = entity.clone();
    vec![
        SemBinding::interactive("increment", accesskit::Role::Button, "Increment"),
        SemBinding::interactive("decrement", accesskit::Role::Button, "Decrement"),
        SemBinding::valued("name-input", accesskit::Role::TextInput, move |app| {
            name_value.read(app).name.clone()
        }),
        SemBinding::labeled("count-label", accesskit::Role::Label, move |app| {
            format!("Count: {}", count.read(app).count)
        }),
        SemBinding::labeled("greeting", accesskit::Role::Label, move |app| {
            format!("Welcome, {}", greeting.read(app).name)
        }),
    ]
}

/// One rasterized frame (§10): the RGBA raster, its pixel size and the
/// §15.5 strided-surface digest — the unit the `last_frame` readback
/// cache stores (the m86 pixel row: iced and freya ride the same shape).
#[derive(Clone, Debug)]
struct SceneFrame {
    pixels: std::sync::Arc<[u8]>,
    size: (u32, u32),
    hash: u64,
}

/// The §10 `last_frame` readback cache (m87): the most recent presented
/// frame's raster, keyed by the `pass_seq` that presented it. Served
/// without re-rendering; invalidated when a newer pass lands.
#[derive(Clone, Debug)]
struct FrameCache {
    seq: u64,
    frame: SceneFrame,
}

impl FrameCache {
    /// The cached `FrameRef` (an `Arc` clone — no pixel copy, §15.4).
    fn frame_ref(&self) -> FrameRef {
        FrameRef {
            stamp: FrameStamp {
                seq: self.seq,
                hash: self.frame.hash,
            },
            size: self.frame.size,
            pixels: Some(std::sync::Arc::clone(&self.frame.pixels)),
        }
    }
}

/// One scene capture: the real painted-quad readback (logical px) plus
/// the viewport it was taken at — the raster sizes from it.
#[derive(Debug)]
struct SceneCapture {
    quads: Vec<PaintedQuad>,
    viewport: (f64, f64),
}

/// One real scene quad → the engine's quad record (§4.5 capture, m87):
/// scaled pixels divide out the window scale, the solid background
/// rounds through the §10 pixel ramp, and the rect clips to the content
/// mask the GPU path clips to (`painted_quads` bounds are *not*
/// pre-clipped — the API doc's own caveat; a mask-less quad carries the
/// full viewport). A fully transparent quad paints nothing on the GPU —
/// the capture skips it the same way.
fn capture_quad(quad: &gpui::Quad, scale: f32) -> Option<PaintedQuad> {
    let solid = quad.background.as_solid()?;
    let rgba = solid.to_rgb();
    let alpha = rgba.a.clamp(0.0, 1.0) * 255.0;
    if alpha.round() == 0.0 {
        return None;
    }
    let mask = quad.content_mask.bounds;
    let (mx0, my0) = (mask.origin.x.0 / scale, mask.origin.y.0 / scale);
    let (mx1, my1) = (
        (mask.origin.x.0 + mask.size.width.0) / scale,
        (mask.origin.y.0 + mask.size.height.0) / scale,
    );
    let (qx0, qy0) = (
        quad.bounds.origin.x.0 / scale,
        quad.bounds.origin.y.0 / scale,
    );
    let (qx1, qy1) = (
        qx0 + quad.bounds.size.width.0 / scale,
        qy0 + quad.bounds.size.height.0 / scale,
    );
    let (x0, y0, x1, y1) = (qx0.max(mx0), qy0.max(my0), qx1.min(mx1), qy1.min(my1));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(PaintedQuad {
        x0,
        y0,
        x1,
        y1,
        color: rgba_bytes(rgba),
        radius: quad.corner_radii.top_left.0 / scale,
    })
}

/// The f32 channels → the §10 pixel ramp (round-then-clamp, the same
/// guarded conversion every adapter's quantization rides).
fn rgba_bytes(rgba: gpui::Rgba) -> [u8; 4] {
    let channel =
        |value: f32| -> u8 { (value.clamp(0.0, 1.0) * 255.0).round().max(0.0).min(255.0) as u8 };
    [
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        channel(rgba.a),
    ]
}

/// Rasterize one scene capture into a [`SceneFrame`] (§4.5 m87): the
/// quad list is the framework's readback, the surface is the engine's
/// source-over quad raster. Unsized captures present at the viewport;
/// a size override scales the logical coordinates onto the new surface
/// (the floem `PaintState::new` resize shape — same ops, new surface).
fn rasterize(capture: &SceneCapture, size: Option<(u32, u32)>) -> SceneFrame {
    let (width, height) = size.unwrap_or_else(|| {
        (
            (capture.viewport.0.round() as u32).max(1),
            (capture.viewport.1.round() as u32).max(1),
        )
    });
    let (sx, sy) = if capture.viewport.0 > 0.0 && capture.viewport.1 > 0.0 {
        (
            f64::from(width) / capture.viewport.0,
            f64::from(height) / capture.viewport.1,
        )
    } else {
        (1.0, 1.0)
    };
    let scaled: Vec<PaintedQuad> = capture
        .quads
        .iter()
        .map(|quad| PaintedQuad {
            x0: quad.x0 * sx,
            y0: quad.y0 * sy,
            x1: quad.x1 * sx,
            y1: quad.y1 * sy,
            color: quad.color,
            radius: quad.radius * sx.min(sy) as f32,
        })
        .collect();
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    composite_quads(&scaled, &mut pixels, width, height);
    let hash = FrameDigest::of(&pixels).value();
    SceneFrame {
        pixels: std::sync::Arc::from(pixels),
        size: (width, height),
        hash,
    }
}

/// The facts one fold collects from the app before any window update
/// runs (borrows of the app state never span a `update_window` call).
struct AppFacts {
    texts: Vec<Option<String>>,
    values: Vec<Option<String>>,
    input_focused: bool,
}

/// The gpui adapter (§6.1): the test-support hybrid — real
/// `dispatch_event` input, framework-recorded geometry folded into
/// synthesized AccessKit diffs, the seeded dispatcher clock, the
/// foreground-entity seam runner.
pub struct GpuiAdapter {
    meta: AdapterMeta,
    cx: TestAppContext,
    window: AnyWindowHandle,
    entity: gpui::Entity<CounterForm>,
    bindings: Vec<SemBinding>,
    hybrid: A11yHybrid,
    /// Minted `NodeId` per binding index (stable across passes; the
    /// bindings are boot-declared, so identity is boot-stable).
    nodes: Vec<NodeId>,
    /// The §13 overlay binding (m86): the active plan + the engine's
    /// overlay host — the present-time composite the real test-support
    /// capture routes (the host's quads land after the app's own in the
    /// same raster, so the overlay composites onto the presented pixels,
    /// §13). The host's draw log stays the §13 evidence surface.
    overlay_plan: Option<guiwright_inspect::overlay::OverlayPlan>,
    overlay_host: OverlayHost,
    /// The §10 `last_frame` readback cache (m87): the most recent
    /// presented frame's raster — the pixel row the video recorder and
    /// the screenshot baselines read (§6.2 gpui column, m87).
    frame_cache: Option<FrameCache>,
    size: (f64, f64),
    pointer: Point<Pixels>,
    pass_seq: u64,
    virtual_ms: u64,
    paused: bool,
}

impl GpuiAdapter {
    /// Boot the harness: build the seeded test context, open a real
    /// window on the real test platform running the counter+form
    /// entity, and take the first fold pass (the session settles from
    /// there).
    pub fn boot(window_size: (f64, f64), seed: u64) -> Result<Self, AdapterError> {
        let mut cx = TestAppContext::build(TestDispatcher::new(seed), None);
        let handle = cx.open_window(
            size(px(window_size.0 as f32), px(window_size.1 as f32)),
            CounterForm::new,
        );
        let entity = handle
            .entity(&cx)
            .map_err(|err| AdapterError::Internal(format!("gpui root entity: {err}")))?;
        let bindings = counter_bindings(&entity);
        let window = AnyWindowHandle::from(handle);
        let nodes = (0..=(bindings.len() as u64)).map(NodeId::from).collect();
        let mut adapter = Self {
            meta: AdapterMeta {
                name: "gpui",
                framework: "gpui",
                framework_version: "zed main (test-support)".to_string(),
                // The test-platform row of the §6.2 hybrid: no a11y
                // tree flows (the platform never activates it), the
                // synthesized-diff fallback covers resolve; pixels are
                // the m87 painted-quad capture (real scene readback,
                // driver-side raster).
                caps: Caps {
                    tree_update: false,
                    logical_snapshot: true,
                    input: true,
                    last_frame: true,
                    render_frame: true,
                    clock: true,
                },
            },
            cx,
            window,
            entity,
            bindings,
            hybrid: A11yHybrid::new(ActivationPolicy::Immediate),
            nodes,
            overlay_plan: None,
            overlay_host: OverlayHost::new(),
            frame_cache: None,
            size: window_size,
            pointer: point(px64(window_size.0 / 2.0), px64(window_size.1 / 2.0)),
            pass_seq: 0,
            virtual_ms: 0,
            paused: false,
        };
        adapter.fold_pass()?;
        Ok(adapter)
    }

    /// The bounds the framework recorded for one named element at
    /// paint — real laid-out geometry, read through the public test
    /// surface (`VisualTestContext::debug_bounds`; dev/test builds
    /// record selectors — the API's own doc names that gate).
    fn bounds_of(&self, test_id: &'static str) -> Result<[f64; 4], AdapterError> {
        let mut vt = gpui::VisualTestContext::from_window(self.window, &self.cx);
        let bounds = vt.debug_bounds(test_id).ok_or_else(|| {
            AdapterError::Internal(format!("no laid-out bounds recorded for `{test_id}`"))
        })?;
        Ok([
            bounds.origin.x.to_f64(),
            bounds.origin.y.to_f64(),
            bounds.origin.x.to_f64() + bounds.size.width.to_f64(),
            bounds.origin.y.to_f64() + bounds.size.height.to_f64(),
        ])
    }

    /// Collect the app-state facts (labels, values, the form field's
    /// real focus) without holding any borrow across a window update.
    fn app_facts(&mut self) -> AppFacts {
        let mut texts = Vec::with_capacity(self.bindings.len());
        let mut values = Vec::with_capacity(self.bindings.len());
        for binding in &self.bindings {
            texts.push(
                binding
                    .text
                    .as_ref()
                    .map(|read| read(&self.cx.app.borrow())),
            );
            values.push(
                binding
                    .value
                    .as_ref()
                    .map(|read| read(&self.cx.app.borrow())),
            );
        }
        let input_focused = self
            .cx
            .update_window(self.window, |_view, window, app| {
                self.entity.read(app).name_input_focus.is_focused(window)
            })
            .unwrap_or(false);
        AppFacts {
            texts,
            values,
            input_focused,
        }
    }

    /// One fold: refresh the diff the session reads, then one real draw
    /// (`Window::draw` — the framework's own render pipeline) whose
    /// painted-quad scene the capture rasterizes (§4.5, m87). The
    /// presented frame's seq is the fold's pass; the raster joins the
    /// §10 readback cache (`last_frame` serves it without a re-render).
    fn fold_pass(&mut self) -> Result<u64, AdapterError> {
        self.logical_snapshot();
        let capture = self.capture_scene()?;
        let frame = rasterize(&capture, None);
        let hash = frame.hash;
        self.pass_seq += 1;
        self.frame_cache = Some(FrameCache {
            seq: self.pass_seq,
            frame,
        });
        Ok(hash)
    }

    /// One real draw + the painted-quad readback (§4.5, m87):
    /// `Window::draw` re-runs the framework's render pipeline and
    /// assigns the frame the readback serves; quads land in paint order,
    /// clipped to their content masks and divided out of the window
    /// scale. The §13 overlay composites at present time — the host's
    /// quads land after the app's own (the m86 route, now in the
    /// raster).
    fn capture_scene(&mut self) -> Result<SceneCapture, AdapterError> {
        let mut quads: Vec<PaintedQuad> = Vec::new();
        let mut scale = 1.0_f32;
        let mut viewport = self.size;
        self.cx
            .update_window(self.window, |_view, window, app| {
                window.draw(app).clear(app);
                scale = window.scale_factor();
                let size = window.viewport_size();
                viewport = (size.width.to_f64(), size.height.to_f64());
                for quad in window.painted_quads() {
                    if let Some(captured) = capture_quad(&quad, scale) {
                        quads.push(captured);
                    }
                }
            })
            .map_err(|err| AdapterError::Internal(format!("gpui capture: {err}")))?;
        if let Some(plan) = self.overlay_plan.clone() {
            self.overlay_host.clear();
            quads.extend(self.overlay_host.paint_quads(&plan));
        }
        Ok(SceneCapture { quads, viewport })
    }

    /// Dispatch one `PlatformInput` through the canonical seam.
    fn dispatch(&mut self, input: PlatformInput) {
        self.cx
            .update_window(self.window, |_view, window, app| {
                window.dispatch_event(input, app);
            })
            .map_err(|err| AdapterError::Internal(format!("gpui dispatch: {err}")))
            .expect("gpui test window dispatch");
    }

    fn pointer_at(&mut self, x: f64, y: f64) {
        self.pointer = point(px64(x), px64(y));
        self.dispatch(PlatformInput::MouseMove(gpui::MouseMoveEvent {
            position: self.pointer,
            pressed_button: None,
            modifiers: Modifiers::default(),
        }));
    }

    /// Click at a point: move → down → up (the §9 gesture synthesis,
    /// executed through `Window::dispatch_event` — real hit-testing,
    /// real focus, real listeners).
    fn click_at(&mut self, x: f64, y: f64) {
        self.pointer_at(x, y);
        self.dispatch(PlatformInput::MouseDown(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position: self.pointer,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }));
        self.dispatch(PlatformInput::MouseUp(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position: self.pointer,
            modifiers: Modifiers::default(),
            click_count: 1,
        }));
    }

    fn keystroke(stroke: &KeyStroke) -> Keystroke {
        let mut modifiers = Modifiers::default();
        for modifier in &stroke.modifiers {
            match modifier {
                Modifier::Control => modifiers.control = true,
                Modifier::Alt => modifiers.alt = true,
                Modifier::Shift => modifiers.shift = true,
                Modifier::Platform => modifiers.platform = true,
                Modifier::Function => modifiers.function = true,
            }
        }
        Keystroke {
            key: stroke.key.clone(),
            modifiers,
            key_char: None,
        }
    }

    /// The binding + fresh geometry of a resolved node (stale nodes
    /// fail loud, §6.1 normalization).
    fn facts_of(&mut self, node: NodeId) -> Result<(usize, [f64; 4]), AdapterError> {
        let index = self
            .nodes
            .iter()
            .position(|minted| *minted == node)
            .filter(|index| *index > 0)
            .ok_or_else(|| AdapterError::Internal(format!("unknown node {node:?}")))?;
        let index = index - 1;
        let test_id = self
            .bindings
            .get(index)
            .ok_or_else(|| AdapterError::Internal(format!("node {node:?} has no binding")))?
            .test_id;
        let bounds = self.bounds_of(test_id)?;
        Ok((index, bounds))
    }
}

impl FrameworkAdapter for GpuiAdapter {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    /// No a11y tree flows on the test platform (crate docs) — the
    /// synthesized-diff fallback covers resolve.
    fn tree_update(&mut self) -> Option<TreeUpdate> {
        None
    }

    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome {
        let mut outcome = PumpOutcome::default();
        // The dispatcher drains queued tasks/timers; the draw runs the
        // real render pipeline (§5.1's frame-boundary marshalling).
        self.cx.run_until_parked();
        let mut digest = self.fold_pass().unwrap_or(0);
        outcome.frames.push(FrameStamp {
            seq: self.pass_seq,
            hash: digest,
        });
        while !deadline.expired() {
            self.cx.run_until_parked();
            let next = self.fold_pass().unwrap_or(0);
            outcome.frames.push(FrameStamp {
                seq: self.pass_seq,
                hash: next,
            });
            if next == digest {
                break;
            }
            digest = next;
        }
        outcome
    }

    /// The hybrid's fallback (§6.1): a synthesized AccessKit-shaped
    /// diff — roles from the app's declared bindings (gpui's own
    /// `role(...)` DSL), labels/values from the entity's real state,
    /// bounds from the framework's recorded paint geometry — fed
    /// through the engine's [`A11yHybrid`] bookkeeping.
    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        let facts = self.app_facts();
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        let mut test_ids: Vec<(u64, String)> = Vec::new();
        let mut focus = NodeId(0);

        let mut root = Node::new(accesskit::Role::Window);
        root.set_label("CounterForm");
        root.set_bounds(Rect {
            x0: 0.0,
            y0: 0.0,
            x1: self.size.0,
            y1: self.size.1,
        });
        nodes.push((NodeId(0), root));

        for (index, binding) in self.bindings.iter().enumerate() {
            let node_id = self.nodes[index + 1];
            let mut node = Node::new(binding.role);
            if let Some(text) = &facts.texts[index] {
                node.set_label(text.clone());
            }
            if let Some(value) = &facts.values[index] {
                node.set_value(value.clone());
            }
            if let Ok(bounds) = self.bounds_of(binding.test_id) {
                node.set_bounds(Rect {
                    x0: bounds[0],
                    y0: bounds[1],
                    x1: bounds[2],
                    y1: bounds[3],
                });
            }
            if binding.test_id == "name-input" && facts.input_focused {
                focus = node_id;
            }
            test_ids.push((node_id.0, binding.test_id.to_string()));
            node.set_children(Vec::new());
            nodes.push((node_id, node));
        }
        // The consumer tree validates parent/child consistency: the
        // root must list every binding's minted node.
        let child_ids: Vec<NodeId> = self.nodes[1..].to_vec();
        if let Some((_, root)) = nodes.first_mut() {
            root.set_children(child_ids);
        }

        let update = TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                root: NodeId(0),
                toolkit_name: Some("guiwright-gpui".to_string()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus,
        };
        let _ = self.hybrid.apply(&update);
        Some(LogicalTreeDiff {
            adapter: "gpui",
            update,
            removed: Vec::new(),
            test_ids,
            changed: true,
        })
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        let (index, bounds) = self.facts_of(node)?;
        let center = (
            bounds[0] + (bounds[2] - bounds[0]) / 2.0,
            bounds[1] + (bounds[3] - bounds[1]) / 2.0,
        );
        let detail = match action {
            SemAction::Click | SemAction::Check | SemAction::Uncheck => {
                self.click_at(center.0, center.1);
                self.cx.run_until_parked();
                "clicked".to_string()
            }
            SemAction::DoubleClick => {
                self.click_at(center.0, center.1);
                self.click_at(center.0, center.1);
                self.cx.run_until_parked();
                "double clicked".to_string()
            }
            SemAction::SetValue { text } => {
                // The second documented upstream blocker (m80): the real
                // `KeyDown` path panics inside zed's test scheduler —
                // `scheduler/src/test_scheduler.rs:232`,
                // `state.next_session_id.0 += 1` overflow —
                // reproducible with three real `Keystroke` dispatches
                // after a real focus click (the scheduler's session-id
                // accounting wraps on the keystroke path; click
                // dispatches never allocate there). The gate reports
                // the capability row honestly; the real fill sequence
                // (a real focus click, then real `Keystroke`s through
                // `Window::dispatch_event` into the focused element's
                // `on_key_down`) is specified in the crate docs and
                // waits on the upstream scheduler fix.
                let _ = text;
                return Err(AdapterError::Unsupported(
                    "fill (zed's test scheduler overflows its session-id counter \
                     during real KeyDown dispatch — \
                     scheduler/src/test_scheduler.rs:232 — reached after a real \
                     focus click + three Keystroke dispatches)",
                ));
            }
            // The m89 select vocabulary rides the same documented
            // blocker the fill path names (m80): selecting an option on
            // a combobox drives the same keystroke-dispatch pipeline
            // the scheduler cannot account headless.
            SemAction::SelectOption { .. } => {
                return Err(AdapterError::Unsupported(
                    "select_option (same zed test-scheduler session-id overflow \
                     as fill — scheduler/src/test_scheduler.rs:232)",
                ));
            }
            // The m90 scroll-into-view vocabulary: the test-support
            // dispatch surface carries no scroll-into-view request —
            // loud, like the other framework-native vocabulary here.
            SemAction::ScrollIntoView => {
                return Err(AdapterError::Unsupported(
                    "scroll_into_view_if_needed (no scroll-into-view dispatch \
                     on the test-support event surface)",
                ));
            }
            SemAction::Focus => {
                // gpui focuses tracked elements on click (the real
                // focus path; there is no focus-without-activate
                // dispatch).
                self.click_at(center.0, center.1);
                self.cx.run_until_parked();
                "focused via click".to_string()
            }
            SemAction::Blur => {
                return Err(AdapterError::Unsupported(
                    "blur (gpui has no unfocus dispatch; focus moves by clicking elsewhere)",
                ));
            }
            SemAction::Hover => {
                self.pointer_at(center.0, center.1);
                self.cx.run_until_parked();
                "hovered".to_string()
            }
            SemAction::Press { key, modifiers } => {
                let stroke = KeyStroke {
                    modifiers: modifiers
                        .iter()
                        .filter_map(|name| Modifier::parse(name))
                        .collect(),
                    key: key.to_ascii_lowercase(),
                };
                let keystroke = Self::keystroke(&stroke);
                self.dispatch(PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                }));
                self.dispatch(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
                self.cx.run_until_parked();
                format!("pressed {key}")
            }
            SemAction::Scroll { dx, dy } => {
                self.dispatch(PlatformInput::Scroll(ScrollEvent {
                    position: self.pointer,
                    delta: ScrollDelta::Pixels(point(px64(dx), px64(dy))),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                    source: ScrollSource::Pointer,
                }));
                self.cx.run_until_parked();
                format!("scrolled ({dx},{dy})")
            }
        };
        self.logical_snapshot();
        self.pass_seq += 1;
        Ok(ActionOutcome {
            applied_at_frame: self.pass_seq,
            detail,
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        for step in translate(ev) {
            match step {
                GpuiInputStep::MouseMove { x, y } => {
                    self.pointer_at(x, y);
                }
                GpuiInputStep::MouseDown { x, y, button } => {
                    self.pointer_at(x, y);
                    self.dispatch(PlatformInput::MouseDown(gpui::MouseDownEvent {
                        button: gpui_button(button),
                        position: self.pointer,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }));
                }
                GpuiInputStep::MouseUp { x, y, button } => {
                    self.pointer_at(x, y);
                    self.dispatch(PlatformInput::MouseUp(gpui::MouseUpEvent {
                        button: gpui_button(button),
                        position: self.pointer,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }));
                }
                GpuiInputStep::Scroll { dx, dy } => {
                    self.dispatch(PlatformInput::Scroll(ScrollEvent {
                        position: self.pointer,
                        delta: ScrollDelta::Pixels(point(px64(dx), px64(dy))),
                        modifiers: Modifiers::default(),
                        touch_phase: TouchPhase::Moved,
                        source: ScrollSource::Pointer,
                    }));
                }
                GpuiInputStep::KeyDown { keystroke } => {
                    self.dispatch(PlatformInput::KeyDown(KeyDownEvent {
                        keystroke: Self::keystroke(&keystroke),
                        is_held: false,
                        prefer_character_input: false,
                    }));
                }
                GpuiInputStep::KeyUp { keystroke } => {
                    self.dispatch(PlatformInput::KeyUp(KeyUpEvent {
                        keystroke: Self::keystroke(&keystroke),
                    }));
                }
                GpuiInputStep::ImeCommit { text } => {
                    // The current `PlatformInput` carries no IME-commit
                    // variant; text rides real `KeyDown`/`KeyUp`
                    // keystrokes (the documented adaptation — the
                    // characters still enter the real listener).
                    for ch in text.chars() {
                        let stroke = KeyStroke {
                            modifiers: Vec::new(),
                            key: ch.to_string(),
                        };
                        let keystroke = Self::keystroke(&stroke);
                        self.dispatch(PlatformInput::KeyDown(KeyDownEvent {
                            keystroke: keystroke.clone(),
                            is_held: false,
                            prefer_character_input: false,
                        }));
                        self.dispatch(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
                    }
                }
            }
        }
        self.cx.run_until_parked();
        Ok(())
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        for step in clock_plan(cmd) {
            match step {
                GpuiClockStep::DispatchRound { advance_ms } => {
                    // The seeded virtual scheduler: advance the clock
                    // and drain everything scheduled before the new
                    // deadline (§5.1's GPUI waker row).
                    self.cx
                        .dispatcher
                        .advance_clock(Duration::from_millis(advance_ms));
                    self.cx.run_until_parked();
                    self.virtual_ms = self.virtual_ms.saturating_add(advance_ms);
                }
                // Install happens at app build time (the seeded
                // dispatcher IS the virtual scheduler); pause/resume
                // gate the driver's run_for.
                GpuiClockStep::MarkInstalled => {}
                GpuiClockStep::MarkPaused { paused } => self.paused = paused,
            }
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_millis(self.virtual_ms)),
            paused: self.paused,
        })
    }

    /// The §10 `last_frame` readback (m87): the most recent presented
    /// frame's pixels, without disturbing app state — the pixel row the
    /// video recorder and the screenshot baselines ride (§6.2 gpui
    /// column, m87). Served from the per-pass cache when it is fresh.
    fn last_frame(&mut self) -> Option<FrameRef> {
        if let Some(cache) = self.frame_cache.as_ref()
            && cache.seq == self.pass_seq
        {
            return Some(cache.frame_ref());
        }
        let capture = self.capture_scene().ok()?;
        let frame = rasterize(&capture, None);
        self.pass_seq += 1;
        self.frame_cache = Some(FrameCache {
            seq: self.pass_seq,
            frame: frame.clone(),
        });
        Some(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash: frame.hash,
            },
            size: frame.size,
            pixels: Some(frame.pixels),
        })
    }

    /// Render on demand (m87): one fresh draw + readback, rasterized at
    /// the requested pixel size (the scene's logical coordinates scale
    /// to the override — the floem `PaintState::new` resize shape) or at
    /// the viewport when unsized.
    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        let capture = self.capture_scene()?;
        let frame = rasterize(&capture, size);
        self.pass_seq += 1;
        self.frame_cache = Some(FrameCache {
            seq: self.pass_seq,
            frame: frame.clone(),
        });
        Ok(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash: frame.hash,
            },
            size: frame.size,
            pixels: Some(frame.pixels),
        })
    }

    /// Inspector overlay (§13, m86 → m87): store the plan — every
    /// capture composites the host's quads fresh on the present path
    /// (over the app's own painted scene), so stale scenes never
    /// accumulate and the overlay lands in the presented pixels. `None`
    /// clears the scene (the `DisableOverlay` reset); the host's draw
    /// log stays the §13 evidence surface.
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

    /// State seams (§14): the task runs inside the root entity — the
    /// runner context is `&mut CounterForm` (documented per the trait's
    /// contract).
    fn run_in(&mut self, task: SeamTask) -> Result<String, AdapterError> {
        self.cx.update(|app| {
            self.entity.update(app, |state, _| {
                task(state as &mut dyn std::any::Any).map_err(AdapterError::Internal)
            })
        })
    }
}

fn gpui_button(button: GpuiButton) -> MouseButton {
    match button {
        GpuiButton::Left => MouseButton::Left,
        GpuiButton::Right => MouseButton::Right,
        GpuiButton::Middle => MouseButton::Middle,
    }
}

impl std::fmt::Debug for GpuiAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuiAdapter")
            .field("meta", &self.meta)
            .field("pass_seq", &self.pass_seq)
            .field("virtual_ms", &self.virtual_ms)
            .field("paused", &self.paused)
            .field("bindings", &self.bindings.len())
            .finish_non_exhaustive()
    }
}
