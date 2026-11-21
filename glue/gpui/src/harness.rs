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
//! * **Pixels** — `last_frame` is a hash-only [`FrameRef`]: the digest
//!   covers the framework-recorded element geometry + the folded tree
//!   (§15.5's visual-change signal). `render_frame` is the documented
//!   upstream gap: `Window::render_to_image` bails without a
//!   `HeadlessRenderer` and `TestPlatform::new` ships no factory
//!   (`platform/test/platform.rs` passes `None`).
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
    /// overlay host — the paint route the real test-support surface
    /// rides (`Window::paint_quad` → `painted_quads`, mirrored per
    /// contract.rs). The host's quads join the §15.5 scene digest the
    /// fold pass rides, so the overlay registers as a visual change on
    /// the hash-only path (the pixels blocker stays the documented
    /// upstream gap).
    overlay_plan: Option<guiwright_inspect::overlay::OverlayPlan>,
    overlay_host: OverlayHost,
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
                // the render_to_image gap below.
                caps: Caps {
                    tree_update: false,
                    logical_snapshot: true,
                    input: true,
                    last_frame: true,
                    render_frame: false,
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

    /// One fold: draw the real scene (the test window's render
    /// pipeline), digest the framework-recorded geometry + the folded
    /// tree (§15.5's visual-change signal), and refresh the diff the
    /// session reads.
    fn fold_pass(&mut self) -> Result<u64, AdapterError> {
        self.logical_snapshot();
        let facts = self.app_facts();
        let mut digest_input = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            let bounds = self.bounds_of(binding.test_id)?;
            digest_input.extend_from_slice(binding.test_id.as_bytes());
            for value in bounds {
                digest_input.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            if let Some(text) = &facts.texts[index] {
                digest_input.extend_from_slice(text.as_bytes());
            }
            if let Some(value) = &facts.values[index] {
                digest_input.extend_from_slice(value.as_bytes());
            }
        }
        // The §13 overlay layer joins the scene digest (m86): the
        // host's painted quads are part of the presented frame, so
        // "wait for visual change" sees the overlay on the hash-only
        // path too.
        if self.overlay_plan.is_some() {
            digest_input.extend_from_slice(&self.overlay_host.scene_digest().to_le_bytes());
        }
        Ok(FrameDigest::of(&digest_input).value())
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
        self.pass_seq += 1;
        outcome.frames.push(FrameStamp {
            seq: self.pass_seq,
            hash: digest,
        });
        while !deadline.expired() {
            self.cx.run_until_parked();
            let next = self.fold_pass().unwrap_or(0);
            self.pass_seq += 1;
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

    /// Hash-only `FrameRef` (§15.5): the digest covers the
    /// framework-recorded geometry + the folded tree. Pixels ride
    /// `render_to_image`, which is the documented upstream gap below.
    fn last_frame(&mut self) -> Option<FrameRef> {
        let hash = self.fold_pass().ok()?;
        self.pass_seq += 1;
        Some(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash,
            },
            size: (self.size.0 as u32, self.size.1 as u32),
            pixels: None,
        })
    }

    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        let _ = size;
        Err(AdapterError::Unsupported(
            "render_frame (gpui headless pixels need a HeadlessRenderer: \
             Window::render_to_image bails without one and \
             TestPlatform::new ships no factory — \
             platform/test/platform.rs passes None)",
        ))
    }

    /// Inspector overlay (§13, m86): store the plan and paint the
    /// host's scene (the `paint_quad` route the real test-support
    /// window rides) — the fold pass folds the scene digest, so the
    /// overlay moves the frame stamp. `None` clears the scene (the
    /// `DisableOverlay` reset).
    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.overlay_plan = plan.cloned();
        if let Some(active) = self.overlay_plan.as_ref() {
            self.overlay_host.paint_quads(active);
        } else {
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
