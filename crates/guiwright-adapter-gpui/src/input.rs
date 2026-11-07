//! The input mapping (§4.5, §9): Guiwright's normalized vocabulary →
//! GPUI `PlatformInput` steps.
//!
//! `Window::dispatch_event(PlatformInput)` (`window.rs:5864`) is the
//! canonical seam; real backends feed it through `platform_window.on_input`
//! — synthetic events are bit-identical to OS input. The
//! `VisualTestContext` vocabulary (`app/test_context.rs:935-1068`) defines
//! the keystroke surface: `simulate_keystrokes("cmd-shift-p enter")`.
//! [`parse_keystrokes`] implements that parser; [`translate`] maps the
//! driver's events onto dispatch steps.

use guiwright_driver::adapter::{InputEvent, PointerButton};

/// GPUI modifiers (§4.5 `Keystroke.modifiers`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Control,
    Alt,
    Shift,
    /// `cmd` on macOS, `super` elsewhere (GPUI's `platform` modifier).
    Platform,
    Function,
}

impl Modifier {
    pub fn parse(name: &str) -> Option<Modifier> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Modifier::Control),
            "alt" | "option" => Some(Modifier::Alt),
            "shift" => Some(Modifier::Shift),
            "cmd" | "command" | "super" | "logo" | "meta" => Some(Modifier::Platform),
            "fn" | "function" => Some(Modifier::Function),
            _ => None,
        }
    }
}

/// One keystroke: modifiers + key (`"cmd-shift-p"` → `{[Platform, Shift], "p"}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyStroke {
    pub modifiers: Vec<Modifier>,
    pub key: String,
}

/// A GPUI input step (the glue wraps each into `PlatformInput` and calls
/// `Window::dispatch_event`, in order).
#[derive(Clone, Debug, PartialEq)]
pub enum GpuiInputStep {
    MouseMove { x: f64, y: f64 },
    MouseDown { x: f64, y: f64, button: GpuiButton },
    MouseUp { x: f64, y: f64, button: GpuiButton },
    Scroll { dx: f64, dy: f64 },
    KeyDown { keystroke: KeyStroke },
    KeyUp { keystroke: KeyStroke },
    ImeCommit { text: String },
}

/// GPUI mouse button vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuiButton {
    Left,
    Right,
    Middle,
}

impl From<PointerButton> for GpuiButton {
    fn from(b: PointerButton) -> Self {
        match b {
            PointerButton::Primary => GpuiButton::Left,
            PointerButton::Secondary => GpuiButton::Right,
            PointerButton::Middle => GpuiButton::Middle,
        }
    }
}

/// Parse the `VisualTestContext` keystroke vocabulary (§4.5
/// `simulate_keystrokes("cmd-shift-p enter")`): space-separated strokes,
/// dash-joined modifiers per stroke.
pub fn parse_keystrokes(vocabulary: &str) -> Vec<KeyStroke> {
    vocabulary.split_whitespace().map(parse_stroke).collect()
}

fn parse_stroke(stroke: &str) -> KeyStroke {
    let mut modifiers = Vec::new();
    let mut key = stroke.to_string();
    while let Some(dash) = key.find('-') {
        let (head, rest) = key.split_at(dash);
        let Some(after) = rest.strip_prefix('-') else {
            break;
        };
        match Modifier::parse(head) {
            Some(m) => {
                modifiers.push(m);
                key = after.to_string();
            }
            None => break, // a literal key containing '-' (e.g. "up-left")
        }
    }
    KeyStroke {
        modifiers,
        key: key.to_ascii_lowercase(),
    }
}

/// One Guiwright input → GPUI dispatch steps (§9 "synthetic events enter
/// at the same choke point as OS events").
pub fn translate(ev: InputEvent) -> Vec<GpuiInputStep> {
    match ev {
        InputEvent::PointerDown { x, y, button } => vec![GpuiInputStep::MouseDown {
            x,
            y,
            button: button.into(),
        }],
        InputEvent::PointerUp { x, y, button } => vec![GpuiInputStep::MouseUp {
            x,
            y,
            button: button.into(),
        }],
        InputEvent::PointerMove { x, y } => vec![GpuiInputStep::MouseMove { x, y }],
        InputEvent::Wheel { dx, dy } => vec![GpuiInputStep::Scroll { dx, dy }],
        InputEvent::KeyDown { key, modifiers } => {
            let mut steps: Vec<GpuiInputStep> = modifiers
                .iter()
                .filter_map(|m| Modifier::parse(m))
                .map(|m| GpuiInputStep::KeyDown {
                    keystroke: KeyStroke {
                        modifiers: vec![m],
                        key: m_name(m).to_string(),
                    },
                })
                .collect();
            steps.push(GpuiInputStep::KeyDown {
                keystroke: KeyStroke {
                    modifiers: modifiers
                        .iter()
                        .filter_map(|m| Modifier::parse(m))
                        .collect(),
                    key: key.to_ascii_lowercase(),
                },
            });
            steps
        }
        InputEvent::KeyUp { key, modifiers } => vec![GpuiInputStep::KeyUp {
            keystroke: KeyStroke {
                modifiers: modifiers
                    .iter()
                    .filter_map(|m| Modifier::parse(m))
                    .collect(),
                key: key.to_ascii_lowercase(),
            },
        }],
        InputEvent::TextInput { text } => vec![GpuiInputStep::ImeCommit { text }],
    }
}

fn m_name(m: Modifier) -> &'static str {
    match m {
        Modifier::Control => "ctrl",
        Modifier::Alt => "alt",
        Modifier::Shift => "shift",
        Modifier::Platform => "cmd",
        Modifier::Function => "fn",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystroke_vocabulary_parses_like_visual_test_context() {
        // §4.5 "simulate_keystrokes(\"cmd-shift-p enter\")".
        assert_eq!(
            parse_keystrokes("cmd-shift-p enter"),
            vec![
                KeyStroke {
                    modifiers: vec![Modifier::Platform, Modifier::Shift],
                    key: "p".into(),
                },
                KeyStroke {
                    modifiers: vec![],
                    key: "enter".into(),
                },
            ]
        );
    }

    #[test]
    fn literal_dashes_stay_in_keys() {
        // A key containing '-' that is not a modifier prefix survives.
        assert_eq!(
            parse_stroke("up-down"),
            KeyStroke {
                modifiers: vec![],
                key: "up-down".into(),
            }
        );
    }

    #[test]
    fn pointer_vocabulary_maps_one_to_one() {
        assert_eq!(
            translate(InputEvent::PointerDown {
                x: 10.0,
                y: 20.0,
                button: PointerButton::Primary,
            }),
            vec![GpuiInputStep::MouseDown {
                x: 10.0,
                y: 20.0,
                button: GpuiButton::Left,
            }]
        );
        assert_eq!(
            translate(InputEvent::Wheel { dx: 0.0, dy: -3.0 }),
            vec![GpuiInputStep::Scroll { dx: 0.0, dy: -3.0 }]
        );
    }

    #[test]
    fn key_down_prepends_modifier_downs() {
        let steps = translate(InputEvent::KeyDown {
            key: "P".into(),
            modifiers: vec!["cmd".into(), "shift".into()],
        });
        assert_eq!(
            steps,
            vec![
                GpuiInputStep::KeyDown {
                    keystroke: KeyStroke {
                        modifiers: vec![Modifier::Platform],
                        key: "cmd".into(),
                    }
                },
                GpuiInputStep::KeyDown {
                    keystroke: KeyStroke {
                        modifiers: vec![Modifier::Shift],
                        key: "shift".into(),
                    }
                },
                GpuiInputStep::KeyDown {
                    keystroke: KeyStroke {
                        modifiers: vec![Modifier::Platform, Modifier::Shift],
                        key: "p".into(),
                    }
                },
            ]
        );
    }

    #[test]
    fn text_input_is_ime_commit() {
        assert_eq!(
            translate(InputEvent::TextInput { text: "ada".into() }),
            vec![GpuiInputStep::ImeCommit { text: "ada".into() }]
        );
    }
}
