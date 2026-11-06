//! Input vocabulary translation (§9, Level-2 flavor): the harness executes
//! synthetic input through the framework's **real** event pipeline (iced:
//! `UserInterface::update(&[core::Event])`, §4.6 "Injection =
//! `UserInterface::update(&[core::Event])`"). This module is the checked
//! boundary between the driver's [`InputEvent`] and the pipeline events.
//!
//! Frame-quantization caveat (§9): move → down → up ordering is the
//! harness's responsibility; the translation is pure and total.

use guiwright_driver::adapter::{AdapterError, InputEvent, PointerButton};

/// Pointer operations through the real pipeline (logical px coordinates,
/// window-relative — the adapter owns any scale conversion, §9).
#[derive(Clone, Debug, PartialEq)]
pub enum PointerOp {
    Move {
        x: f64,
        y: f64,
    },
    Down {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    Up {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    Wheel {
        dx: f64,
        dy: f64,
    },
}

/// Keyboard operations through the real pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyOp {
    Down { key: String, modifiers: Vec<String> },
    Up { key: String, modifiers: Vec<String> },
    Text { text: String },
}

/// One translated pipeline input.
#[derive(Clone, Debug, PartialEq)]
pub enum InputOp {
    Pointer(PointerOp),
    Key(KeyOp),
}

/// Translate a driver [`InputEvent`] into the pipeline vocabulary.
pub fn translate(ev: &InputEvent) -> Result<InputOp, AdapterError> {
    Ok(match ev {
        InputEvent::PointerDown { x, y, button } => InputOp::Pointer(PointerOp::Down {
            x: *x,
            y: *y,
            button: *button,
        }),
        InputEvent::PointerUp { x, y, button } => InputOp::Pointer(PointerOp::Up {
            x: *x,
            y: *y,
            button: *button,
        }),
        InputEvent::PointerMove { x, y } => InputOp::Pointer(PointerOp::Move { x: *x, y: *y }),
        InputEvent::Wheel { dx, dy } => InputOp::Pointer(PointerOp::Wheel { dx: *dx, dy: *dy }),
        InputEvent::KeyDown { key, modifiers } => InputOp::Key(KeyOp::Down {
            key: key.clone(),
            modifiers: modifiers.clone(),
        }),
        InputEvent::KeyUp { key, modifiers } => InputOp::Key(KeyOp::Up {
            key: key.clone(),
            modifiers: modifiers.clone(),
        }),
        InputEvent::TextInput { text } => InputOp::Key(KeyOp::Text { text: text.clone() }),
    })
}

/// The modifier vocabulary (normalized lowercase, §9 "keyboard fidelity").
pub fn normalize_modifiers(modifiers: &[String]) -> Vec<String> {
    let mut out: Vec<String> = modifiers
        .iter()
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| !m.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_vocabulary_passes_through() {
        let op = translate(&InputEvent::PointerDown {
            x: 12.0,
            y: 34.0,
            button: PointerButton::Primary,
        })
        .expect("translate");
        assert_eq!(
            op,
            InputOp::Pointer(PointerOp::Down {
                x: 12.0,
                y: 34.0,
                button: PointerButton::Primary
            })
        );
        let op = translate(&InputEvent::Wheel {
            dx: 0.0,
            dy: -120.0,
        })
        .expect("translate");
        assert_eq!(
            op,
            InputOp::Pointer(PointerOp::Wheel {
                dx: 0.0,
                dy: -120.0
            })
        );
    }

    #[test]
    fn keyboard_vocabulary_passes_through() {
        let op = translate(&InputEvent::KeyDown {
            key: "a".into(),
            modifiers: vec!["Control".into()],
        })
        .expect("translate");
        assert_eq!(
            op,
            InputOp::Key(KeyOp::Down {
                key: "a".into(),
                modifiers: vec!["Control".into()]
            })
        );
        let op = translate(&InputEvent::TextInput {
            text: "héllo".into(),
        })
        .expect("translate");
        assert_eq!(
            op,
            InputOp::Key(KeyOp::Text {
                text: "héllo".into()
            })
        );
    }

    #[test]
    fn modifiers_normalize_sorted_and_deduped() {
        assert_eq!(
            normalize_modifiers(&["Shift".into(), "CTRL".into(), " ctrl ".into(), "".into()]),
            vec!["ctrl", "shift"]
        );
    }

    #[test]
    fn translation_is_total() {
        // The driver vocabulary is closed — every variant translates.
        for ev in [
            InputEvent::PointerMove { x: 1.0, y: 2.0 },
            InputEvent::PointerUp {
                x: 1.0,
                y: 2.0,
                button: PointerButton::Middle,
            },
            InputEvent::KeyUp {
                key: "Escape".into(),
                modifiers: vec![],
            },
        ] {
            assert!(translate(&ev).is_ok());
        }
    }
}
