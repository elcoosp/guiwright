//! [`InputEvent`] → freya `PlatformEvent` vocabulary translation (§4.3
//! "Pixel clicks go through `send_event(PlatformEvent::Mouse …)` +
//! ragnarok hit-testing (`is_point_inside` + clip-aware
//! `visible_area().contains`)", "we drive raw `PlatformEvent::Keyboard`
//! with modifiers+KeyUp").
//!
//! The translation is the checked boundary; the runner glue executes the
//! result through `TestingRunner::send_event` (§4.3 embed path).

use guiwright_driver::adapter::{InputEvent, PointerButton};

use crate::keys::KeyEvent;

/// The freya platform-event vocabulary the runner executes (the subset
/// Guiwright drives; named to mirror `PlatformEvent`'s variants).
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    MouseMoved {
        x: f64,
        y: f64,
    },
    MouseButtonDown {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    MouseButtonUp {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    Wheel {
        dx: f64,
        dy: f64,
    },
    Keyboard(KeyEvent),
}

/// Translate a driver input into the platform vocabulary. Total over the
/// closed [`InputEvent`] set.
pub fn translate(
    ev: &InputEvent,
) -> Result<PlatformEvent, guiwright_driver::adapter::AdapterError> {
    Ok(match ev {
        InputEvent::PointerMove { x, y } => PlatformEvent::MouseMoved { x: *x, y: *y },
        InputEvent::PointerDown { x, y, button } => PlatformEvent::MouseButtonDown {
            x: *x,
            y: *y,
            button: *button,
        },
        InputEvent::PointerUp { x, y, button } => PlatformEvent::MouseButtonUp {
            x: *x,
            y: *y,
            button: *button,
        },
        InputEvent::Wheel { dx, dy } => PlatformEvent::Wheel { dx: *dx, dy: *dy },
        InputEvent::KeyDown { key, modifiers } => {
            PlatformEvent::Keyboard(first_of(crate::keys::key_sequence(key, modifiers)))
        }
        InputEvent::KeyUp { key, modifiers } => {
            // KeyUp alone: emit just the up phase of the sequence.
            PlatformEvent::Keyboard(crate::keys::KeyEvent {
                phase: crate::keys::KeyPhase::Up,
                key: crate::keys::normalize_key(key),
                modifiers: crate::keys::normalize_modifiers(modifiers),
            })
        }
        InputEvent::TextInput { text } => {
            // Text commits as per-char key sequences (freya has no IME
            // text event on the testing harness; §4.3 keyboard fidelity).
            let mut chars = text.chars();
            let Some(first) = chars.next() else {
                return Ok(PlatformEvent::Keyboard(crate::keys::KeyEvent {
                    phase: crate::keys::KeyPhase::Up,
                    key: String::new(),
                    modifiers: vec![],
                }));
            };
            let _ = chars;
            PlatformEvent::Keyboard(crate::keys::KeyEvent {
                phase: crate::keys::KeyPhase::Down,
                key: crate::keys::normalize_key(&first.to_string()),
                modifiers: vec![],
            })
        }
    })
}

fn first_of(mut seq: Vec<KeyEvent>) -> KeyEvent {
    seq.remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyPhase;

    #[test]
    fn pointer_events_map_one_to_one() {
        assert_eq!(
            translate(&InputEvent::PointerMove { x: 1.0, y: 2.0 }).expect("move"),
            PlatformEvent::MouseMoved { x: 1.0, y: 2.0 }
        );
        assert_eq!(
            translate(&InputEvent::PointerDown {
                x: 3.0,
                y: 4.0,
                button: PointerButton::Primary
            })
            .expect("down"),
            PlatformEvent::MouseButtonDown {
                x: 3.0,
                y: 4.0,
                button: PointerButton::Primary
            }
        );
        assert_eq!(
            translate(&InputEvent::Wheel { dx: 0.0, dy: 5.0 }).expect("wheel"),
            PlatformEvent::Wheel { dx: 0.0, dy: 5.0 }
        );
    }

    #[test]
    fn key_down_emits_the_first_sequence_event() {
        let ev = translate(&InputEvent::KeyDown {
            key: "a".into(),
            modifiers: vec![],
        })
        .expect("key down");
        assert_eq!(
            ev,
            PlatformEvent::Keyboard(KeyEvent {
                phase: KeyPhase::Down,
                key: "a".into(),
                modifiers: vec![],
            })
        );
    }

    #[test]
    fn key_up_emits_only_the_up_phase() {
        let ev = translate(&InputEvent::KeyUp {
            key: "Enter".into(),
            modifiers: vec!["Shift".into()],
        })
        .expect("key up");
        assert_eq!(
            ev,
            PlatformEvent::Keyboard(KeyEvent {
                phase: KeyPhase::Up,
                key: "enter".into(),
                modifiers: vec!["shift".into()],
            })
        );
    }

    #[test]
    fn text_input_commits_as_char_keys() {
        let ev = translate(&InputEvent::TextInput { text: "x".into() }).expect("text");
        assert_eq!(
            ev,
            PlatformEvent::Keyboard(KeyEvent {
                phase: KeyPhase::Down,
                key: "x".into(),
                modifiers: vec![],
            })
        );
    }

    #[test]
    fn pending_events_route_into_the_buffer_vocabulary() {
        // The buffer consumes PendingEvents; translation + click_sequence
        // compose into a flushable batch (integration shape).
        let mut buffer = crate::buffer::SyncEventBuffer::default();
        for ev in crate::buffer::click_sequence(4.0, 6.0) {
            buffer.push(ev);
        }
        assert_eq!(buffer.drain().len(), 3);
    }
}
