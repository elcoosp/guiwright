//! The `freya` feature: the glue binding the engine to the **real**
//! `freya_testing::TestingRunner` (architecture §4.3 "Guiwright wraps it
//! 1:1 for embedded mode", §17 row 3, m79).
//!
//! Build with `--features freya` (pulls freya-engine → software Skia);
//! the CI matrix job `adapter-freya (features: freya)` runtime-tests it
//! (m79 upgraded the job from `cargo check`). The default build carries
//! none of it (§5.3). No local clang means the Skia stack compiles only
//! on CI — the framework-free surface (everything outside this feature)
//! still compile-verifies locally.
//!
//! The binding executes through the verified public surface only:
//! `TestingRunner::{sync_and_update, send_event, send_accessibility_action,
//! poll_n, render, render_to_file}` —
//! * `sync_and_update` returns the committed AccessKit `TreeUpdate`
//!   (`process_events_and_layout` + `AccessibilityTree::commit_…`) — the
//!   Level-1 diff the adapter surfaces through `tree_update` (§4.3);
//! * input rides `send_event(PlatformEvent)` in the frame-quantized
//!   order (§4.3: "input is frame-quantized by ragnarok's `EventBuffer`
//!   — respect press→sync→move→release ordering"): press → sync →
//!   release, keys with modifiers and explicit `KeyUp` (§4.3 gap);
//! * semantic `Focus`/`Blur` ride the native
//!   `AccessibilityTree::handle_action` path
//!   (`send_accessibility_action`); coordinate actions ride the pointer
//!   pipeline (§4.3 "pixel clicks go through `send_event` + ragnarok
//!   hit-testing");
//! * freya 0.5 aliases `AccessibilityId = accesskit::NodeId`, so the
//!   app-assigned `.a11y_id(AccessibilityId(n))` numbers ARE the tree
//!   update's node ids — the [`AccessibilityMap`] bridge binds the ids
//!   the update carries and unbinds the ones that left.

use std::collections::HashMap;
use std::time::Duration;

use accesskit::{NodeId, TreeUpdate};
use freya_testing::TestingRunner;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline, FrameRef,
    FrameStamp, FrameworkAdapter, InputEvent, PumpOutcome, SemAction,
};

use crate::buffer::{PendingEvent, SyncEventBuffer};
use crate::ids::AccessibilityMap;
use crate::overlay;
use crate::platform::PlatformEvent;

use guiwright_inspect::overlay::OverlayPlan;

/// The runner-facing seam (§4.3): run one frame (`sync_and_update`),
/// deliver one translated platform event, commit the accessibility diff,
/// poll the ticker and rasterize a snapshot.
pub trait TestingSeam {
    /// One harness step (freya: `sync_and_update()`) — returns the
    /// committed AccessKit tree update.
    fn sync_and_update(&mut self) -> TreeUpdate;
    /// Deliver one translated platform event (`send_event`; no internal
    /// sync — the adapter owns the frame-quantized ordering).
    fn send_event(&mut self, event: PlatformEvent) -> Result<(), AdapterError>;
    /// The accessibility ids that left the tree since the last step.
    fn removed_ids(&mut self) -> Vec<u64>;
    /// Native semantic handling on the `AccessibilityTree` (§4.3:
    /// `Focus` / `Blur` handled natively, the rest routed as
    /// `EventName::AccessibilityAction`).
    fn handle_action(&mut self, freya_id: u64, action: &SemAction) -> Result<String, AdapterError>;
    /// Poll the ticker (`poll_n`) — the §14 freya row.
    fn poll(&mut self, step: Duration, times: u32);
    /// One rasterized snapshot as PNG bytes (`render()`).
    fn render(&mut self) -> Result<Vec<u8>, AdapterError>;
    /// Screen size in logical px.
    fn screen_size(&self) -> (f64, f64);
}

/// The freya adapter (§6.1): Level-1 — real AccessKit diffs, real event
/// pipeline, frame-quantized input.
pub struct FreyaAdapter<S: TestingSeam> {
    meta: AdapterMeta,
    ids: AccessibilityMap,
    buffer: SyncEventBuffer,
    seam: S,
    pass_seq: u64,
    clock: ClockState,
    /// The active §13 overlay plan (M83): composited onto every rendered
    /// frame at present time; `None` stops compositing.
    overlay_plan: Option<OverlayPlan>,
    /// The §13 draw-call evidence: one entry per overlay draw.
    overlay_paint_log: Vec<String>,
    /// The latest committed update (the `tree_update` one-shot).
    latest: Option<TreeUpdate>,
    /// Laid-out bounds per accessibility id (the coordinate actions' target).
    bounds: HashMap<u64, [f64; 4]>,
    /// Current editor values per id (the fill's clear count).
    values: HashMap<u64, String>,
}

impl<S: TestingSeam> FreyaAdapter<S> {
    /// Capability row for freya (§6.2): Level-1 tree diffs, real input,
    /// software-Skia headless rendering, ticker-driven clock.
    pub fn new(seam: S) -> Self {
        Self {
            meta: AdapterMeta {
                name: "freya",
                framework: "freya",
                framework_version: "0.5.0".to_string(),
                caps: Caps {
                    tree_update: true,
                    logical_snapshot: false,
                    input: true,
                    last_frame: true,
                    render_frame: true,
                    clock: true,
                },
            },
            ids: AccessibilityMap::new(),
            buffer: SyncEventBuffer::default(),
            seam,
            pass_seq: 0,
            clock: ClockState {
                virtual_time: None,
                paused: false,
            },
            latest: None,
            overlay_plan: None,
            overlay_paint_log: Vec::new(),
            bounds: HashMap::new(),
            values: HashMap::new(),
        }
    }

    /// Drain the overlay paint log (the §13 draw-call evidence).
    pub fn take_overlay_paint_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.overlay_paint_log)
    }

    pub fn ids(&self) -> &AccessibilityMap {
        &self.ids
    }

    /// Mutate the id bridge (tests bind nodes before acting on them).
    pub fn ids_mut(&mut self) -> &mut AccessibilityMap {
        &mut self.ids
    }

    /// Queue input through the frame-quantized buffer (§4.3 ordering).
    pub fn queue(&mut self, event: PendingEvent) {
        self.buffer.push(event);
    }

    fn flush(&mut self) -> Result<usize, AdapterError> {
        let batch = self.buffer.drain();
        let count = batch.len();
        for event in batch {
            let translated = match event {
                PendingEvent::Press { x, y } => PlatformEvent::MouseButtonDown {
                    x,
                    y,
                    button: guiwright_driver::adapter::PointerButton::Primary,
                },
                PendingEvent::Move { x, y } => PlatformEvent::MouseMoved { x, y },
                PendingEvent::Release { x, y } => PlatformEvent::MouseButtonUp {
                    x,
                    y,
                    button: guiwright_driver::adapter::PointerButton::Primary,
                },
                PendingEvent::Wheel { dx, dy } => PlatformEvent::Wheel { dx, dy },
                PendingEvent::Keyboard(key) => PlatformEvent::Keyboard(key),
            };
            self.seam.send_event(translated)?;
        }
        Ok(count)
    }

    /// Absorb one committed update: bind the ids it carries, unbind the
    /// ones the seam reports as removed, and record the geometry the
    /// coordinate actions dispatch at.
    fn absorb(&mut self, update: TreeUpdate) {
        let removed = self.seam.removed_ids();
        self.ids.unbind_all(removed);
        for (id, node) in &update.nodes {
            self.ids.bind(id.0);
            if let Some(rect) = node.bounds() {
                self.bounds
                    .insert(id.0, [rect.x0, rect.y0, rect.x1, rect.y1]);
            } else {
                self.bounds.remove(&id.0);
            }
            match node.value() {
                Some(value) => {
                    self.values.insert(id.0, value.to_string());
                }
                None => {
                    self.values.remove(&id.0);
                }
            }
        }
        // freya 0.5 commits DIFFS: several land between two session pumps
        // (a frame-quantized click runs press→sync→release→sync), and an
        // empty diff must not wipe a pending one — merge instead of
        // replace, later entries winning per node id (the §9 quirk row).
        match &mut self.latest {
            Some(pending) => {
                for (id, node) in update.nodes {
                    if let Some(slot) = pending.nodes.iter_mut().find(|(pid, _)| *pid == id) {
                        *slot = (id, node);
                    } else {
                        pending.nodes.push((id, node));
                    }
                }
                if update.tree.is_some() {
                    pending.tree = update.tree;
                }
                pending.focus = update.focus;
            }
            None => self.latest = Some(update),
        }
    }

    /// One frame-quantized click at a node's laid-out center (§4.3:
    /// press → sync → release).
    fn click_at(&mut self, x: f64, y: f64) -> Result<(), AdapterError> {
        self.queue(PendingEvent::Press { x, y });
        self.flush()?;
        let update = self.seam.sync_and_update();
        self.absorb(update);
        self.queue(PendingEvent::Release { x, y });
        self.flush()?;
        let update = self.seam.sync_and_update();
        self.absorb(update);
        Ok(())
    }

    /// The laid-out center of a bound node.
    fn center_of(&self, freya_id: u64) -> Result<(f64, f64), AdapterError> {
        let [x0, y0, x1, y1] = self.bounds.get(&freya_id).copied().ok_or_else(|| {
            AdapterError::Internal(format!("node {freya_id} has no laid-out bounds"))
        })?;
        Ok((f64::midpoint(x0, x1), f64::midpoint(y0, y1)))
    }

    /// Fill a text field: click it into focus, clear through the real
    /// editor pipeline (backspaces), commit the text (the m79 fill
    /// contract — the upstream `write_text` shape).
    fn fill_at(&mut self, freya_id: u64, text: &str) -> Result<(), AdapterError> {
        let (x, y) = self.center_of(freya_id)?;
        self.click_at(x, y)?;
        let clear = self.values.get(&freya_id).map(String::len).unwrap_or(0);
        for _ in 0..clear {
            self.queue(PendingEvent::Keyboard(crate::keys::KeyEvent {
                phase: crate::keys::KeyPhase::Down,
                key: "backspace".into(),
                modifiers: vec![],
            }));
        }
        if !text.is_empty() {
            self.buffer
                .push(PendingEvent::Keyboard(crate::keys::KeyEvent {
                    phase: crate::keys::KeyPhase::Down,
                    key: text.to_string(),
                    modifiers: vec![],
                }));
        }
        self.flush()?;
        let update = self.seam.sync_and_update();
        self.absorb(update);
        Ok(())
    }
}

impl<S: TestingSeam> std::fmt::Debug for FreyaAdapter<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreyaAdapter")
            .field("meta", &self.meta)
            .field("pass_seq", &self.pass_seq)
            .field("clock", &self.clock)
            .field("tracked_nodes", &self.bounds.len())
            .finish_non_exhaustive()
    }
}

impl<S: TestingSeam + 'static> FrameworkAdapter for FreyaAdapter<S> {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // Frame-quantized input (§4.3): the queued batch is delivered at
        // the frame boundary, then one harness step commits the tree.
        let flushed = self.flush().unwrap_or(0);
        let update = self.seam.sync_and_update();
        self.absorb(update);
        self.pass_seq += 1;
        PumpOutcome {
            frames: vec![guiwright_driver::adapter::FrameStamp {
                seq: self.pass_seq,
                hash: self.pass_seq, // TestingRunner renders deterministically; the pixel digest rides render_frame
            }],
            repaint_requested: false,
            quit_requested: false,
        }
        .with_flushed(flushed)
    }

    fn tree_update(&mut self) -> Option<TreeUpdate> {
        // §4.3: the committed `TreeUpdate` from the last harness step —
        // one-shot per diff (None = unchanged, the Level-1 contract).
        self.latest.take()
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        let freya_id = self
            .ids
            .freya_of(node)
            .ok_or_else(|| AdapterError::Internal(format!("unknown node {node:?}")))?;
        let detail = match action {
            // Native semantic path (§4.3): focus moves through the
            // accessibility tree's own handling.
            SemAction::Focus | SemAction::Blur => self.seam.handle_action(freya_id, &action)?,
            SemAction::Click | SemAction::Check | SemAction::Uncheck => {
                let (x, y) = self.center_of(freya_id)?;
                self.click_at(x, y)?;
                "clicked".to_string()
            }
            SemAction::DoubleClick => {
                let (x, y) = self.center_of(freya_id)?;
                self.click_at(x, y)?;
                self.click_at(x, y)?;
                "double clicked".to_string()
            }
            SemAction::SetValue { text } => {
                self.fill_at(freya_id, &text)?;
                format!("filled {text:?}")
            }
            SemAction::Hover => {
                let (x, y) = self.center_of(freya_id)?;
                self.queue(PendingEvent::Move { x, y });
                self.flush()?;
                let update = self.seam.sync_and_update();
                self.absorb(update);
                "hovered".to_string()
            }
            SemAction::Press { key, modifiers } => {
                // The §4.3 gap: full key sequences with modifiers AND
                // KeyUp ride the raw Keyboard pipeline.
                for ev in crate::keys::key_sequence(&key, &modifiers) {
                    self.queue(PendingEvent::Keyboard(ev));
                }
                self.flush()?;
                let update = self.seam.sync_and_update();
                self.absorb(update);
                format!("pressed {key}")
            }
            SemAction::Scroll { dx, dy } => {
                let (x, y) = self.center_of(freya_id)?;
                self.queue(PendingEvent::Wheel { dx, dy });
                self.flush()?;
                let update = self.seam.sync_and_update();
                self.absorb(update);
                let _ = (x, y);
                format!("scrolled ({dx},{dy})")
            }
        };
        self.pass_seq += 1;
        Ok(ActionOutcome {
            applied_at_frame: self.pass_seq,
            detail,
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        let translated = crate::platform::translate(&ev)?;
        self.buffer.push(match translated {
            PlatformEvent::MouseButtonDown { x, y, .. } => PendingEvent::Press { x, y },
            PlatformEvent::MouseButtonUp { x, y, .. } => PendingEvent::Release { x, y },
            PlatformEvent::MouseMoved { x, y } => PendingEvent::Move { x, y },
            PlatformEvent::Wheel { dx, dy } => PendingEvent::Wheel { dx, dy },
            PlatformEvent::Keyboard(key) => PendingEvent::Keyboard(key),
            PlatformEvent::TextInput { text } => PendingEvent::Keyboard(crate::keys::KeyEvent {
                phase: crate::keys::KeyPhase::Down,
                key: text,
                modifiers: vec![],
            }),
        });
        // Deliver at this frame boundary (§5.1 rendezvous — one pump).
        self.flush()?;
        let update = self.seam.sync_and_update();
        self.absorb(update);
        Ok(())
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        // §14: freya virtual time ticks the RenderingTicker via
        // `TestingRunner::poll(step, duration)` — bound through the seam.
        match cmd {
            ClockCmd::InstallVirtual => self.clock.virtual_time = Some(Duration::ZERO),
            ClockCmd::FastForward(dt) | ClockCmd::RunFor(dt) => {
                self.seam
                    .poll(Duration::from_millis(1), dt.as_millis() as u32);
                let total = self
                    .clock
                    .virtual_time
                    .unwrap_or_default()
                    .saturating_add(dt);
                self.clock.virtual_time = Some(total);
            }
            ClockCmd::Pause => self.clock.paused = true,
            ClockCmd::Resume => self.clock.paused = false,
            // Absolute time: the ticker steps to the target (§14 freya row).
            ClockCmd::SetTime(t) => {
                self.seam
                    .poll(Duration::from_millis(1), t.as_millis() as u32);
                self.clock.virtual_time = Some(t);
            }
            ClockCmd::Probe => {}
        }
        Ok(self.clock)
    }

    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        let _ = size; // the TestingRunner rasterizes at its boot viewport
        let png = self.seam.render()?;
        let image = guiwright_shot::codec::decode_png(&png)
            .map_err(|err| AdapterError::Internal(format!("freya snapshot decode: {err}")))?;
        let (width, height) = (image.width, image.height);
        let mut rgba: Vec<u8> = image.rgba.into();
        // §13 present-time composition (M83): the overlay paints into the
        // frame the runner just rasterized — same engine, no extra pass.
        if let Some(plan) = &self.overlay_plan {
            overlay::composite(plan, &mut rgba, width, height);
            for entry in overlay::paint_log(plan) {
                self.overlay_paint_log.push(entry);
            }
        }
        let pixels: std::sync::Arc<[u8]> = std::sync::Arc::from(rgba);
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
            size: (width, height),
            pixels: Some(pixels),
        })
    }

    fn last_frame(&mut self) -> Option<FrameRef> {
        // No continuous presentation on the testing runner; the frame
        // hash rides the §10 pixel digest of the last render call.
        None
    }

    /// Inspector overlay (§13, M83): store the plan — every
    /// `render_frame` composites it at present time (the runner's own
    /// Skia engine paints the outline + badges into the frame pixels).
    fn show_overlay(
        &mut self,
        plan: Option<&guiwright_inspect::overlay::OverlayPlan>,
    ) -> Result<(), AdapterError> {
        self.overlay_plan = plan.cloned();
        Ok(())
    }
}

/// The **real** seam: `freya_testing::TestingRunner` owned wholesale, its
/// public surface answering the [`TestingSeam`] vocabulary. `T` is the
/// hook's provided root context (the app state the test observes).
pub struct FreyaTestingHarness<T> {
    runner: TestingRunner,
    state: T,
    /// Ids that left the tree at the last step (drained by `removed_ids`
    /// — always empty on freya 0.5 diffs, see `sync_and_update`).
    last_removed: Vec<u64>,
    size: (f64, f64),
}

impl<T: 'static> FreyaTestingHarness<T> {
    /// Boot a real testing runner: physical viewport `size`, scale factor
    /// 1.0 (logical px == physical px, the §9 adapter-owns-scale rule
    /// degenerates), the hook providing the app's root contexts.
    ///
    /// freya 0.5 moved `AppComponent` and `Runner` out of the testing
    /// prelude — they live on `freya_core::{element, runner}` now.
    pub fn boot(
        app: impl Into<freya_core::element::AppComponent>,
        size: (f64, f64),
        hook: impl FnOnce(&mut freya_core::runner::Runner) -> T,
    ) -> Self {
        let (runner, state) = TestingRunner::new(
            app,
            torin::prelude::Size2D::new(size.0 as f32, size.1 as f32),
            hook,
            1.0,
        );
        Self {
            runner,
            state,
            last_removed: Vec::new(),
            size,
        }
    }

    /// The hook-provided app state (the test's observation handle).
    pub fn state(&self) -> &T {
        &self.state
    }

    /// The underlying runner (tests drive `render_to_file` and friends).
    pub fn runner(&mut self) -> &mut TestingRunner {
        &mut self.runner
    }
}

/// Map the mirror key vocabulary onto the real `keyboard_types::Key`
/// (freya 0.5 rides keyboard_types: named keys live under `Key::Named`).
fn real_key(key: &str) -> freya_testing::prelude::Key {
    use freya_testing::prelude::NamedKey;
    type Key = freya_testing::prelude::Key;
    match key {
        "enter" | "return" => Key::Named(NamedKey::Enter),
        "tab" => Key::Named(NamedKey::Tab),
        "escape" => Key::Named(NamedKey::Escape),
        "backspace" => Key::Named(NamedKey::Backspace),
        "delete" => Key::Named(NamedKey::Delete),
        "home" => Key::Named(NamedKey::Home),
        "end" => Key::Named(NamedKey::End),
        "left" => Key::Named(NamedKey::ArrowLeft),
        "right" => Key::Named(NamedKey::ArrowRight),
        "up" => Key::Named(NamedKey::ArrowUp),
        "down" => Key::Named(NamedKey::ArrowDown),
        "shift" => Key::Named(NamedKey::Shift),
        "ctrl" => Key::Named(NamedKey::Control),
        "alt" => Key::Named(NamedKey::Alt),
        "meta" => Key::Named(NamedKey::Meta),
        other => Key::Character(other.into()),
    }
}

/// Map the canonical §8.4 modifier names onto the real bitflags.
fn real_modifiers(modifiers: &[String]) -> freya_testing::prelude::Modifiers {
    use freya_testing::prelude::Modifiers;
    let mut out = Modifiers::empty();
    for modifier in modifiers {
        out |= match modifier.as_str() {
            "shift" => Modifiers::SHIFT,
            "ctrl" => Modifiers::CONTROL,
            "alt" => Modifiers::ALT,
            "meta" => Modifiers::META,
            _ => Modifiers::empty(),
        };
    }
    out
}

impl<T: 'static> std::fmt::Debug for FreyaTestingHarness<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreyaTestingHarness")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<T: 'static> TestingSeam for FreyaTestingHarness<T> {
    fn sync_and_update(&mut self) -> TreeUpdate {
        let update = self.runner.sync_and_update();
        // freya 0.5 commits DIFFS, not full trees: an update carries only
        // the added/updated nodes and NO removal markers, so nothing can
        // be inferred to have left the tree here — the consumer tree
        // handles removals through re-emitted parent children lists, and
        // the id bridge keeps its bindings (the §9 quirk row; a stale
        // binding surfaces at action time, loud).
        self.last_removed = Vec::new();
        update
    }

    fn send_event(&mut self, event: PlatformEvent) -> Result<(), AdapterError> {
        // freya 0.5: `CursorPoint` / `Size2D` live on `torin::prelude`
        // (the testing prelude stopped re-exporting them); the event
        // names and `PlatformEvent` stay on `freya_testing::prelude`.
        use freya_testing::prelude::{
            Code, KeyboardEventName, Modifiers, MouseEventName, TouchPhase, WheelEventName,
            WheelSource,
        };
        use torin::prelude::CursorPoint;
        let real = match event {
            PlatformEvent::MouseMoved { x, y } => freya_testing::prelude::PlatformEvent::Mouse {
                name: MouseEventName::MouseMove,
                cursor: CursorPoint::new(x, y),
                button: None,
            },
            PlatformEvent::MouseButtonDown { x, y, .. } => {
                freya_testing::prelude::PlatformEvent::Mouse {
                    name: MouseEventName::MouseDown,
                    cursor: CursorPoint::new(x, y),
                    button: Some(freya_testing::prelude::MouseButton::Left),
                }
            }
            PlatformEvent::MouseButtonUp { x, y, .. } => {
                freya_testing::prelude::PlatformEvent::Mouse {
                    name: MouseEventName::MouseUp,
                    cursor: CursorPoint::new(x, y),
                    button: Some(freya_testing::prelude::MouseButton::Left),
                }
            }
            PlatformEvent::Wheel { dx, dy } => freya_testing::prelude::PlatformEvent::Wheel {
                name: WheelEventName::Wheel,
                scroll: CursorPoint::new(dx, dy),
                cursor: CursorPoint::new(0.0, 0.0),
                source: WheelSource::Custom,
                phase: TouchPhase::Moved,
            },
            PlatformEvent::Keyboard(key) => {
                let name = match key.phase {
                    crate::keys::KeyPhase::Down => KeyboardEventName::KeyDown,
                    crate::keys::KeyPhase::Up => KeyboardEventName::KeyUp,
                };
                freya_testing::prelude::PlatformEvent::Keyboard {
                    name,
                    key: real_key(&key.key),
                    code: Code::Unidentified,
                    modifiers: real_modifiers(&key.modifiers),
                }
            }
            // Text commits as one character KeyDown — the upstream
            // `write_text` shape (the Input editor inserts the string).
            PlatformEvent::TextInput { text } => freya_testing::prelude::PlatformEvent::Keyboard {
                name: KeyboardEventName::KeyDown,
                key: freya_testing::prelude::Key::Character(text.into()),
                code: Code::Unidentified,
                modifiers: Modifiers::default(),
            },
        };
        self.runner.send_event(real);
        Ok(())
    }

    fn removed_ids(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.last_removed)
    }

    fn handle_action(&mut self, freya_id: u64, action: &SemAction) -> Result<String, AdapterError> {
        let (accesskit_action, label) = match action {
            SemAction::Focus => (accesskit::Action::Focus, "focus"),
            SemAction::Blur => (accesskit::Action::Blur, "blur"),
            // The rest rides the native `EventName::AccessibilityAction`
            // route (§4.3) — elements that declare the a11y action fire.
            _ => (accesskit::Action::Click, "accessibility-action"),
        };
        let queued = self
            .runner
            .send_accessibility_action(accesskit::ActionRequest {
                action: accesskit_action,
                // accesskit 0.25 split the target into tree + node: the
                // runner's committed tree is the AccessKit root (§4.3).
                target_tree: accesskit::TreeId::ROOT,
                target_node: NodeId(freya_id),
                data: None,
            });
        Ok(format!(
            "{label} {}",
            if queued { "queued" } else { "rejected" }
        ))
    }

    fn poll(&mut self, step: Duration, times: u32) {
        self.runner.poll_n(step, times);
    }

    fn render(&mut self) -> Result<Vec<u8>, AdapterError> {
        // The runner rasterizes its N32 surface and PNG-encodes it; the
        // adapter decodes back to RGBA (§10 capture pipeline).
        Ok(self.runner.render().to_vec())
    }

    fn screen_size(&self) -> (f64, f64) {
        self.size
    }
}

trait PumpExt {
    fn with_flushed(self, flushed: usize) -> PumpOutcome;
}

impl PumpExt for PumpOutcome {
    fn with_flushed(self, flushed: usize) -> PumpOutcome {
        let _ = flushed; // recorded in traces via the session's event stream
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::key_sequence;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared seam: the adapter owns it, the test observes the same log.
    type SharedLog = Rc<RefCell<Vec<String>>>;

    fn empty_update() -> TreeUpdate {
        TreeUpdate {
            nodes: vec![],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(0),
        }
    }

    struct LoggingRunner {
        log: SharedLog,
        removed: Vec<u64>,
    }

    impl LoggingRunner {
        fn new() -> (Self, SharedLog) {
            let log: SharedLog = Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    log: Rc::clone(&log),
                    removed: Vec::new(),
                },
                log,
            )
        }
    }

    impl TestingSeam for LoggingRunner {
        fn sync_and_update(&mut self) -> TreeUpdate {
            self.log.borrow_mut().push("sync".into());
            empty_update()
        }
        fn send_event(&mut self, event: PlatformEvent) -> Result<(), AdapterError> {
            self.log.borrow_mut().push(format!("event:{event:?}"));
            Ok(())
        }
        fn removed_ids(&mut self) -> Vec<u64> {
            std::mem::take(&mut self.removed)
        }
        fn handle_action(
            &mut self,
            freya_id: u64,
            action: &SemAction,
        ) -> Result<String, AdapterError> {
            self.log
                .borrow_mut()
                .push(format!("action:{freya_id}:{}", action.describe()));
            Ok("focused".into())
        }
        fn poll(&mut self, step: Duration, times: u32) {
            self.log.borrow_mut().push(format!("poll:{step:?}x{times}"));
        }
        fn render(&mut self) -> Result<Vec<u8>, AdapterError> {
            Ok(vec![])
        }
        fn screen_size(&self) -> (f64, f64) {
            (400.0, 300.0)
        }
    }

    fn logging_adapter() -> (FreyaAdapter<LoggingRunner>, SharedLog) {
        let (seam, log) = LoggingRunner::new();
        (FreyaAdapter::new(seam), log)
    }

    #[test]
    fn caps_row_is_level_1() {
        let (adapter, _log) = logging_adapter();
        let caps = &adapter.meta().caps;
        assert!(caps.tree_update, "real AccessKit diffs");
        assert!(!caps.logical_snapshot);
        assert!(caps.input);
        assert!(caps.render_frame, "software Skia");
    }

    #[test]
    fn input_flushes_at_the_frame_boundary_in_canonical_order() {
        let (mut adapter, log) = logging_adapter();
        // Queue out of order: release first (a producer race).
        adapter.queue(PendingEvent::Release { x: 8.0, y: 8.0 });
        adapter.queue(PendingEvent::Press { x: 8.0, y: 8.0 });
        adapter.queue(PendingEvent::Move { x: 8.0, y: 8.0 });
        let deadline = FrameDeadline::within(Duration::from_millis(10));
        let _ = adapter.pump(deadline);
        let entries = log.borrow();
        let delivered: Vec<&String> = entries.iter().filter(|e| e.starts_with("event:")).collect();
        assert_eq!(delivered.len(), 3, "all three flushed: {entries:?}");
        assert!(
            delivered[0].contains("MouseDown"),
            "press first: {delivered:?}"
        );
        assert!(
            delivered[1].contains("MouseMove"),
            "move second: {delivered:?}"
        );
        assert!(
            delivered[2].contains("MouseUp"),
            "release third: {delivered:?}"
        );
        assert!(entries.iter().any(|e| e == "sync"), "one harness step ran");
    }

    #[test]
    fn keyboard_input_uses_the_full_sequence() {
        let (mut adapter, log) = logging_adapter();
        for ev in key_sequence("a", &["Control".into()]) {
            adapter.queue(PendingEvent::Keyboard(ev));
        }
        let deadline = FrameDeadline::within(Duration::from_millis(10));
        let _ = adapter.pump(deadline);
        let entries = log.borrow();
        let keys: Vec<&String> = entries.iter().filter(|e| e.contains("Keyboard")).collect();
        assert_eq!(
            keys.len(),
            4,
            "ctrl-down, a-down, a-up, ctrl-up: {entries:?}"
        );
        assert!(
            keys[0].contains("phase: Down") && keys[0].contains("ctrl"),
            "first is ctrl-down: {keys:?}"
        );
        assert!(keys[2].contains("phase: Up") && keys[2].contains("\"a\""));
        assert!(keys[3].contains("ctrl"), "modifier releases last (reverse)");
    }

    #[test]
    fn native_actions_route_through_handle_action() {
        let (mut adapter, log) = logging_adapter();
        let node = adapter.ids_mut().bind(11);
        let outcome = adapter
            .act(node, SemAction::Focus)
            .expect("focus through the seam");
        assert_eq!(outcome.detail, "focused");
        assert!(outcome.applied_at_frame > 0);
        assert!(
            log.borrow().iter().any(|e| e == "action:11:focus"),
            "the freya id reached the seam: {:?}",
            log.borrow()
        );
    }

    #[test]
    fn coordinate_actions_demand_laid_out_bounds() {
        let (mut adapter, _log) = logging_adapter();
        let node = adapter.ids_mut().bind(7);
        let err = adapter
            .act(node, SemAction::Click)
            .expect_err("no bounds yet");
        assert!(err.to_string().contains("no laid-out bounds"), "{err}");
    }

    #[test]
    fn unknown_nodes_are_loud() {
        let (mut adapter, _log) = logging_adapter();
        let err = adapter
            .act(NodeId(999), SemAction::Click)
            .expect_err("unbound node");
        assert!(err.to_string().contains("unknown node"), "{err}");
    }

    #[test]
    fn clock_polls_the_ticker_through_the_seam() {
        let (mut adapter, log) = logging_adapter();
        adapter.clock(ClockCmd::InstallVirtual).expect("install");
        let state = adapter
            .clock(ClockCmd::FastForward(Duration::from_millis(9)))
            .expect("ff");
        assert_eq!(state.virtual_time, Some(Duration::from_millis(9)));
        assert!(
            log.borrow().iter().any(|e| e.contains("poll:1msx9")),
            "the ticker stepped through the seam: {:?}",
            log.borrow()
        );
    }

    #[test]
    fn key_up_phase_reaches_the_buffer() {
        let (mut adapter, log) = logging_adapter();
        adapter
            .input(InputEvent::KeyUp {
                key: "Escape".into(),
                modifiers: vec![],
            })
            .expect("key up");
        let entries = log.borrow();
        assert!(
            entries
                .iter()
                .any(|e| e.contains("phase: Up") && e.contains("escape")),
            "the up phase is delivered: {entries:?}"
        );
    }
}
