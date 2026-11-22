//! The floem glue (architecture §4.7, §17 row 6, m80):
//! [`FrameworkAdapter`] over the **real** `lapce/floem`
//! `HeadlessHarness`.
//!
//! The engine (`guiwright-adapter-floem`, main workspace) stays
//! framework-free; this crate is the framework half, scoped to its own
//! workspace by the recorded floem constraint (crate docs of
//! `guiwright-adapter-floem`: floem 0.2.0 predates the harness, so the
//! binding rides the pinned `lapce/floem@1351ffb` revision, and floem's
//! Linux graph never enters the main workspace). Everything here rides
//! the verified public surface of floem at that revision:
//!
//! * **Boot** — [`floem::headless::TestRoot`] claims the thread's root
//!   view, the app factory builds the real view tree against it, and
//!   [`HeadlessHarness::new_with_size`] wraps it in the headless
//!   `WindowHandle` (whose `PaintState` "will never initialize" —
//!   `window/handle.rs` `new_headless` — so the harness is tree+input
//!   real and pixel-free by design).
//! * **Tree** — the [`ViewId`] walk (`children`, `debug_name`,
//!   `get_visual_rect`) folds into [`FloemLogicalTree`]: kind from the
//!   view type name, the §8.1 test id from floem's own `debug_name`
//!   decorator (the engine's [`TestIdRegistry`] is fed from it — the
//!   upstream `test_id(...)` decorator is its thin half), bounds from
//!   the real layout, focus from [`HeadlessHarness::is_focused`].
//! * **Input** — [`HeadlessHarness::dispatch_event`] with real
//!   `Event::Pointer` / `Event::Key` payloads (the same pipeline a live
//!   window routes, §4.7 "headless = `dispatch_event`"); clicks land
//!   through real hit-testing (`element_id_at`), fills type real
//!   `KeyboardEvent`s into the real editor buffer.
//! * **Pixels** — the engine's tiny-skia [`PaintState`] rasterizes the
//!   real laid-out view geometry into [`PaintOp`]s (§4.7 contribution
//!   2's driver-side half; the upstream patch routing floem's display
//!   list would make it bit-identical to the GPU scene, and the frame
//!   digest is the §15.5 strided xxh3). `last_frame` serves the most
//!   recent presented raster from a per-pass readback cache and
//!   `render_frame` re-rasters on demand — the §10 video + baseline
//!   rows ride real pixels (m87, the m86 iced/freya shape).
//! * **Clock** — the harness-timer plan ([`guiwright_adapter_floem::clock`]):
//!   each tick drains the harness's update queue at a virtual deadline.
//!
//! Known seams encoded here (§9/§18, each named at its use site):
//! floem exposes no text getters on core views at this revision, so the
//! *text/value* facts ride the app's real reactive state through
//! [`SemReader`]s (the same signals the views render from — the
//! documented upstream-contribution stub); disabled-state and blur have
//! no dispatch surface headless (they resolve to `Unsupported`).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use accesskit::NodeId;
use floem::event::Event;
use floem::headless::{HeadlessHarness, TestRoot};
use floem::view::{AnyView, ViewId};
use guiwright_adapter_floem::clock::{FloemClockStep, plan as clock_plan};
use guiwright_adapter_floem::input::{FloemEvent, translate};
use guiwright_adapter_floem::paint::{Color, PaintOp, PaintState};
use guiwright_adapter_floem::{FloemLogicalTree, OverlayHost, TestIdRegistry, ViewReport};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PumpOutcome, SemAction,
};
use guiwright_inspect::overlay::OverlayPlan;
use ui_events::keyboard::{Code, Key, KeyState, KeyboardEvent, Modifiers, NamedKey};

/// The mock gap between press and release of the double-click pair
/// (floem tracks click counts from the event's own `count`, so no gap is
/// needed — the constant documents the single-tap pacing instead).
const CLICK_GAP_MS: u64 = 0;

/// One rasterized frame (§10): the RGBA raster, its pixel size and the
/// §15.5 strided-surface digest — the unit the `last_frame` readback
/// cache stores (the m86 pixel row: iced and freya ride the same shape).
#[derive(Clone, Debug)]
struct RasterFrame {
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
    frame: RasterFrame,
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

/// Facts the latest introspection pass recorded per view handle — the
/// action-time geometry (`act` resolves a minted `NodeId` back to the
/// view's laid-out rect) and the editor length a fill clears.
#[derive(Clone, Copy, Debug)]
struct ItemFacts {
    center: (f64, f64),
    right_edge: (f64, f64),
    value_len: usize,
}

/// The app-declared reader map: test id → (text reader, value reader).
type ReaderMap = HashMap<
    String,
    (
        Option<Rc<dyn Fn() -> String>>,
        Option<Rc<dyn Fn() -> String>>,
    ),
>;

/// One app-declared fact reader (the §4.7 contribution-1b seam): floem
/// exposes no text getters on core views at the pinned revision, so the
/// *text/value* facts ride the app's real reactive state — the same
/// signals the views render from. Upstream patch 1b (a `text()` getter
/// on core views) is what would retire this seam; the rest of the walk
/// (kind, test ids, bounds, focus, children) is framework-real.
pub struct SemReader {
    /// The §8.1 test id of the view the fact belongs to (the floem
    /// `debug_name` decorator argument).
    pub test_id: String,
    kind: SemKind,
    read: Rc<dyn Fn() -> String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SemKind {
    Text,
    Value,
}

impl SemReader {
    /// A text-fact reader (the accessible-name fallback).
    pub fn text(test_id: impl Into<String>, read: impl Fn() -> String + 'static) -> Self {
        Self {
            test_id: test_id.into(),
            kind: SemKind::Text,
            read: Rc::new(read),
        }
    }

    /// A value-fact reader (editable content: the editor buffer).
    pub fn value(test_id: impl Into<String>, read: impl Fn() -> String + 'static) -> Self {
        Self {
            test_id: test_id.into(),
            kind: SemKind::Value,
            read: Rc::new(read),
        }
    }
}

impl std::fmt::Debug for SemReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemReader")
            .field("test_id", &self.test_id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// The floem adapter (§6.1): Level-2 — the engine folds real `ViewId`
/// walks into synthesized AccessKit diffs; input rides the real
/// harness event pipeline; pixels come from the engine's tiny-skia
/// `PaintState` over real view geometry; time is the harness-timer plan.
pub struct FloemAdapter {
    meta: AdapterMeta,
    harness: HeadlessHarness,
    engine: FloemLogicalTree,
    test_ids: TestIdRegistry,
    paint: PaintState,
    size: (f64, f64),
    facts: HashMap<u64, ItemFacts>,
    /// The app-declared fact source (see [`SemReader`]).
    readers: ReaderMap,
    /// The §13 overlay binding (m86): the active plan + the engine's
    /// overlay host — the present-path hook the real floem surface
    /// routes (`WindowHandle` post-paint → PaintOps, mirrored per
    /// contract.rs). `render_frame` extends its scene with the host's
    /// ops, so the overlay composites onto the presented pixels.
    overlay_plan: Option<OverlayPlan>,
    overlay_host: OverlayHost,
    /// The §10 `last_frame` readback cache (m87): the most recent
    /// presented frame's raster — the pixel row the video recorder and
    /// the screenshot baselines read (§6.2 floem column, m87).
    frame_cache: Option<FrameCache>,
    pointer: (f64, f64),
    pass_seq: u64,
    virtual_ms: u64,
    paused: bool,
}

impl FloemAdapter {
    /// Boot a harness around a freshly built view tree: claim the
    /// [`TestRoot`] (views created by `build` attach to it), run the
    /// factory, wrap the tree in the headless harness at `size`, and
    /// take the first introspection pass. The factory runs **after**
    /// the root is claimed — floem views attach to the current view.
    pub fn boot(
        size: (f64, f64),
        build: impl FnOnce() -> AnyView,
        readers: Vec<SemReader>,
    ) -> Result<Self, AdapterError> {
        let root = TestRoot::new();
        let view = build();
        let mut reader_map: ReaderMap = HashMap::new();
        for reader in readers {
            let entry = reader_map.entry(reader.test_id.clone()).or_default();
            match reader.kind {
                SemKind::Text => entry.0 = Some(reader.read),
                SemKind::Value => entry.1 = Some(reader.read),
            }
        }
        let adapter = Self {
            meta: AdapterMeta {
                name: "floem",
                framework: "floem",
                framework_version: "0.2.0 (lapce/floem@1351ffb)".to_string(),
                // §6.2 floem column with the tiny-skia PaintState
                // contribution active — the contract is the data shape
                // the glue cannot silently drift from.
                caps: guiwright_adapter_floem::FloemAdapterContract::default()
                    .caps_with_paint_state_patch(),
            },
            harness: HeadlessHarness::new_with_size(root, view, size.0, size.1),
            engine: FloemLogicalTree::new(),
            test_ids: TestIdRegistry::new(),
            paint: PaintState::new(size.0 as u32, size.1 as u32)
                .map_err(|err| AdapterError::Internal(format!("floem paint state: {err}")))?,
            size,
            facts: HashMap::new(),
            readers: reader_map,
            overlay_plan: None,
            overlay_host: OverlayHost::new(),
            frame_cache: None,
            pointer: (size.0 / 2.0, size.1 / 2.0),
            pass_seq: 0,
            virtual_ms: 0,
            paused: false,
        };
        Ok(adapter)
    }

    /// The view type name the role registry keys on: floem's
    /// `debug_name` returns `"<decorator names> - <TypeName>"` where the
    /// type slot is either a full module path
    /// (`"floem::views::button::Button"`) or a content-carrying payload
    /// (`"Label: \"Count: 0\""`, `"TextInput: \"Ada\""`). So: truncate a
    /// `": "` payload first, then take the last `::` path segment, then
    /// `snake_case` (`"Button"` → `"button"`).
    fn kind_key(debug: &str) -> String {
        let last = debug.rsplit(" - ").next().unwrap_or(debug).trim();
        let stripped = match last.split_once(": ") {
            Some((type_name, _)) => type_name,
            None => last,
        };
        let type_name = stripped.rsplit("::").next().unwrap_or(stripped);
        camel_to_snake(type_name.trim())
    }

    /// The §8.1 test id: the first `debug_name` decorator segment
    /// (floem chains them most-recent-first before the type name).
    fn test_id_of(debug: &str) -> Option<String> {
        let first = debug.split(" - ").next()?;
        let type_name = debug.rsplit(" - ").next()?;
        (first != type_name && !first.is_empty()).then(|| first.to_string())
    }

    /// One introspection pass: the real [`ViewId`] walk folded into the
    /// engine. Also refreshes the per-handle facts the actions dispatch
    /// at and the test-id registry the O(1) hash reads. Public so the
    /// m80 bench can measure exactly one fold.
    pub fn introspect_once(&mut self) -> u64 {
        let report = self.walk_view();
        let digest = twox_digest(&report);
        let _diff = self.engine.rebuild(&report);
        digest
    }

    /// Walk the real view tree into a [`ViewReport`], refreshing the
    /// per-handle facts alongside (one pass serves both the engine fold
    /// and the action-time geometry).
    fn walk_view(&mut self) -> ViewReport {
        let root = self.harness.root_id();
        let mut facts = HashMap::new();
        let report = self.walk_inner(root, &mut facts);
        self.facts = facts;
        report
    }

    fn walk_inner(&mut self, id: ViewId, facts: &mut HashMap<u64, ItemFacts>) -> ViewReport {
        let handle = slotmap_key(id);
        let debug = id.debug_name();
        let kind = Self::kind_key(&debug);
        let mut report = ViewReport::new(&kind, handle);

        let test_id = Self::test_id_of(&debug);
        if let Some(test_id) = &test_id {
            report = report.with_test_id(test_id.clone());
            // The upstream `test_id(...)` decorator is a thin patch over
            // this registry (§4.7 contribution 1a): duplicate ids are an
            // app bug and fail loud here.
            if let Err(err) = self.test_ids.register(test_id.clone(), handle) {
                panic!("floem glue: {err}");
            }
        }

        let rect = id.get_visual_rect();
        report = report.with_rect((rect.x0, rect.y0), (rect.width(), rect.height()));

        let center = rect_center(rect.x0, rect.y0, rect.x1, rect.y1);
        let right_edge = rect_right_edge(rect.x0, rect.y0, rect.x1, rect.y1);
        let mut value_len = 0;
        if let Some(test_id) = &test_id
            && let Some((text, value)) = self.readers.get(test_id)
        {
            if let Some(text) = text {
                report = report.with_text(text());
            }
            if let Some(value) = value {
                let value = value();
                value_len = value.chars().count();
                report = report.with_value(value);
            }
        }
        facts.insert(
            handle,
            ItemFacts {
                center,
                right_edge,
                value_len,
            },
        );

        if self.harness.is_focused(id) {
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

    fn dispatch_key(
        &mut self,
        key: &str,
        modifiers: &[String],
        state: KeyState,
    ) -> Result<(), AdapterError> {
        self.dispatch(Event::Key(Self::key_event(key, modifiers, state)?));
        Ok(())
    }

    fn pointer_moved(&mut self, x: f64, y: f64) {
        self.pointer = (x, y);
        self.harness.pointer_move(x, y);
    }

    /// Click at a point: move → press → release (the §9 gesture
    /// synthesis, executed through the harness's real pipeline).
    fn click_at(&mut self, x: f64, y: f64) {
        self.pointer_moved(x, y);
        self.harness.pointer_down(x, y);
        Self::mock_time(CLICK_GAP_MS);
        self.harness.pointer_up(x, y);
    }

    /// One harness tick: the update queue drains at the virtual
    /// deadline (§4.7 "pumps `UpdatePhase` ticks with virtual time" —
    /// the timers themselves are the upstream harness action-queue
    /// patch; the tick advances the glue's virtual clock either way).
    fn tick(&mut self, advance_ms: u64) {
        self.virtual_ms = self.virtual_ms.saturating_add(advance_ms);
        self.harness.rebuild();
    }

    fn mock_time(_ms: u64) {}

    /// Map a named key onto floem's `ui-events` keyboard vocabulary.
    fn named_key(name: &str) -> Option<(Key, Code)> {
        let lower = name.trim().to_ascii_lowercase();
        let key = match lower.as_str() {
            "enter" | "return" => (NamedKey::Enter, Code::Enter),
            "backspace" => (NamedKey::Backspace, Code::Backspace),
            "delete" => (NamedKey::Delete, Code::Delete),
            "tab" => (NamedKey::Tab, Code::Tab),
            "escape" => (NamedKey::Escape, Code::Escape),
            "home" => (NamedKey::Home, Code::Home),
            "end" => (NamedKey::End, Code::End),
            "left" | "arrowleft" => (NamedKey::ArrowLeft, Code::ArrowLeft),
            "right" | "arrowright" => (NamedKey::ArrowRight, Code::ArrowRight),
            "up" | "arrowup" => (NamedKey::ArrowUp, Code::ArrowUp),
            "down" | "arrowdown" => (NamedKey::ArrowDown, Code::ArrowDown),
            "shift" => (NamedKey::Shift, Code::ShiftLeft),
            "control" | "ctrl" => (NamedKey::Control, Code::ControlLeft),
            "alt" => (NamedKey::Alt, Code::AltLeft),
            "meta" | "cmd" | "super" => (NamedKey::Meta, Code::MetaLeft),
            _ => return None,
        };
        Some((Key::Named(key.0), key.1))
    }

    /// A single character: physical-code mapping for the letters and
    /// digits (position matters to nothing the harness routes, but a
    /// real `Code` keeps the event faithful).
    fn char_key(ch: char) -> (Key, Code) {
        let key = Key::Character(ch.to_string());
        let code = match ch.to_ascii_lowercase() {
            'a' => Code::KeyA,
            'b' => Code::KeyB,
            'c' => Code::KeyC,
            'd' => Code::KeyD,
            'e' => Code::KeyE,
            'f' => Code::KeyF,
            'g' => Code::KeyG,
            'h' => Code::KeyH,
            'i' => Code::KeyI,
            'j' => Code::KeyJ,
            'k' => Code::KeyK,
            'l' => Code::KeyL,
            'm' => Code::KeyM,
            'n' => Code::KeyN,
            'o' => Code::KeyO,
            'p' => Code::KeyP,
            'q' => Code::KeyQ,
            'r' => Code::KeyR,
            's' => Code::KeyS,
            't' => Code::KeyT,
            'u' => Code::KeyU,
            'v' => Code::KeyV,
            'w' => Code::KeyW,
            'x' => Code::KeyX,
            'y' => Code::KeyY,
            'z' => Code::KeyZ,
            '0' => Code::Digit0,
            '1' => Code::Digit1,
            '2' => Code::Digit2,
            '3' => Code::Digit3,
            '4' => Code::Digit4,
            '5' => Code::Digit5,
            '6' => Code::Digit6,
            '7' => Code::Digit7,
            '8' => Code::Digit8,
            '9' => Code::Digit9,
            ' ' => Code::Space,
            _ => Code::Backquote,
        };
        (key, code)
    }

    fn modifiers_of(names: &[String]) -> Modifiers {
        let mut modifiers = Modifiers::empty();
        for name in names {
            match name.trim().to_ascii_lowercase().as_str() {
                "shift" => modifiers |= Modifiers::SHIFT,
                "control" | "ctrl" => modifiers |= Modifiers::CONTROL,
                "alt" => modifiers |= Modifiers::ALT,
                "meta" | "cmd" | "super" => modifiers |= Modifiers::META,
                _ => {}
            }
        }
        modifiers
    }

    fn key_event(
        key: &str,
        modifiers: &[String],
        state: KeyState,
    ) -> Result<KeyboardEvent, AdapterError> {
        let modifiers = Self::modifiers_of(modifiers);
        let (key, code) = if key.chars().count() == 1 && !is_modifier_name(key) {
            Self::char_key(key.chars().next().expect("single char"))
        } else {
            Self::named_key(key).ok_or(AdapterError::Unsupported(
                "key not in the floem key surface (Enter/Backspace/Delete/Tab/Escape/Home/End/arrows/modifiers or a single character)",
            ))?
        };
        Ok(KeyboardEvent {
            key,
            code,
            state,
            modifiers,
            ..Default::default()
        })
    }

    /// Fill the field at `handle`'s facts: click the right edge (the
    /// editor places the text cursor at the click point), clear through
    /// the real editor pipeline, retype (the m76/m78 fill contract).
    fn fill_at(&mut self, facts: ItemFacts, text: &str) -> Result<(), AdapterError> {
        self.click_at(facts.right_edge.0, facts.right_edge.1);
        for _ in 0..=facts.value_len {
            self.dispatch_key("backspace", &[], KeyState::Down)?;
            self.dispatch_key("backspace", &[], KeyState::Up)?;
        }
        for ch in text.chars() {
            self.dispatch_key(&ch.to_string(), &[], KeyState::Down)?;
            self.dispatch_key(&ch.to_string(), &[], KeyState::Up)?;
        }
        Ok(())
    }

    /// Geometry of the resolved node, fresh: one introspection pass
    /// refreshes the facts, then the minted id maps back to the view
    /// handle (stale nodes fail loud, §6.1 normalization).
    fn facts_of(&mut self, node: NodeId) -> Result<ItemFacts, AdapterError> {
        if self.engine.handle_of(node).is_none() {
            self.introspect_once();
        }
        let handle = self
            .engine
            .handle_of(node)
            .ok_or_else(|| AdapterError::Internal(format!("unknown node {node:?}")))?;
        self.facts.get(&handle).copied().ok_or_else(|| {
            AdapterError::Internal(format!("node {node:?} has no laid-out geometry"))
        })
    }

    /// The framework's own hit test, exposed to tests: the point inside
    /// a node's visual rect resolves (through real `element_id_at`
    /// hit-testing) to the view the walk found there — the §8.4
    /// action-time proof that resolved geometry and routed input agree.
    /// Hit-testing lands on the frontmost box-tree element, which may be
    /// a decorated view's child (the button's inner label), so the walk
    /// climbs to the nearest decorated ancestor before comparing.
    pub fn assert_hit(&self, x: f64, y: f64, test_id: &str) -> Result<(), AdapterError> {
        let id = self
            .harness
            .element_id_at(x, y)
            .ok_or_else(|| AdapterError::Internal("no element at the point".into()))?;
        let mut view = Some(id.owning_id());
        while let Some(current) = view {
            if Self::test_id_of(&current.debug_name()).as_deref() == Some(test_id) {
                return Ok(());
            }
            view = current.parent();
        }
        let hit = Self::kind_key(&id.owning_id().debug_name());
        Err(AdapterError::Internal(format!(
            "hit test resolved `{hit}` at ({x},{y}); no decorated ancestor carries `{test_id}`"
        )))
    }
}

/// `ViewId`'s stable opaque handle (the engine's `u64` identity, §8.2
/// ABA guards key on it).
fn slotmap_key(id: ViewId) -> u64 {
    use slotmap::Key;
    id.data().as_ffi()
}

fn is_modifier_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "shift" | "control" | "ctrl" | "alt" | "meta" | "cmd" | "super"
    )
}

/// `"TextInput"` → `"text_input"` (the role-registry vocabulary).
fn camel_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn rect_center(x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64) {
    (f64::midpoint(x0, x1), f64::midpoint(y0, y1))
}

fn rect_right_edge(x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64) {
    ((x1 - 2.0).max(x0), f64::midpoint(y0, y1))
}

/// Frame digest (§10): xxh3 over the introspection summary — the cheap
/// visual-change signal for the pump's settle loop.
fn twox_digest(report: &ViewReport) -> u64 {
    use std::hash::Hasher;
    use twox_hash::xxhash3_64::Hasher as Xxh3;
    fn walk(hasher: &mut Xxh3, report: &ViewReport) {
        hasher.write(report.kind.as_bytes());
        hasher.write_u64(report.handle);
        if let Some(text) = &report.text {
            hasher.write(text.as_bytes());
        }
        if let Some(value) = &report.value {
            hasher.write(value.as_bytes());
        }
        if let Some([x0, y0, x1, y1]) = report.bounds {
            for v in [x0, y0, x1, y1] {
                hasher.write_u64(v.to_bits());
            }
        }
        hasher.write_u8(u8::from(report.enabled));
        hasher.write_u32(report.children.len() as u32);
        for child in &report.children {
            walk(hasher, child);
        }
    }
    let mut hasher = Xxh3::default();
    walk(&mut hasher, report);
    hasher.finish()
}

/// The §4.7 contribution 2's driver-side half: translate the walked
/// view geometry into [`PaintOp`]s the engine's tiny-skia surface
/// rasterizes (the upstream patch routing floem's display list is what
/// would make this bit-identical to the GPU scene; the geometry, order
/// and determinism are already real).
fn paint_ops(report: &ViewReport) -> Vec<PaintOp> {
    fn walk(report: &ViewReport, ops: &mut Vec<PaintOp>) {
        if let Some([x0, y0, x1, y1]) = report.bounds {
            let (x, y) = (x0 as f32, y0 as f32);
            let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
            let color = match report.kind.as_str() {
                "button" => Color::rgba(37, 99, 235, 255),
                "text_input" => Color::rgba(255, 255, 255, 255),
                "label" | "text" | "rich_text" => Color::rgba(15, 23, 42, 255),
                _ => Color::rgba(203, 213, 225, 255),
            };
            ops.push(PaintOp::RoundedRect {
                x,
                y,
                w,
                h,
                radius: 4.0,
                color,
            });
        }
        for child in &report.children {
            walk(child, ops);
        }
    }
    let mut ops = vec![PaintOp::SolidRect {
        x: 0.0,
        y: 0.0,
        w: f32::MAX / 4.0,
        h: f32::MAX / 4.0,
        color: Color::rgba(248, 250, 252, 255),
    }];
    walk(report, &mut ops);
    ops
}

impl FrameworkAdapter for FloemAdapter {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    /// Level-2 adapter: no AccessKit tree of its own (§6.2 row).
    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome {
        let mut outcome = PumpOutcome::default();
        // The rebuild drains reactive effects, style and layout passes
        // (the harness's `process_update` cycle, §4.7); the fold
        // captures the resulting tree.
        self.harness.rebuild();
        let mut digest = self.introspect_once();
        self.pass_seq += 1;
        outcome.frames.push(FrameStamp {
            seq: self.pass_seq,
            hash: digest,
        });
        // Settle: one extra pass; a stable digest means the UI stopped
        // producing changes (the Zen shape, driven by the harness's own
        // update cycle).
        while !deadline.expired() {
            self.harness.rebuild();
            let next = self.introspect_once();
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
        // Prime the §10 readback cache (m87): the presented frame's
        // raster rides the pump's final pass, so the video recorder and
        // the screenshot reads serve pixels without a re-render
        // (§15.4 "never disturb the app under test").
        let (width, height) = (self.size.0 as u32, self.size.1 as u32);
        if let Ok(frame) = self.raster(width, height) {
            self.frame_cache = Some(FrameCache {
                seq: self.pass_seq,
                frame,
            });
        }
        outcome
    }

    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        let report = self.walk_view();
        Some(self.engine.rebuild(&report))
    }

    fn act(
        &mut self,
        node: accesskit::NodeId,
        action: SemAction,
    ) -> Result<ActionOutcome, AdapterError> {
        let facts = self.facts_of(node)?;
        let detail = match action {
            SemAction::Click | SemAction::Check | SemAction::Uncheck => {
                self.click_at(facts.center.0, facts.center.1);
                "clicked".to_string()
            }
            SemAction::DoubleClick => {
                self.click_at(facts.center.0, facts.center.1);
                Self::mock_time(CLICK_GAP_MS);
                // floem tracks click counts from the event payload; the
                // harness's double-click helper carries count 2.
                self.harness.double_click(facts.center.0, facts.center.1);
                "double clicked".to_string()
            }
            SemAction::SetValue { text } => {
                // The documented upstream blocker (m80): floem's
                // text_input requests IME the moment it gains focus
                // (`src/views/text_input.rs:1403`,
                // `set_ime_allowed(true)` hard-coded), and the headless
                // `MockWindow` rejects every IME request
                // (`src/window/mock.rs:262`,
                // `Err(ImeRequestError::NotSupported)`) which
                // `src/window/handle.rs:1270` unwraps into a panic —
                // floem's own harness suite (40 tests) never focuses a
                // text_input. The gate fires BEFORE the focus click so
                // the app never panics; the fill implementation below
                // stays real (real `KeyboardEvent`s through the real
                // pipeline) for the day an upstream patch routes IME
                // off the headless window.
                if self.engine.role_of(node) == Some(accesskit::Role::TextInput) {
                    return Err(AdapterError::Unsupported(
                        "fill (floem text_input focus panics headless: \
                         MockWindow::request_ime_update returns \
                         Err(NotSupported) — window/mock.rs:262 — \
                         unwrapped at window/handle.rs:1270)",
                    ));
                }
                self.fill_at(facts, &text)?;
                format!("filled {text:?}")
            }
            SemAction::Focus => {
                // floem focuses the hit element on pointer-down (§4.7
                // "Down: moves keyboard focus to the hit element").
                self.click_at(facts.center.0, facts.center.1);
                "focused via click".to_string()
            }
            SemAction::Blur => {
                return Err(AdapterError::Unsupported(
                    "blur (floem has no unfocus dispatch; focus moves by clicking elsewhere)",
                ));
            }
            SemAction::Hover => {
                self.pointer_moved(facts.center.0, facts.center.1);
                "hovered".to_string()
            }
            SemAction::Press { key, modifiers } => {
                self.dispatch_key(&key, &modifiers, KeyState::Down)?;
                self.dispatch_key(&key, &modifiers, KeyState::Up)?;
                format!("pressed {key}")
            }
            SemAction::Scroll { dx, dy } => {
                self.pointer_moved(facts.center.0, facts.center.1);
                self.harness.scroll(facts.center.0, facts.center.1, dx, dy);
                format!("scrolled ({dx},{dy})")
            }
        };
        // The change landed through the reactive runtime; one fold
        // refreshes the tree the session reads.
        self.introspect_once();
        self.pass_seq += 1;
        Ok(ActionOutcome {
            applied_at_frame: self.pass_seq,
            detail,
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        for step in translate(ev) {
            match step {
                FloemEvent::PointerDown { x, y, .. } => {
                    self.pointer_moved(x, y);
                    self.harness.pointer_down(x, y);
                }
                FloemEvent::PointerUp { x, y, .. } => {
                    self.pointer_moved(x, y);
                    self.harness.pointer_up(x, y);
                }
                FloemEvent::PointerMove { x, y } => self.pointer_moved(x, y),
                FloemEvent::PointerLeave => {
                    self.harness.pointer_leave();
                }
                FloemEvent::Wheel { x, y, dx, dy } => {
                    self.harness.scroll(x, y, dx, dy);
                }
                FloemEvent::KeyDown { key, modifiers } => {
                    self.dispatch_key(&key, &modifiers, KeyState::Down)?;
                }
                FloemEvent::KeyUp { key, modifiers } => {
                    self.dispatch_key(&key, &modifiers, KeyState::Up)?;
                }
                FloemEvent::ImeCommit { text } => {
                    // floem's `Event` has no IME-commit variant at the
                    // pinned revision; text flows through real
                    // `KeyboardEvent`s (the documented adaptation — the
                    // characters still enter the real editor buffer).
                    for ch in text.chars() {
                        self.dispatch_key(&ch.to_string(), &[], KeyState::Down)?;
                        self.dispatch_key(&ch.to_string(), &[], KeyState::Up)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        for step in clock_plan(cmd) {
            match step {
                FloemClockStep::Tick { advance_ms } => self.tick(advance_ms),
                // The harness clock is virtual from the first tick;
                // install is a marker, pause/resume gate run_for.
                FloemClockStep::MarkInstalled => {}
                FloemClockStep::MarkPaused { paused } => self.paused = paused,
            }
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_millis(self.virtual_ms)),
            paused: self.paused,
        })
    }

    /// One tiny-skia raster of the walked view geometry (§4.7
    /// contribution 2's driver-side half): fresh fold, one scene (app +
    /// overlay), one surface paint. Shared by `render_frame`,
    /// `last_frame` and the pump's cache priming.
    fn raster(&mut self, width: u32, height: u32) -> Result<RasterFrame, AdapterError> {
        if (self.paint.width(), self.paint.height()) != (width, height) {
            self.paint = PaintState::new(width, height)
                .map_err(|err| AdapterError::Internal(format!("floem paint state: {err}")))?;
        }
        // One fresh fold: the raster reflects the tree the session
        // reads (geometry, order and content all real).
        let report = self.walk_view();
        self.engine.rebuild(&report);
        // One scene, app + overlay (§13 present-time composition, m86):
        // the overlay host's ops paint after the app's own, so the
        // outline + badges land on top of the presented pixels.
        let mut ops = paint_ops(&report);
        if let Some(plan) = self.overlay_plan.as_ref() {
            ops.extend(self.overlay_host.present(plan));
        }
        self.paint.paint(&ops);
        let pixels: std::sync::Arc<[u8]> = std::sync::Arc::from(self.paint.rgba());
        let hash = self.paint.strided_hash();
        Ok(RasterFrame {
            pixels,
            size: (width, height),
            hash,
        })
    }

    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        let (width, height) = size.unwrap_or((self.size.0 as u32, self.size.1 as u32));
        // The per-pass cache serves an unsized re-read at the current
        // pass (§10 "prefer last_frame — no re-render"); anything else
        // (a size override, a newer pass) re-rasters (the shared helper).
        if let Some(cache) = self.frame_cache.as_ref()
            && cache.seq == self.pass_seq
            && cache.frame.size == (width, height)
        {
            return Ok(cache.frame_ref());
        }
        let frame = self.raster(width, height)?;
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

    /// The §10 `last_frame` readback (m87): the most recent presented
    /// frame's pixels, without disturbing app state — the pixel row the
    /// video recorder and the screenshot baselines ride (§6.2 floem
    /// column, m87).
    fn last_frame(&mut self) -> Option<FrameRef> {
        if let Some(cache) = self.frame_cache.as_ref()
            && cache.seq == self.pass_seq
        {
            return Some(cache.frame_ref());
        }
        let (width, height) = (self.size.0 as u32, self.size.1 as u32);
        let frame = self.raster(width, height).ok()?;
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

    /// Inspector overlay (§13, m86): store the plan — every
    /// `render_frame` composites it on the present path (the tiny-skia
    /// surface paints the host's ops into the same frame); `None`
    /// clears the scene (the `DisableOverlay` reset).
    fn show_overlay(
        &mut self,
        plan: Option<&OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.overlay_plan = plan.cloned();
        if self.overlay_plan.is_none() {
            self.overlay_host.clear();
        }
        Ok(())
    }
}

impl std::fmt::Debug for FloemAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloemAdapter")
            .field("meta", &self.meta)
            .field("size", &self.size)
            .field("pass_seq", &self.pass_seq)
            .field("virtual_ms", &self.virtual_ms)
            .field("paused", &self.paused)
            .field("tracked_views", &self.facts.len())
            .finish_non_exhaustive()
    }
}
