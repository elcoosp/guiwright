//! The slint glue (architecture §4.4 door A, §17 row 5, m78):
//! [`FrameworkAdapter`] over the **real** `i-slint-backend-testing`
//! `TestingBackend`.
//!
//! The engine (`guiwright-adapter-slint`, main workspace) stays
//! framework-free; this crate is the framework half, scoped to its own
//! workspace by the skia-links record (crate docs of
//! `guiwright-adapter-slint`). Everything here rides the verified public
//! surface of slint 1.19.0:
//!
//! * **Boot** — [`slint::platform::set_platform`] installs a
//!   `TestingBackend` with mock time and the software rasterizer (the
//!   platform must be installed before the first component instantiates;
//!   the testing backend is per-thread, so one adapter per test thread).
//! * **Input** — `Window::dispatch_event(WindowEvent::…)` — "the same
//!   code path real winit events take" (§4.4): pointer moved/pressed/
//!   released/scrolled, keys pressed/released with explicit modifier
//!   presses and releases (the §4.3 gaps' slint counterpart).
//! * **Tree** — the `ElementHandle` introspection (debug-info builds:
//!   element names become the §8.1 test ids) walks into
//!   [`guiwright_adapter_slint::IntrospectionItem`] and folds through the
//!   engine's [`SlintTreeBuilder`]. The public API exposes the walk as a
//!   flat DFS, so nesting is reconstructed by bounds containment — the
//!   same technique the iced read-back uses for closing siblings (m76).
//! * **Pixels** — `Window::take_snapshot()` (§4.4 "the canonical headless
//!   raster path") through the backend's software rasterizer.
//! * **Clock** — `mock_elapsed_time` (§14: "first-class"); the clock plan
//!   ([`guiwright_adapter_slint::clock::plan`]) executes step by step.
//!
//! Known quirks encoded here (§9): the testing backend exposes no public
//! focus query, so the synthesized nodes carry no focus fact; `Blur` has
//! no target (focus moves by clicking elsewhere); equal-rect siblings
//! nest by containment order; `DoubleClick` keeps 50 ms mock gaps inside
//! the platform's 500 ms click interval.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use accesskit::NodeId;
use guiwright_adapter_slint::clock::{SlintClockStep, plan as clock_plan};
use guiwright_adapter_slint::{IntrospectionItem, SlintId, SlintTreeBuilder};
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PointerButton, PumpOutcome,
    SemAction,
};
use guiwright_inspect::overlay::OverlayPlan;
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, SharedString};

use crate::overlay::OverlayHost;

/// The mock gap between press and release (§4.4 upstream's
/// `mock_single_click` timing — inside the 500 ms default click
/// interval, so double clicks stay recognizable).
const CLICK_GAP_MS: u64 = 50;

/// Install the testing backend as the thread's platform: mock time, no
/// event-loop queue, software rasterizer (headless `take_snapshot`).
/// Must run before the first component instantiates; panics when a
/// platform is already installed (one harness per thread — §18's
/// process-per-test discipline maps onto threads here).
pub fn install_testing_platform() {
    slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
        i_slint_backend_testing::TestingBackendOptions {
            mock_time: true,
            threading: false,
            renderer_name: Some(SharedString::from("software")),
        },
    )))
    .expect("slint testing platform already installed (one harness per thread)");
}

/// Facts the latest introspection pass recorded per walk handle — the
/// action-time geometry (`act` resolves a minted `NodeId` back to the
/// element's laid-out rect) and the editor length a fill clears.
#[derive(Clone, Copy, Debug)]
struct ItemFacts {
    center: (f64, f64),
    right_edge: (f64, f64),
    value_len: usize,
}

type Rect = [f64; 4];

/// The slint adapter (§6.1): Level-2 — the engine folds real
/// `ElementHandle` introspection into synthesized AccessKit diffs; input
/// rides the real `Window::dispatch_event` pipeline; pixels come from the
/// software rasterizer; time is the testing backend's mock clock.
pub struct SlintAdapter<C: ComponentHandle> {
    meta: AdapterMeta,
    engine: SlintTreeBuilder,
    app: C,
    size: (f64, f64),
    facts: HashMap<u64, ItemFacts>,
    pointer: (f64, f64),
    pass_seq: u64,
    paused: bool,
    /// The §13 inspector overlay (the §4.4 top-level overlay window,
    /// m84): shown while the `inspect` domain drives it, composited onto
    /// the presented frame at capture time.
    overlay: OverlayHost,
    /// The §10 frame cache (m85): the pixels `last_frame` serves, keyed
    /// on the pass sequence so one snapshot per pump serves every
    /// presented stamp of that pump and any `act`/`render_frame` fold
    /// invalidates it.
    frame_cache: Option<(u64, Arc<[u8]>, (u32, u32))>,
}

impl<C: ComponentHandle + 'static> SlintAdapter<C> {
    /// Boot a harness around a freshly created component: install the
    /// testing platform, run `factory`, size the headless window, and
    /// take the first introspection pass. The factory runs **after** the
    /// platform is installed — component instantiation picks up the
    /// testing backend's window adapter.
    pub fn boot(
        factory: impl FnOnce() -> Result<C, slint::PlatformError>,
        size: (f64, f64),
    ) -> Result<Self, AdapterError> {
        install_testing_platform();
        let app =
            factory().map_err(|err| AdapterError::Internal(format!("slint app boot: {err}")))?;
        let adapter = Self {
            meta: AdapterMeta {
                name: "slint",
                framework: "slint",
                framework_version: "1.19.0".to_string(),
                caps: Caps {
                    tree_update: false,
                    logical_snapshot: true,
                    input: true,
                    last_frame: true,
                    render_frame: true,
                    clock: true,
                },
            },
            engine: SlintTreeBuilder::new("slint"),
            app,
            size,
            facts: HashMap::new(),
            pointer: (size.0 / 2.0, size.1 / 2.0),
            pass_seq: 0,
            paused: false,
            overlay: OverlayHost::default(),
            frame_cache: None,
        };
        adapter
            .app
            .window()
            .set_size(LogicalSize::new(size.0 as f32, size.1 as f32));
        Ok(adapter)
    }

    /// One introspection pass: the `ElementHandle` walk (debug-info
    /// builds carry element type names + names) folded into the engine.
    /// Also refreshes the per-handle facts the actions dispatch at.
    fn introspect(&mut self) -> IntrospectionItem {
        let root = self.app.root_element();
        let mut elements = Vec::new();
        root.visit_descendants(|handle| {
            elements.push(handle);
            std::ops::ControlFlow::<()>::Continue(())
        });

        // The walk is a flat DFS; rebuild the nesting by bounds
        // containment (m76's sibling-closing technique): a new element
        // attaches to the innermost open rect that contains it. Widgets
        // nest their internal editors under layout boxes (the
        // `LineEdit`'s `TextInput` editor sits below generic containers
        // and the placeholder text), so a non-generic element whose role
        // matches an open ancestor is marked collapsed: at unwind it
        // folds its facts into that ancestor instead of attaching — the
        // strict-mode resolver then sees one semantic node per widget.
        let window_rect: Rect = [0.0, 0.0, self.size.0, self.size.1];
        let mut stack: Vec<OpenItem> = vec![OpenItem {
            rect: window_rect,
            index: 0,
            role: accesskit::Role::Window,
            item: self.window_item(),
            collapsed_into: None,
        }];
        let mut facts = HashMap::new();
        let mut walk_index: u64 = 1;
        for handle in &elements {
            let (item, rect) = Self::element_item(handle, walk_index);
            let role = guiwright_adapter_slint::role_of_kind(&item.kind);
            facts.insert(
                walk_index,
                ItemFacts {
                    center: rect_center(rect),
                    right_edge: rect_right_edge(rect),
                    value_len: handle.accessible_value().map_or(0, |v| v.chars().count()),
                },
            );
            walk_index += 1;
            while stack.len() > 1 && !rect_contains(stack.last().expect("open rect").rect, rect) {
                unwind(&mut stack, &mut facts);
            }
            // The window root stays open: collapse only below it.
            let collapsed_into = (role != accesskit::Role::GenericContainer)
                .then(|| {
                    stack
                        .iter()
                        .rposition(|open| open.role == role)
                        .filter(|pos| *pos > 0)
                })
                .flatten();
            stack.push(OpenItem {
                rect,
                index: walk_index - 1,
                role,
                item,
                collapsed_into,
            });
        }
        while stack.len() > 1 {
            unwind(&mut stack, &mut facts);
        }
        let root_item = stack.pop().expect("root item").item;
        self.facts = facts;
        root_item
    }

    fn window_item(&self) -> IntrospectionItem {
        IntrospectionItem::new("Window", SlintId::new(0, 0)).with_bounds(
            0.0,
            0.0,
            self.size.0,
            self.size.1,
        )
    }

    fn element_item(handle: &ElementHandle, index: u64) -> (IntrospectionItem, Rect) {
        // Debug-info builds carry type names and element names; without
        // them the walk degrades to unlabeled containers (the §4.4
        // missing-debug-info warning rides the backend's debug log).
        let kind = handle
            .type_name()
            .map_or_else(|| "Unknown".to_string(), |name| name.to_string());
        let mut item = IntrospectionItem::new(&kind, SlintId::new(0, index as u32));
        if let Some(label) = handle.accessible_label() {
            item = item.with_text(label);
        }
        if let Some(value) = handle.accessible_value() {
            item = item.with_value(value);
        }
        if let Some(name) = handle.id() {
            item = item.with_test_id(name);
        }
        let position = handle.absolute_position();
        let size = handle.size();
        let rect: Rect = [
            f64::from(position.x),
            f64::from(position.y),
            f64::from(position.x) + f64::from(size.width),
            f64::from(position.y) + f64::from(size.height),
        ];
        item = item.with_bounds(rect[0], rect[1], rect[2], rect[3]);
        if handle.accessible_enabled() == Some(false) {
            item = item.disabled();
        }
        // Focus: the testing backend has no public focus query — the
        // fact stays unset (documented quirk, §9).
        (item, rect)
    }

    /// One fold: introspect + rebuild the engine diff (the frame digest
    /// source and the resolve surface).
    fn fold_pass(&mut self) -> u64 {
        let item = self.introspect();
        let diff = self.engine.rebuild(&item);
        let mut digest = twox_digest(&item);
        digest ^= diff.removed.len() as u64;
        digest
    }

    fn dispatch(&mut self, event: WindowEvent) -> Result<(), AdapterError> {
        // Both dispatch verdicts (accepted/rejected) are delivery
        // successes — `Rejected` means no element consumed the event; a
        // platform error (renderer resize failures) is the real failure.
        self.app
            .window()
            .dispatch_event_with_result(event)
            .map(|_| ())
            .map_err(|err| AdapterError::Internal(format!("slint dispatch_event: {err}")))
    }

    fn pointer_moved(&mut self, x: f64, y: f64) -> Result<(), AdapterError> {
        self.pointer = (x, y);
        self.dispatch(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x as f32, y as f32),
        })
    }

    fn press(&mut self, button: PointerEventButton) -> Result<(), AdapterError> {
        self.dispatch(WindowEvent::PointerPressed {
            position: LogicalPosition::new(self.pointer.0 as f32, self.pointer.1 as f32),
            button,
        })
    }

    fn release(&mut self, button: PointerEventButton) -> Result<(), AdapterError> {
        self.dispatch(WindowEvent::PointerReleased {
            position: LogicalPosition::new(self.pointer.0 as f32, self.pointer.1 as f32),
            button,
        })
    }

    /// Click at a point: move → press → mock gap → release (the §9
    /// gesture synthesis, executed through `Window::dispatch_event`).
    fn click_at(&mut self, x: f64, y: f64) -> Result<(), AdapterError> {
        self.pointer_moved(x, y)?;
        self.press(PointerEventButton::Left)?;
        Self::mock_time(CLICK_GAP_MS);
        self.release(PointerEventButton::Left)
    }

    fn mock_time(ms: u64) {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
    }

    /// Map a named key onto the §9 slint key vocabulary; plain single
    /// characters pass through as their text.
    fn key_text(name: &str) -> Result<SharedString, AdapterError> {
        let named = match name.trim().to_ascii_lowercase().as_str() {
            "enter" | "return" => Some(Key::Return),
            "escape" => Some(Key::Escape),
            "tab" => Some(Key::Tab),
            "backspace" => Some(Key::Backspace),
            "delete" => Some(Key::Delete),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            "pageup" => Some(Key::PageUp),
            "pagedown" => Some(Key::PageDown),
            "left" | "arrowleft" => Some(Key::LeftArrow),
            "right" | "arrowright" => Some(Key::RightArrow),
            "up" | "arrowup" => Some(Key::UpArrow),
            "down" | "arrowdown" => Some(Key::DownArrow),
            "shift" => Some(Key::Shift),
            "control" | "ctrl" => Some(Key::Control),
            "alt" => Some(Key::Alt),
            _ => None,
        };
        if let Some(key) = named {
            return Ok(key.into());
        }
        let mut chars = name.chars();
        let (first, rest) = (chars.next(), chars.count());
        if rest == 0 {
            return Ok(SharedString::from(
                first.expect("non-empty name").to_string(),
            ));
        }
        Err(AdapterError::Unsupported(
            "key not in the slint key surface (Enter/Escape/Tab/Backspace/Delete/Home/End/arrows/modifiers or a single character)",
        ))
    }

    fn modifier_key(name: &str) -> Result<Option<Key>, AdapterError> {
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "shift" => Some(Key::Shift),
            "control" | "ctrl" => Some(Key::Control),
            "alt" => Some(Key::Alt),
            "" => None,
            _ => {
                return Err(AdapterError::Unsupported(
                    "modifier not in the slint key surface (Shift/Control/Alt)",
                ));
            }
        })
    }

    /// A typed character: uppercase rides an explicit Shift press/release
    /// pair (mirrors the backend's own string-sequence helper).
    fn type_char(&mut self, ch: char) -> Result<(), AdapterError> {
        let upper = ch.is_uppercase();
        if upper {
            self.dispatch(WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            })?;
        }
        self.dispatch(WindowEvent::KeyPressed {
            text: SharedString::from(ch.to_string()),
        })?;
        self.dispatch(WindowEvent::KeyReleased {
            text: SharedString::from(ch.to_string()),
        })?;
        if upper {
            self.dispatch(WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            })?;
        }
        Ok(())
    }

    /// Fill the field at `handle`'s facts: click the right edge (the
    /// editor places the text cursor at the click point), clear through
    /// the real editor pipeline, retype (the m76 fill contract).
    fn fill_at(&mut self, facts: ItemFacts, text: &str) -> Result<(), AdapterError> {
        self.click_at(facts.right_edge.0, facts.right_edge.1)?;
        for _ in 0..=facts.value_len {
            self.dispatch(WindowEvent::KeyPressed {
                text: Key::Backspace.into(),
            })?;
            self.dispatch(WindowEvent::KeyReleased {
                text: Key::Backspace.into(),
            })?;
        }
        for ch in text.chars() {
            self.type_char(ch)?;
        }
        Ok(())
    }

    /// Keyboard press with modifiers: modifier presses precede the key,
    /// and releases follow it (the §9 synthesis; the §4.3 `KeyUp` gap's
    /// slint counterpart).
    fn press_key(&mut self, key: &str, modifiers: &[String]) -> Result<(), AdapterError> {
        let mut pressed_modifiers = Vec::new();
        for modifier in modifiers {
            if let Some(key) = Self::modifier_key(modifier)? {
                self.dispatch(WindowEvent::KeyPressed { text: key.into() })?;
                pressed_modifiers.push(key);
            }
        }
        let text = Self::key_text(key)?;
        self.dispatch(WindowEvent::KeyPressed { text: text.clone() })?;
        self.dispatch(WindowEvent::KeyReleased { text })?;
        for key in pressed_modifiers.into_iter().rev() {
            self.dispatch(WindowEvent::KeyReleased { text: key.into() })?;
        }
        Ok(())
    }

    fn key_up(&mut self, key: &str, modifiers: &[String]) -> Result<(), AdapterError> {
        let text = Self::key_text(key)?;
        self.dispatch(WindowEvent::KeyReleased { text })?;
        // KeyUp's modifiers release in reverse (the key_sequence shape).
        for modifier in modifiers.iter().rev() {
            if let Some(key) = Self::modifier_key(modifier)? {
                self.dispatch(WindowEvent::KeyReleased { text: key.into() })?;
            }
        }
        Ok(())
    }

    /// Geometry of the resolved node, fresh: one introspection pass
    /// refreshes the facts, then the minted id maps back to the walk
    /// handle (stale nodes fail loud, §6.1 normalization).
    fn facts_of(&mut self, node: NodeId) -> Result<ItemFacts, AdapterError> {
        if self.engine.item_of(node).is_none() {
            let item = self.introspect();
            self.engine.rebuild(&item);
        }
        let handle = self
            .engine
            .item_of(node)
            .ok_or_else(|| AdapterError::Internal(format!("unknown node {node:?}")))?;
        self.facts.get(&handle.to_handle()).copied().ok_or_else(|| {
            AdapterError::Internal(format!("node {node:?} has no laid-out geometry"))
        })
    }
}

/// One open element during the containment walk: its laid-out rect, walk
/// handle, synthesized role and (when the element duplicates an open
/// ancestor's role) the stack position to fold into at unwind.
struct OpenItem {
    rect: Rect,
    index: u64,
    role: accesskit::Role,
    item: IntrospectionItem,
    collapsed_into: Option<usize>,
}

/// Close one open element: collapsed elements fold their facts into the
/// role-carrying ancestor they duplicate (label, value, test id — and the
/// fill's editor length) and re-parent their children there; everything
/// else attaches to the innermost remaining open element.
fn unwind(stack: &mut Vec<OpenItem>, facts: &mut HashMap<u64, ItemFacts>) {
    let closed = stack.pop().expect("non-empty stack");
    if let Some(target_pos) = closed.collapsed_into {
        let target = stack
            .get_mut(target_pos)
            .expect("collapse target stays open below the closed element");
        let child = closed.item;
        if target.item.text.is_none() {
            target.item.text.clone_from(&child.text);
        }
        if target.item.value.is_none() {
            if let Some(value) = &child.value
                && let Some(target_facts) = facts.get_mut(&target.index)
            {
                target_facts.value_len = value.chars().count();
            }
            target.item.value.clone_from(&child.value);
        }
        if target.item.test_id.is_none() {
            target.item.test_id.clone_from(&child.test_id);
        }
        // The inner editor is the interactive region (the widget rect
        // includes padding the editor does not fill) — fills and focus
        // clicks aim at the editor's geometry.
        let child_facts = facts.get(&closed.index).copied();
        if let (Some(target_facts), Some(child_facts)) = (facts.get_mut(&target.index), child_facts)
        {
            target_facts.center = child_facts.center;
            target_facts.right_edge = child_facts.right_edge;
        }
        target.item.children.extend(child.children);
    } else {
        let parent = stack.last_mut().expect("non-empty stack");
        parent.item.children.push(closed.item);
    }
}

fn rect_center(rect: Rect) -> (f64, f64) {
    (
        f64::midpoint(rect[0], rect[2]),
        f64::midpoint(rect[1], rect[3]),
    )
}

fn rect_right_edge(rect: Rect) -> (f64, f64) {
    (
        (rect[2] - 2.0).max(rect[0]),
        f64::midpoint(rect[1], rect[3]),
    )
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    // Half-logical-px tolerance: layout coordinates arrive as floats and
    // touching edges must count as contained.
    const TOLERANCE: f64 = 0.5;
    inner[0] >= outer[0] - TOLERANCE
        && inner[1] >= outer[1] - TOLERANCE
        && inner[2] <= outer[2] + TOLERANCE
        && inner[3] <= outer[3] + TOLERANCE
}

/// Frame digest (§10): xxh3 over the introspection summary — the cheap
/// visual-change signal for the pump's settle loop.
fn twox_digest(item: &IntrospectionItem) -> u64 {
    use std::hash::Hasher;
    use twox_hash::xxhash3_64::Hasher as Xxh3;
    fn walk(hasher: &mut Xxh3, item: &IntrospectionItem) {
        hasher.write(item.kind.as_bytes());
        hasher.write_u64(item.id.to_handle());
        if let Some(text) = &item.text {
            hasher.write(text.as_bytes());
        }
        if let Some(value) = &item.value {
            hasher.write(value.as_bytes());
        }
        if let Some([x0, y0, x1, y1]) = item.bounds {
            for v in [x0, y0, x1, y1] {
                hasher.write_u64(v.to_bits());
            }
        }
        hasher.write_u8(u8::from(item.enabled));
        hasher.write_u32(item.children.len() as u32);
        for child in &item.children {
            walk(hasher, child);
        }
    }
    let mut hasher = Xxh3::default();
    walk(&mut hasher, item);
    hasher.finish()
}

impl<C: ComponentHandle + 'static> FrameworkAdapter for SlintAdapter<C> {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    /// Level-2 adapter: no AccessKit tree of its own (§6.2 row).
    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome {
        let mut outcome = PumpOutcome::default();
        // Mock time flushes timers, animations, change handlers and
        // repeater instantiation on every live testing window (§4.4
        // clock row); the fold captures the resulting tree.
        Self::mock_time(0);
        let mut digest = self.fold_pass();
        self.pass_seq += 1;
        outcome.frames.push(FrameStamp {
            seq: self.pass_seq,
            hash: digest,
        });
        // Settle: one extra pass; a stable digest means the UI stopped
        // producing changes (the Zen shape, driven by the framework's
        // own clock instead of an Emulator mode).
        while !deadline.expired() {
            Self::mock_time(0);
            let next = self.fold_pass();
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

    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        let item = self.introspect();
        Some(self.engine.rebuild(&item))
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        let facts = self.facts_of(node)?;
        let detail = match action {
            SemAction::Click | SemAction::Check | SemAction::Uncheck => {
                self.click_at(facts.center.0, facts.center.1)?;
                "clicked".to_string()
            }
            SemAction::DoubleClick => {
                self.click_at(facts.center.0, facts.center.1)?;
                Self::mock_time(CLICK_GAP_MS);
                self.click_at(facts.center.0, facts.center.1)?;
                "double clicked".to_string()
            }
            SemAction::SetValue { text } => {
                self.fill_at(facts, &text)?;
                format!("filled {text:?}")
            }
            // The m89 select vocabulary rides the same fill path — the
            // TestingBackend's `Window::dispatch_event` applies the
            // value the same way it applies text (§4.4).
            SemAction::SelectOption { option } => {
                self.fill_at(facts, &option)?;
                format!("selected {option:?}")
            }
            // The m90 scroll-into-view vocabulary: the TestingBackend's
            // dispatch surface carries no scroll-into-view request —
            // loud, like the other framework-native vocabulary here.
            SemAction::ScrollIntoView => {
                return Err(AdapterError::Unsupported(
                    "scroll_into_view_if_needed (no scroll-into-view dispatch \
                     on the TestingBackend event surface)",
                ));
            }
            SemAction::Focus => {
                // Slint focuses focusable elements on click; there is no
                // focus-without-activate path on the dispatch surface.
                self.click_at(facts.center.0, facts.center.1)?;
                "focused via click".to_string()
            }
            SemAction::Blur => {
                return Err(AdapterError::Unsupported(
                    "blur (slint has no unfocus dispatch; focus moves by clicking elsewhere)",
                ));
            }
            SemAction::Hover => {
                self.pointer_moved(facts.center.0, facts.center.1)?;
                "hovered".to_string()
            }
            SemAction::Press { key, modifiers } => {
                self.press_key(&key, &modifiers)?;
                format!("pressed {key}")
            }
            SemAction::Scroll { dx, dy } => {
                self.pointer_moved(facts.center.0, facts.center.1)?;
                self.dispatch(WindowEvent::PointerScrolled {
                    position: LogicalPosition::new(facts.center.0 as f32, facts.center.1 as f32),
                    delta_x: dx as f32,
                    delta_y: dy as f32,
                })?;
                format!("scrolled ({dx},{dy})")
            }
        };
        // The change landed synchronously through the property graph;
        // one fold refreshes the tree the session reads.
        self.fold_pass();
        self.pass_seq += 1;
        Ok(ActionOutcome {
            applied_at_frame: self.pass_seq,
            detail,
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        match ev {
            InputEvent::PointerMove { x, y } => self.pointer_moved(x, y),
            InputEvent::PointerDown { x, y, button } => {
                self.pointer_moved(x, y)?;
                self.press(match button {
                    PointerButton::Primary => PointerEventButton::Left,
                    PointerButton::Secondary => PointerEventButton::Right,
                    PointerButton::Middle => PointerEventButton::Middle,
                })
            }
            InputEvent::PointerUp { x, y, button } => {
                self.pointer_moved(x, y)?;
                self.release(match button {
                    PointerButton::Primary => PointerEventButton::Left,
                    PointerButton::Secondary => PointerEventButton::Right,
                    PointerButton::Middle => PointerEventButton::Middle,
                })
            }
            InputEvent::Wheel { dx, dy } => self.dispatch(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(self.pointer.0 as f32, self.pointer.1 as f32),
                delta_x: dx as f32,
                delta_y: dy as f32,
            }),
            InputEvent::KeyDown { key, modifiers } => {
                for modifier in &modifiers {
                    if let Some(k) = Self::modifier_key(modifier)? {
                        self.dispatch(WindowEvent::KeyPressed { text: k.into() })?;
                    }
                }
                let text = Self::key_text(&key)?;
                self.dispatch(WindowEvent::KeyPressed { text })
            }
            InputEvent::KeyUp { key, modifiers } => self.key_up(&key, &modifiers),
            InputEvent::TextInput { text } => {
                for ch in text.chars() {
                    self.type_char(ch)?;
                }
                Ok(())
            }
        }
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        for step in clock_plan(cmd) {
            match step {
                SlintClockStep::MockElapsedTime { ms } => Self::mock_time(ms),
                // The testing backend is always virtual: install is a
                // marker, pause/resume gate the driver's run_for.
                SlintClockStep::MarkInstalled => {}
                SlintClockStep::MarkPaused { paused } => self.paused = paused,
            }
        }
        Ok(ClockState {
            virtual_time: Some(Duration::from_millis(
                i_slint_backend_testing::get_mocked_time(),
            )),
            paused: self.paused,
        })
    }

    /// Last presented frame (§6.1, m85): the testing backend has no
    /// presentation loop — pixels materialize on demand through the same
    /// `take_snapshot` raster path `render_frame` rides — so the cache
    /// keys on the pass sequence: one snapshot per pump serves every
    /// presented stamp of that pump, and the next `act`/`render_frame`
    /// fold re-snapshots. The §13 overlay composites here too, so a
    /// recording while the `inspect` domain is on shows the overlay.
    /// `None` when the rasterizer fails (the recorder sees no frame).
    fn last_frame(&mut self) -> Option<FrameRef> {
        if let Some((seq, pixels, size)) = &self.frame_cache
            && *seq == self.pass_seq
        {
            return Some(FrameRef {
                stamp: FrameStamp { seq: *seq, hash: 0 },
                size: *size,
                pixels: Some(Arc::clone(pixels)),
            });
        }
        let buffer = self.app.window().take_snapshot().ok()?;
        let physical = buffer.size();
        let mut pixels: Vec<u8> = buffer.as_bytes().to_vec();
        self.overlay.composite(&mut pixels);
        let pixels: Arc<[u8]> = Arc::from(pixels);
        let size = (physical.width, physical.height);
        self.frame_cache = Some((self.pass_seq, Arc::clone(&pixels), size));
        Some(FrameRef {
            stamp: FrameStamp {
                seq: self.pass_seq,
                hash: 0,
            },
            size,
            pixels: Some(pixels),
        })
    }

    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        if let Some((w, h)) = size {
            self.size = (f64::from(w), f64::from(h));
            self.app
                .window()
                .set_size(LogicalSize::new(w as f32, h as f32));
            // The overlay mirrors the app window's geometry (§4.4): a
            // resize while the session is on re-mirrors instead of
            // letting the mirrored rects drift off the frame.
            self.overlay.mirror(self.size)?;
        }
        let buffer = self
            .app
            .window()
            .take_snapshot()
            .map_err(|err| AdapterError::Internal(format!("slint take_snapshot: {err}")))?;
        let physical = buffer.size();
        // The §13 present-time composite: the overlay window's snapshot
        // blends onto the app frame (straight-alpha source-over — the
        // overlay paints nothing where its plan is empty, §15.4). The
        // blend works on the byte view; the frame's own copy feeds the
        // digest afterward.
        let mut pixels: Vec<u8> = buffer.as_bytes().to_vec();
        self.overlay.composite(&mut pixels);
        let pixels: Arc<[u8]> = Arc::from(pixels);
        let hash = {
            use std::hash::Hasher;
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
            size: (physical.width, physical.height),
            pixels: Some(pixels),
        })
    }

    /// Inspector overlay (§13, m84): the plan composites through the
    /// §4.4 top-level overlay window — a borderless second `.slint`
    /// window in the same `TestingBackend` registry, mirroring the app
    /// window's geometry. `None` clears the session (the window leaves
    /// the registry).
    fn show_overlay(&mut self, plan: Option<&OverlayPlan>) -> Result<(), AdapterError> {
        match plan {
            Some(plan) => self.overlay.show(plan, self.size),
            None => {
                self.overlay.clear();
                Ok(())
            }
        }
    }
}

impl<C: ComponentHandle + 'static> SlintAdapter<C> {
    /// Drain the overlay's paint log (the §13 draw-call evidence): one
    /// entry per outline/badge the overlay composited since the last
    /// take; `None` when nothing was drawn (no session, or it was
    /// cleared).
    pub fn take_overlay_paint_log(&mut self) -> Option<Vec<String>> {
        self.overlay.take_paint_log()
    }
}

impl<C: ComponentHandle> std::fmt::Debug for SlintAdapter<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlintAdapter")
            .field("meta", &self.meta)
            .field("size", &self.size)
            .field("pass_seq", &self.pass_seq)
            .field("paused", &self.paused)
            .field("tracked_items", &self.facts.len())
            .finish_non_exhaustive()
    }
}
