//! The m80 consumer app: a real gpui counter+form (the §17 row 7
//! consumer). Everything interactive is a real gpui element — `div()`
//! buttons with real `on_click` listeners over real entity state, and a
//! focused text field whose real `on_key_down` listener writes real
//! `Keystroke`s into the entity. The §8.1 test ids ride gpui's own
//! `debug_selector` (the framework records the laid-out bounds of every
//! named element at paint — the walk's geometry source), and the roles
//! ride gpui's own `role(accesskit::Role)` DSL.

use accesskit_gpui as accesskit;
use gpui::{Context, FocusHandle, KeyDownEvent, Render, Window, div, prelude::*, px, rgb};

/// The app state: real gpui entity fields, mutated only by real event
/// listeners (§5.1 — the adapter never touches widgets off-thread).
pub struct CounterForm {
    pub count: i32,
    pub name: String,
    /// The form field's real focus handle (focus follows a real click;
    /// key events route to the focused element's `on_key_down`).
    pub name_input_focus: FocusHandle,
}

impl CounterForm {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            count: 0,
            name: String::new(),
            name_input_focus: cx.focus_handle(),
        }
    }
}

/// One text keystroke into the focused field (the real handler the
/// dispatched `KeyDown` events hit).
fn type_into_field(state: &mut CounterForm, event: &KeyDownEvent) {
    let keystroke = &event.keystroke;
    match keystroke.key.as_str() {
        "backspace" => {
            state.name.pop();
        }
        key if key.chars().count() == 1 => {
            // Uppercase rides the shift modifier — the same shape the
            // real IME-free keyboards produce (`shift+a` → "A").
            let ch = if keystroke.modifiers.shift {
                key.to_uppercase()
            } else {
                key.to_string()
            };
            if !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
            {
                state.name.push_str(&ch);
            }
        }
        _ => {}
    }
}

impl Render for CounterForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count_label = format!("Count: {}", self.count);
        let greeting = format!("Welcome, {}", self.name);
        // Real painted surfaces (m87): the capture's scene is the quad
        // scene the render pipeline paints, so the app carries real
        // backgrounds — a base, the two buttons, the field — and a bar
        // whose width tracks the count (the repaint the §10 video rows
        // record: a click moves real pixels, not just text).
        div()
            .size_full()
            .bg(rgb(0xf8fafc))
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .id("increment")
                    .role(accesskit::Role::Button)
                    .debug_selector(|| "increment".into())
                    .bg(rgb(0x2563eb))
                    .rounded(px(6.0))
                    .on_click(cx.listener(|this, _event, _window, _cx| {
                        this.count += 1;
                    }))
                    .child("Increment"),
            )
            .child(
                div()
                    .id("decrement")
                    .role(accesskit::Role::Button)
                    .debug_selector(|| "decrement".into())
                    .bg(rgb(0x64748b))
                    .rounded(px(6.0))
                    .on_click(cx.listener(|this, _event, _window, _cx| {
                        this.count -= 1;
                    }))
                    .child("Decrement"),
            )
            .child(
                div()
                    .id("name-input")
                    .role(accesskit::Role::TextInput)
                    .debug_selector(|| "name-input".into())
                    .bg(rgb(0xe2e8f0))
                    .rounded(px(4.0))
                    .track_focus(&self.name_input_focus)
                    .on_click(cx.listener(|this, _event, window, cx| {
                        window.focus(&this.name_input_focus, cx);
                    }))
                    .on_key_down(cx.listener(|this, event, _window, _cx| {
                        type_into_field(this, event);
                    }))
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .id("count-label")
                    .role(accesskit::Role::Label)
                    .debug_selector(|| "count-label".into())
                    .child(count_label),
            )
            .child(
                div()
                    .id("count-bar")
                    .debug_selector(|| "count-bar".into())
                    .w(px(24.0 + 12.0 * self.count.clamp(0, 24) as f32))
                    .h(px(8.0))
                    .rounded(px(2.0))
                    .bg(rgb(0x0f766e)),
            )
            .child(
                div()
                    .id("greeting")
                    .role(accesskit::Role::Label)
                    .debug_selector(|| "greeting".into())
                    .child(greeting),
            )
    }
}
