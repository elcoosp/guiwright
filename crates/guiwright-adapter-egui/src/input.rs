//! Synthetic input translation: Guiwright's [`InputEvent`] vocabulary →
//! `egui::Event` (architecture §9 "Fidelity rule": synthetic events enter
//! at the same choke point as OS events — here, `RawInput.events`).

use egui::{Event, Modifiers, Pos2, Rect};

use guiwright_driver::adapter::{InputEvent, PointerButton};

/// Translate one Guiwright input event into 0..n egui events.
pub fn translate_input(ev: InputEvent, _screen_rect: Rect) -> Vec<Event> {
    match ev {
        InputEvent::PointerDown { x, y, button } => vec![Event::PointerButton {
            pos: Pos2::new(x as f32, y as f32),
            button: translate_button(button),
            pressed: true,
            modifiers: Modifiers::default(),
        }],
        InputEvent::PointerUp { x, y, button } => vec![Event::PointerButton {
            pos: Pos2::new(x as f32, y as f32),
            button: translate_button(button),
            pressed: false,
            modifiers: Modifiers::default(),
        }],
        InputEvent::PointerMove { x, y } => {
            vec![Event::PointerMoved(Pos2::new(x as f32, y as f32))]
        }
        InputEvent::Wheel { dx, dy } => vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::Vec2::new(dx as f32, dy as f32),
            modifiers: Modifiers::default(),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        }],
        InputEvent::KeyDown { key, modifiers } => named_key(&key, &modifiers).into_iter().collect(),
        InputEvent::KeyUp { key, modifiers } => match named_key_raw(&key, &modifiers) {
            Some(mut ev) => {
                if let Event::Key { pressed, .. } = &mut ev {
                    *pressed = false;
                }
                vec![ev]
            }
            None => vec![],
        },
        InputEvent::TextInput { text } => vec![Event::Text(text)],
    }
}

fn translate_button(button: PointerButton) -> egui::PointerButton {
    match button {
        PointerButton::Primary => egui::PointerButton::Primary,
        PointerButton::Secondary => egui::PointerButton::Secondary,
        PointerButton::Middle => egui::PointerButton::Middle,
    }
}

/// Physical-key translation for a small, honest subset (M0): navigation and
/// editing keys. Text entry uses `TextInput`/`SetValue` semantics, which is
/// what the actionability layer drives anyway (§8.4).
pub fn named_key(key: &str, modifiers: &[String]) -> Option<Event> {
    let mut ev = named_key_raw(key, modifiers)?;
    if let Event::Key { pressed, .. } = &mut ev {
        *pressed = true;
    }
    Some(ev)
}

fn named_key_raw(key: &str, modifiers: &[String]) -> Option<Event> {
    use egui::Key;
    let egui_key = match key {
        "Enter" => Key::Enter,
        "Escape" => Key::Escape,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Space" => Key::Space,
        "ArrowUp" | "Up" => Key::ArrowUp,
        "ArrowDown" | "Down" => Key::ArrowDown,
        "ArrowLeft" | "Left" => Key::ArrowLeft,
        "ArrowRight" | "Right" => Key::ArrowRight,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Insert" => Key::Insert,
        _ => return None,
    };
    Some(Event::Key {
        key: egui_key,
        physical_key: None,
        repeat: false,
        pressed: false,
        modifiers: translate_modifiers(modifiers),
    })
}

fn translate_modifiers(names: &[String]) -> Modifiers {
    let mut m = Modifiers::default();
    for name in names {
        match name.to_lowercase().as_str() {
            "ctrl" | "control" => m.ctrl = true,
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            "super" | "cmd" | "meta" => m.mac_cmd = true,
            _ => {}
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_gesture_translation() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(100.0));
        let down = translate_input(
            InputEvent::PointerDown {
                x: 10.0,
                y: 12.0,
                button: PointerButton::Primary,
            },
            rect,
        );
        assert_eq!(down.len(), 1);
        match &down[0] {
            Event::PointerButton {
                pos,
                pressed,
                button,
                ..
            } => {
                assert_eq!(*pos, Pos2::new(10.0, 12.0));
                assert!(pressed);
                assert_eq!(*button, egui::PointerButton::Primary);
            }
            other => panic!("unexpected {other:?}"),
        }

        let up = translate_input(
            InputEvent::PointerUp {
                x: 10.0,
                y: 12.0,
                button: PointerButton::Primary,
            },
            rect,
        );
        match &up[0] {
            Event::PointerButton { pressed, .. } => assert!(!pressed),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn keyboard_translation() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(100.0));
        let down = translate_input(
            InputEvent::KeyDown {
                key: "Enter".into(),
                modifiers: vec!["shift".into()],
            },
            rect,
        );
        match &down[0] {
            Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } => {
                assert_eq!(*key, egui::Key::Enter);
                assert!(pressed);
                assert!(modifiers.shift);
            }
            other => panic!("unexpected {other:?}"),
        }

        let up = translate_input(
            InputEvent::KeyUp {
                key: "Enter".into(),
                modifiers: vec![],
            },
            rect,
        );
        match &up[0] {
            Event::Key { pressed, .. } => assert!(!pressed),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn text_input_translates_to_text_event() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(100.0));
        let evs = translate_input(InputEvent::TextInput { text: "ada".into() }, rect);
        assert_eq!(evs, vec![Event::Text("ada".into())]);
    }
}
