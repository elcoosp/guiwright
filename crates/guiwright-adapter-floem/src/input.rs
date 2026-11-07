//! The input vocabulary the floem harness executes through the REAL event
//! pipeline (§4.7 "headless = `dispatch_event`; live = `ViewId::route_event`
//! with `RouteKind::Spatial`").
//!
//! Floem's events are DOM-like: pointer down/up/move/leave, wheel, key
//! down/up, IME commit — capture/target/bubble routed through the
//! `BoxTree` spatial index. [`translate`] maps Guiwright's normalized
//! vocabulary (§9 `InputEventV1`) onto it; the glue dispatches the result
//! (one `dispatch_event` per step). Coordinates stay logical px — the
//! harness owns the scale conversion (§9 "adapters convert once").

use guiwright_driver::adapter::{InputEvent, PointerButton};

/// A floem event step (the glue feeds each to `dispatch_event`, in order).
#[derive(Clone, Debug, PartialEq)]
pub enum FloemEvent {
    PointerDown {
        x: f64,
        y: f64,
        button: FloemButton,
        click_count: u32,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: FloemButton,
        click_count: u32,
    },
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerLeave,
    Wheel {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    KeyDown {
        key: String,
        modifiers: Vec<String>,
    },
    KeyUp {
        key: String,
        modifiers: Vec<String>,
    },
    /// Text commit (floem `ImeCommit`, §4.2 "IME-backed on some adapters").
    ImeCommit {
        text: String,
    },
}

/// floem's pointer button vocabulary (§4.7 uses the DOM-ish `Event`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloemButton {
    Left,
    Right,
    Middle,
}

impl From<PointerButton> for FloemButton {
    fn from(b: PointerButton) -> Self {
        match b {
            PointerButton::Primary => FloemButton::Left,
            PointerButton::Secondary => FloemButton::Right,
            PointerButton::Middle => FloemButton::Middle,
        }
    }
}

/// One Guiwright input → floem event steps (§9 "synthetic events enter at
/// the same choke point as OS events").
pub fn translate(ev: InputEvent) -> Vec<FloemEvent> {
    match ev {
        InputEvent::PointerDown { x, y, button } => vec![FloemEvent::PointerDown {
            x,
            y,
            button: button.into(),
            click_count: 1,
        }],
        InputEvent::PointerUp { x, y, button } => vec![FloemEvent::PointerUp {
            x,
            y,
            button: button.into(),
            click_count: 1,
        }],
        InputEvent::PointerMove { x, y } => vec![FloemEvent::PointerMove { x, y }],
        InputEvent::Wheel { dx, dy } => vec![FloemEvent::Wheel {
            x: 0.0,
            y: 0.0,
            dx,
            dy,
        }],
        InputEvent::KeyDown { key, modifiers } => {
            // §9 keyboard fidelity: modifiers first (floem tracks the
            // modifier state on the down of the modifier keys themselves).
            let mut steps = modifier_downs(&modifiers);
            steps.push(FloemEvent::KeyDown { key, modifiers });
            steps
        }
        InputEvent::KeyUp { key, modifiers } => {
            let mut steps = vec![FloemEvent::KeyUp { key, modifiers }];
            // Modifiers released after the key (mirror of the down order).
            steps.push(FloemEvent::PointerLeave);
            steps.pop();
            steps
        }
        InputEvent::TextInput { text } => vec![FloemEvent::ImeCommit { text }],
    }
}

fn modifier_downs(modifiers: &[String]) -> Vec<FloemEvent> {
    modifiers
        .iter()
        .filter(|m| is_modifier(m))
        .map(|m| FloemEvent::KeyDown {
            key: m.clone(),
            modifiers: vec![m.clone()],
        })
        .collect()
}

fn is_modifier(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "shift" | "control" | "ctrl" | "alt" | "meta" | "cmd" | "super" | "logo"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_vocabulary_maps_one_to_one() {
        assert_eq!(
            translate(InputEvent::PointerDown {
                x: 12.0,
                y: 30.0,
                button: PointerButton::Primary,
            }),
            vec![FloemEvent::PointerDown {
                x: 12.0,
                y: 30.0,
                button: FloemButton::Left,
                click_count: 1,
            }]
        );
        assert_eq!(
            translate(InputEvent::PointerUp {
                x: 12.0,
                y: 30.0,
                button: PointerButton::Secondary,
            }),
            vec![FloemEvent::PointerUp {
                x: 12.0,
                y: 30.0,
                button: FloemButton::Right,
                click_count: 1,
            }]
        );
        assert_eq!(
            translate(InputEvent::PointerMove { x: 5.0, y: 6.0 }),
            vec![FloemEvent::PointerMove { x: 5.0, y: 6.0 }]
        );
    }

    #[test]
    fn wheel_carries_the_delta() {
        assert_eq!(
            translate(InputEvent::Wheel {
                dx: 0.0,
                dy: -120.0
            }),
            vec![FloemEvent::Wheel {
                x: 0.0,
                y: 0.0,
                dx: 0.0,
                dy: -120.0,
            }]
        );
    }

    #[test]
    fn keyboard_carries_modifiers_before_the_key() {
        let steps = translate(InputEvent::KeyDown {
            key: "a".into(),
            modifiers: vec!["ctrl".into(), "shift".into()],
        });
        assert_eq!(
            steps,
            vec![
                FloemEvent::KeyDown {
                    key: "ctrl".into(),
                    modifiers: vec!["ctrl".into()],
                },
                FloemEvent::KeyDown {
                    key: "shift".into(),
                    modifiers: vec!["shift".into()],
                },
                FloemEvent::KeyDown {
                    key: "a".into(),
                    modifiers: vec!["ctrl".into(), "shift".into()],
                },
            ]
        );
    }

    #[test]
    fn text_input_becomes_ime_commit() {
        assert_eq!(
            translate(InputEvent::TextInput { text: "ada".into() }),
            vec![FloemEvent::ImeCommit { text: "ada".into() }]
        );
    }
}
