//! The embedded input surface (architecture §9 over the kit; the M7
//! deferred row "kit-embedded input"): `app.mouse()` / `app.keyboard()`
//! drive the session's gesture synthesis and raw-input injection
//! directly — the same Playwright-shaped vocabulary the REPL and the
//! TypeScript client speak over the wire, embedded and in-process.
//!
//! Synthesis stays in the driver ([`guiwright_driver::synthesize`], the
//! adapter's [`guiwright_driver::GesturePacing`] table applies), every
//! batch rides the same per-window rendezvous injection the wire uses
//! (one pump, §15.4), and each command lands in the trace as a `mouse` /
//! `keyboard` event (the same event log the locator actions feed).
//!
//! ```ignore
//! app.mouse().click(10.0, 20.0)?;
//! app.mouse().drag(10.0, 10.0, 110.0, 10.0)?;
//! app.keyboard().press("Enter")?;
//! app.mouse().in_window(&window_handle).click(4.0, 8.0)?; // multi-window
//! ```

use guiwright_driver::adapter::{InputEvent, PointerButton};
use guiwright_driver::session::GestureOutcome;
use guiwright_driver::{Gesture, codegen_call, codegen_event_call, codegen_line};

use crate::app::{App, KitError};

/// `app.mouse()` — the Playwright-shaped pointer surface (§9): the
/// synthesized gestures (`click` / `dblclick` / `drag` / `wheel`) and the
/// raw primitives (`move_to` / `down` / `up`, the `Input` half of the
/// vocabulary). Coordinates are logical px relative to the target
/// window; [`MouseHandle::in_window`] pins the target for multi-window
/// apps (the M6 vocabulary), the session default applies otherwise.
pub struct MouseHandle<'a, A: guiwright_driver::FrameworkAdapter> {
    app: &'a mut App<A>,
    window: Option<String>,
}

impl<A: guiwright_driver::FrameworkAdapter> std::fmt::Debug for MouseHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MouseHandle")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl<'a, A: guiwright_driver::FrameworkAdapter> MouseHandle<'a, A> {
    pub(crate) fn new(app: &'a mut App<A>) -> Self {
        Self { app, window: None }
    }

    /// Pin the target window for the subsequent commands (§9 coordinates
    /// are window-relative; the M6 handle vocabulary). The handle is
    /// validated against the tree by the session when the next command
    /// applies — a dead handle fails with a clean `UnknownWindow` and
    /// injects nothing.
    pub fn in_window(&mut self, handle: impl Into<String>) -> &mut Self {
        self.window = Some(handle.into());
        self
    }

    /// `mouse.click(x, y)` — press and release with the primary button.
    pub fn click(&mut self, x: f64, y: f64) -> Result<GestureOutcome, KitError> {
        self.click_button(x, y, PointerButton::Primary)
    }

    /// `mouse.click(x, y, { button })` — press and release with an
    /// explicit button.
    pub fn click_button(
        &mut self,
        x: f64,
        y: f64,
        button: PointerButton,
    ) -> Result<GestureOutcome, KitError> {
        self.gesture(Gesture::Click {
            x,
            y,
            button,
            count: 1,
        })
    }

    /// `mouse.dblclick(x, y)` — two presses at the same point
    /// (Playwright's clickCount 2).
    pub fn dblclick(&mut self, x: f64, y: f64) -> Result<GestureOutcome, KitError> {
        self.gesture(Gesture::Click {
            x,
            y,
            button: PointerButton::Primary,
            count: 2,
        })
    }

    /// `mouse.dblclick(x, y, { button })` — two presses with an explicit
    /// button.
    pub fn dblclick_button(
        &mut self,
        x: f64,
        y: f64,
        button: PointerButton,
    ) -> Result<GestureOutcome, KitError> {
        self.gesture(Gesture::Click {
            x,
            y,
            button,
            count: 2,
        })
    }

    /// `mouse.drag(from, to)` — press, travel through ratchet-interpolated
    /// moves (the adapter's pacing table), release.
    pub fn drag(
        &mut self,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
    ) -> Result<GestureOutcome, KitError> {
        self.drag_button(from_x, from_y, to_x, to_y, PointerButton::Primary)
    }

    /// `mouse.drag(from, to, { button })` — drag with an explicit button.
    pub fn drag_button(
        &mut self,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        button: PointerButton,
    ) -> Result<GestureOutcome, KitError> {
        self.gesture(Gesture::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        })
    }

    /// `mouse.wheel(dx, dy)` — wheel scroll by logical-px deltas.
    pub fn wheel(&mut self, dx: f64, dy: f64) -> Result<GestureOutcome, KitError> {
        self.gesture(Gesture::Wheel { dx, dy })
    }

    /// `mouse.move(x, y)` (raw primitive): one `PointerMove` batch. The
    /// returned frame seq is the `InputQueued` reply's.
    pub fn move_to(&mut self, x: f64, y: f64) -> Result<u64, KitError> {
        let frame = self.raw(InputEvent::PointerMove { x, y })?;
        self.app
            .record_event("mouse", format!("move to ({x}, {y})"));
        Ok(frame)
    }

    /// `mouse.down(x, y, { button })` (raw primitive): press without
    /// release — release rides [`MouseHandle::up`].
    pub fn down(&mut self, x: f64, y: f64, button: PointerButton) -> Result<u64, KitError> {
        let frame = self.raw(InputEvent::PointerDown { x, y, button })?;
        self.app.record_event("mouse", format!("down ({x}, {y})"));
        Ok(frame)
    }

    /// `mouse.up(x, y, { button })` (raw primitive): release.
    pub fn up(&mut self, x: f64, y: f64, button: PointerButton) -> Result<u64, KitError> {
        let frame = self.raw(InputEvent::PointerUp { x, y, button })?;
        self.app.record_event("mouse", format!("up ({x}, {y})"));
        Ok(frame)
    }

    /// One synthesized gesture through the session (§9): the adapter's
    /// pacing table drives synthesis, one rendezvous pump applies the
    /// batch, the outcome reports the applied frame and the detail line.
    /// Applied gestures record as kit code while a codegen recording runs
    /// (§13 over §9) — only after the batch actually applied.
    fn gesture(&mut self, gesture: Gesture) -> Result<GestureOutcome, KitError> {
        let rendered = if self.app.codegen_active() {
            Some(codegen_call(&gesture))
        } else {
            None
        };
        let outcome = self
            .app
            .session
            .perform_gesture(gesture, self.window.as_deref())?;
        self.app.record_event("mouse", outcome.detail.clone());
        if let Some((surface, call, warning)) = rendered {
            let line = codegen_line(surface, &call, self.window.as_deref());
            self.app.record_input_code(line, warning);
        }
        Ok(outcome)
    }

    /// One raw event through the session's `send_input` (the `Input`
    /// request's embedded twin). Pointer primitives record as kit code
    /// while a codegen recording runs (§13 over §9).
    fn raw(&mut self, ev: InputEvent) -> Result<u64, KitError> {
        let rendered = if self.app.codegen_active() {
            codegen_event_call(&ev)
        } else {
            None
        };
        let frame = self
            .app
            .session
            .send_input(vec![ev], self.window.as_deref())?;
        if let Some((surface, call)) = rendered {
            let line = codegen_line(surface, &call, self.window.as_deref());
            self.app.record_input_code(line, None);
        }
        Ok(frame)
    }
}

/// `app.keyboard()` — the Playwright-shaped keyboard surface (§9): `press`
/// and `type` ride the driver's synthesis (key down → key up; text commits
/// ride `TextInput`, the per-framework IME-vs-keys translation stays the
/// adapter's §4.2 job).
pub struct KeyboardHandle<'a, A: guiwright_driver::FrameworkAdapter> {
    app: &'a mut App<A>,
    window: Option<String>,
}

impl<A: guiwright_driver::FrameworkAdapter> std::fmt::Debug for KeyboardHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyboardHandle")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl<'a, A: guiwright_driver::FrameworkAdapter> KeyboardHandle<'a, A> {
    pub(crate) fn new(app: &'a mut App<A>) -> Self {
        Self { app, window: None }
    }

    /// Pin the target window for the subsequent commands (§9; the M6
    /// handle vocabulary). See [`MouseHandle::in_window`].
    pub fn in_window(&mut self, handle: impl Into<String>) -> &mut Self {
        self.window = Some(handle.into());
        self
    }

    /// `keyboard.press(key)` — key down + key up, no modifiers.
    pub fn press(&mut self, key: &str) -> Result<GestureOutcome, KitError> {
        self.press_with(key, &[])
    }

    /// `keyboard.press(key, { modifiers })` — key down + key up with the
    /// modifier chord (the §8.4 key-name vocabulary, e.g. `"Enter"`,
    /// `"KeyA"` with `["Control", "Shift"]`).
    pub fn press_with(
        &mut self,
        key: &str,
        modifiers: &[&str],
    ) -> Result<GestureOutcome, KitError> {
        let gesture = Gesture::Key {
            key: key.to_string(),
            modifiers: modifiers.iter().map(|m| (*m).to_string()).collect(),
        };
        let rendered = if self.app.codegen_active() {
            Some(codegen_call(&gesture))
        } else {
            None
        };
        let outcome = self
            .app
            .session
            .perform_gesture(gesture, self.window.as_deref())?;
        self.app.record_event("keyboard", outcome.detail.clone());
        if let Some((surface, call, warning)) = rendered {
            let line = codegen_line(surface, &call, self.window.as_deref());
            self.app.record_input_code(line, warning);
        }
        Ok(outcome)
    }

    /// `keyboard.type(text)` — commit text in one step (§4.2).
    pub fn type_text(&mut self, text: &str) -> Result<GestureOutcome, KitError> {
        let gesture = Gesture::Type {
            text: text.to_string(),
        };
        let rendered = if self.app.codegen_active() {
            Some(codegen_call(&gesture))
        } else {
            None
        };
        let outcome = self
            .app
            .session
            .perform_gesture(gesture, self.window.as_deref())?;
        self.app.record_event("keyboard", outcome.detail.clone());
        if let Some((surface, call, warning)) = rendered {
            let line = codegen_line(surface, &call, self.window.as_deref());
            self.app.record_input_code(line, warning);
        }
        Ok(outcome)
    }
}
