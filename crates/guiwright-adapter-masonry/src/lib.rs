//! Guiwright `FrameworkAdapter` for Masonry/Xilem (architecture §4.2, M0
//! embedded scope).
//!
//! Wraps [`masonry_testing::TestHarness`] — the windowless, byte-deterministic
//! harness (§11 "`RenderRoot` + `vello_cpu`, windowless by construction"). One
//! pump = one virtual `AnimFrame` tick + one `vello_cpu` pass, which also
//! refreshes the AccessKit tree.
//!
//! Level-1 facts this adapter relies on (verified against masonry 0.4):
//! * `WidgetId → accesskit::NodeId` is **identity** (`From<WidgetId>`);
//!   masonry itself converts `ActionRequest.target` back to a `WidgetId`
//!   internally, so semantic actions pass the resolved `NodeId` straight
//!   through (no `unsafe`, no transmute in Guiwright code — §4.2).
//! * `harness.render()` runs `render_root.redraw()` and folds the resulting
//!   `TreeUpdate` into the harness's consumer tree; the adapter re-serializes
//!   it into a full `TreeUpdate` for the driver's own consumer tree
//!   (full-rebuild cost is absorbed by consumer diffing, §15.2).
//! * The virtual clock is `animate_ms` (§14: synthetic `AnimFrame` ticks).
//! * Bounds emitted by `run_accessibility_pass` are physical px at the
//!   harness's scale factor (1.0 by default), matching AccessKit convention.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use accesskit::NodeId;
use accesskit::TreeUpdate;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameworkAdapter, InputEvent, PumpOutcome, SemAction,
};
use masonry_core::accesskit as m21;
use masonry_core::core::{DefaultProperties, NewWidget, TextEvent, Widget};
use masonry_core::kurbo::{Point, Vec2};
use masonry_core::ui_events::keyboard::{Code, Key, KeyboardEvent};
use masonry_core::ui_events::pointer::PointerButton;
use masonry_testing::{TestHarness, TestHarnessParams};
use std::hash::Hasher;
use twox_hash::xxhash3_64::Hasher as Xxh3;

/// One pump's virtual frame duration (60 fps).
const FRAME_MS: u64 = 16;

/// One consumer-tree walk → flattened masonry-node list (§15.2: the
/// driver's consumer diffs the normalized update at O(changes)).
fn walk_tree(
    state: &accesskit_consumer::TreeState,
    id: m21::NodeId,
    nodes: &mut Vec<(m21::NodeId, m21::Node)>,
) {
    let Some(node) = state.node_by_id(id) else {
        return;
    };
    nodes.push((id, node.data().clone()));
    for child in node.child_ids() {
        walk_tree(state, child, nodes);
    }
}

/// Windowless masonry adapter (§4.2 "Embed (headless)").
pub struct MasonryAdapter<W: Widget> {
    harness: TestHarness<W>,
    meta: AdapterMeta,
    /// Serialized full tree update from the last pass.
    pending_tree_update: Option<TreeUpdate>,
    /// Cached RGBA pixels of the last pass (backing `last_frame`).
    last_pixels: Option<Arc<[u8]>>,
    pixel_size: (u32, u32),
    frame_seq: u64,
    clock_paused: bool,
    /// False when masonry's wgpu rasterizer has no device (GPU-less host,
    /// §11: CI provisions lavapipe). Tree refresh works regardless.
    gpu_available: bool,
}

impl<W: Widget> std::fmt::Debug for MasonryAdapter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MasonryAdapter")
            .field("meta", &self.meta)
            .field("frame_seq", &self.frame_seq)
            .finish_non_exhaustive()
    }
}

mod normalize;
pub mod overlay;

impl<W: Widget> MasonryAdapter<W> {
    /// Create the adapter with the given root widget (default 400×400
    /// window, scale factor 1.0).
    pub fn new(root: NewWidget<W>) -> Self {
        Self::create_with(
            DefaultProperties::default(),
            root,
            TestHarnessParams::default(),
        )
    }

    /// Create the adapter with an explicit window size (logical px).
    pub fn new_with_size(root: NewWidget<W>, width: f64, height: f64) -> Self {
        let harness = TestHarness::create_with_size(
            DefaultProperties::default(),
            root,
            masonry_core::kurbo::Size::new(width, height),
        );
        Self::from_harness(harness)
    }

    /// Create the adapter from full harness params (crate-internal; the
    /// params type is `#[non_exhaustive]` upstream).
    #[doc(hidden)]
    pub fn create_with(
        props: DefaultProperties,
        root: NewWidget<W>,
        params: TestHarnessParams,
    ) -> Self {
        let harness = TestHarness::create_with(props, root, params);
        Self::from_harness(harness)
    }

    pub fn from_harness(harness: TestHarness<W>) -> Self {
        let mut me = Self {
            harness,
            meta: AdapterMeta {
                name: "masonry",
                framework: "masonry",
                framework_version: env!("CARGO_PKG_VERSION").to_string(),
                caps: Caps {
                    tree_update: true,
                    logical_snapshot: false,
                    input: true,
                    last_frame: true,
                    render_frame: true,
                    clock: true,
                },
            },
            pending_tree_update: None,
            last_pixels: None,
            pixel_size: (0, 0),
            frame_seq: 0,
            clock_paused: false,
            gpu_available: true,
        };
        me.refresh();
        me
    }

    pub fn harness(&self) -> &TestHarness<W> {
        &self.harness
    }

    /// Does masonry's rasterizer have a device on this host?
    pub fn gpu_available(&self) -> bool {
        self.gpu_available
    }

    pub fn harness_mut(&mut self) -> &mut TestHarness<W> {
        &mut self.harness
    }

    /// Run one pass: refresh the a11y tree, rasterize pixels, rebuild the
    /// registry. This is the masonry frame boundary.
    fn refresh(&mut self) -> u64 {
        // masonry 0.4 rasterizes through wgpu+vello and *panics* when no
        // device exists (GPU-less host). Crucially, `render()` refreshes the
        // harness's AccessKit tree **before** touching the device, so a
        // caught panic still leaves the locator stack fully functional
        // (§11: CI provisions lavapipe for real pixels).
        let mut pixels_hash = 0u64;
        let mut harness = std::panic::AssertUnwindSafe(&mut self.harness);
        if let Ok(img) = std::panic::catch_unwind(move || harness.render()) {
            self.gpu_available = true;
            self.pixel_size = (img.width(), img.height());
            pixels_hash = {
                let mut hasher = Xxh3::default();
                // Strided sample of pixel rows (§15.5: full hash on the
                // writer thread, strided here for the cheap signal).
                let stride = 4 * img.width().max(1) as usize;
                let bytes = img.as_raw();
                for chunk in bytes.chunks(stride.max(1)) {
                    hasher.write(chunk);
                }
                hasher.finish()
            };
            self.last_pixels = Some(img.into_raw().into());
        } else {
            self.gpu_available = false;
            self.last_pixels = None;
            // Stable digest: visual-change signaling rides the tree
            // diffs on GPU-less hosts (M0 scope).
        }

        // Full TreeUpdate re-serialization from the harness's consumer tree
        // (masonry's accesskit 0.21 dialect; §15.2: the driver's consumer
        // diffs it at O(changes)), normalized into the core dialect (§5.2).
        // Fresh in both branches: the consumer tree updates before
        // rasterization.
        let tree = self.harness.access_tree();
        let state = tree.state();
        let mut nodes: Vec<(m21::NodeId, m21::Node)> = Vec::with_capacity(64);
        walk_tree(state, state.root_id(), &mut nodes);
        normalize::fixup_accessible_names(&mut nodes);
        let update_raw = m21::TreeUpdate {
            nodes,
            tree: Some(m21::Tree::new(state.root_id())),
            focus: state.focus_id_in_tree(),
        };
        self.pending_tree_update = Some(normalize::normalize_update(update_raw));
        pixels_hash
    }
}

impl<W: Widget> FrameworkAdapter for MasonryAdapter<W> {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // Virtual clock tick (§14): one AnimFrame fires timers + animations.
        if !self.clock_paused {
            self.harness.animate_ms(FRAME_MS);
        }
        self.frame_seq += 1;
        let hash = self.refresh();
        // The harness is synchronous: injected work is already applied, so
        // the app never requests more frames on its own (settle = 2 pumps).
        PumpOutcome {
            frames: vec![guiwright_driver::adapter::FrameStamp {
                seq: self.frame_seq,
                hash,
            }],
            repaint_requested: false,
            quit_requested: false,
        }
    }

    fn tree_update(&mut self) -> Option<TreeUpdate> {
        self.pending_tree_update.take()
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        // §8.4: prefer the framework's own semantics — AccessKit
        // ActionRequests through `process_access_event`; masonry maps the
        // NodeId back to its WidgetId internally (identity, §4.2). The
        // trait hands us a core-dialect (0.25) NodeId; ids are identical
        // numbers in the 0.21 dialect.
        let target_widget = m21::NodeId(node.0);
        match &action {
            SemAction::Click | SemAction::Check | SemAction::Uncheck | SemAction::DoubleClick => {
                self.harness.process_access_event(m21::ActionRequest {
                    action: m21::Action::Click,
                    target: target_widget,
                    data: None,
                });
            }
            SemAction::SetValue { text } => {
                self.harness.process_access_event(m21::ActionRequest {
                    action: m21::Action::SetValue,
                    target: target_widget,
                    data: Some(m21::ActionData::Value(text.clone().into_boxed_str())),
                });
            }
            SemAction::Focus => {
                self.harness.process_access_event(m21::ActionRequest {
                    action: m21::Action::Focus,
                    target: target_widget,
                    data: None,
                });
            }
            SemAction::Blur => {
                self.harness.process_access_event(m21::ActionRequest {
                    action: m21::Action::Blur,
                    target: target_widget,
                    data: None,
                });
            }
            SemAction::Press { key, .. } => {
                // Keyboard rides the real text-event pipeline (§9).
                let (k, c) = translate_key(key)?;
                self.harness
                    .process_text_event(TextEvent::Keyboard(KeyboardEvent::key_down(k.clone(), c)));
                self.harness
                    .process_text_event(TextEvent::Keyboard(KeyboardEvent::key_up(k, c)));
            }
            // Hover/Scroll are real-input actions the session routes
            // through `input()`; reaching `act` means no bounds.
            SemAction::Hover | SemAction::Scroll { .. } => {
                return Err(AdapterError::Internal(
                    "hover/scroll need bounds; the session routes them through input()".into(),
                ));
            }
        }
        Ok(ActionOutcome {
            applied_at_frame: self.frame_seq,
            detail: format!("queued {}", action.describe()),
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        match ev {
            InputEvent::PointerMove { x, y } => {
                self.harness.mouse_move(Point::new(x, y));
            }
            InputEvent::PointerDown { button, .. } => {
                self.harness.mouse_button_press(translate_button(button));
            }
            InputEvent::PointerUp { button, .. } => {
                self.harness.mouse_button_release(translate_button(button));
            }
            InputEvent::Wheel { dx, dy } => {
                self.harness.mouse_wheel(Vec2::new(dx, dy));
            }
            InputEvent::TextInput { text } => {
                // masonry harness commits via IME (§4.2 keyboard fidelity
                // caveat) — the sanctioned path for synthetic text.
                self.harness.keyboard_type_chars(&text);
            }
            InputEvent::KeyDown { key, .. } => {
                let (k, c) = translate_key(&key)?;
                self.harness
                    .process_text_event(TextEvent::Keyboard(KeyboardEvent::key_down(k, c)));
            }
            InputEvent::KeyUp { key, .. } => {
                let (k, c) = translate_key(&key)?;
                self.harness
                    .process_text_event(TextEvent::Keyboard(KeyboardEvent::key_up(k, c)));
            }
        }
        Ok(())
    }

    fn last_frame(&mut self) -> Option<FrameRef> {
        self.last_pixels.as_ref().map(|pixels| FrameRef {
            stamp: guiwright_driver::adapter::FrameStamp {
                seq: self.frame_seq,
                hash: 0,
            },
            size: self.pixel_size,
            pixels: Some(Arc::clone(pixels)),
        })
    }

    fn render_frame(&mut self, _size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        // vello renders on demand (§11); needs a device (GPU or lavapipe).
        let _ = self.refresh();
        if !self.gpu_available {
            return Err(AdapterError::Unsupported(
                "no wgpu device on this host (install lavapipe per §11 for headless pixels)",
            ));
        }
        self.last_frame()
            .ok_or(AdapterError::Unsupported("render_frame"))
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        match cmd {
            // Already virtual (§14 masonry row); Probe reads via the reply.
            ClockCmd::InstallVirtual | ClockCmd::Probe => {}
            ClockCmd::FastForward(dt) => {
                // Advance in frame-sized steps so rewrite passes and timers
                // settle at each frame (§14 "fast_forward(dt) = advance in
                // frame-sized steps until drained").
                let ms = dt.as_millis() as u64;
                let mut remaining = ms;
                while remaining > 0 {
                    let step = remaining.min(FRAME_MS);
                    self.harness.animate_ms(step);
                    remaining -= step;
                }
            }
            ClockCmd::RunFor(dt) => self.harness.animate_ms(dt.as_millis() as u64),
            ClockCmd::SetTime(t) => {
                // Absolute time: jump forward by the delta from the
                // approximate current virtual time (the harness tracks a
                // plain tick counter, §14 masonry row).
                let current = Duration::from_millis(self.frame_seq.saturating_mul(FRAME_MS));
                if let Some(delta) = t.checked_sub(current) {
                    self.harness.animate_ms(delta.as_millis() as u64);
                }
            }
            ClockCmd::Pause => self.clock_paused = true,
            ClockCmd::Resume => self.clock_paused = false,
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_millis(
                // Approximate ledger: one FRAME_MS per pump since attach.
                self.frame_seq.saturating_mul(FRAME_MS),
            )),
            paused: self.clock_paused,
        })
    }
}

fn translate_button(button: InputEventButton) -> PointerButton {
    match button {
        InputEventButton::Primary => PointerButton::Primary,
        InputEventButton::Secondary => PointerButton::Secondary,
        InputEventButton::Middle => PointerButton::Auxiliary,
    }
}

use guiwright_driver::adapter::PointerButton as InputEventButton;

fn translate_key(key: &str) -> Result<(Key, Code), AdapterError> {
    use masonry_core::ui_events::keyboard::NamedKey;
    let named = match key {
        "Enter" => (Key::Named(NamedKey::Enter), Code::Enter),
        "Escape" => (Key::Named(NamedKey::Escape), Code::Escape),
        "Tab" => (Key::Named(NamedKey::Tab), Code::Tab),
        "Backspace" => (Key::Named(NamedKey::Backspace), Code::Backspace),
        "Delete" => (Key::Named(NamedKey::Delete), Code::Delete),
        "ArrowUp" | "Up" => (Key::Named(NamedKey::ArrowUp), Code::ArrowUp),
        "ArrowDown" | "Down" => (Key::Named(NamedKey::ArrowDown), Code::ArrowDown),
        "ArrowLeft" | "Left" => (Key::Named(NamedKey::ArrowLeft), Code::ArrowLeft),
        "ArrowRight" | "Right" => (Key::Named(NamedKey::ArrowRight), Code::ArrowRight),
        "Home" => (Key::Named(NamedKey::Home), Code::Home),
        "End" => (Key::Named(NamedKey::End), Code::End),
        "PageUp" => (Key::Named(NamedKey::PageUp), Code::PageUp),
        "PageDown" => (Key::Named(NamedKey::PageDown), Code::PageDown),
        other => {
            // Single characters become character keys.
            let mut chars = other.chars();
            let (Some(first), None) = (chars.next(), chars.next()) else {
                return Err(AdapterError::Internal(format!("unknown key {other}")));
            };
            (Key::Character(first.to_string()), Code::Unidentified)
        }
    };
    Ok(named)
}
