//! Gesture synthesis (architecture §9 "Gesture synthesis", M7): the
//! driver-side recipes that turn the wire's high-level [`Gesture`]
//! vocabulary into the raw [`InputEvent`] sequence the adapter's real
//! pipeline injects. Synthesis lives in the driver — not in the clients —
//! so each framework's quirks stay encoded in one place, driven by the
//! adapter's [`GesturePacing`] table (§18 "per-adapter gesture pacing
//! tables, validated by cross-framework gesture tests").
//!
//! The §9 recipes: click = move→down→up; drag = down→ratchet-interpolated
//! moves→up; keyboard = down→up with text commits riding `TextInput` (the
//! per-framework IME-vs-keys translation is the adapter's §4.2 job). The
//! press→release "settle" rides the adapter's canonical-order frame flush —
//! the sequence order produced here is the contract (e.g. the freya
//! harness applies its event buffer at the frame boundary, in order).

use crate::adapter::{GesturePacing, InputEvent, PointerButton};
use guiwright_proto::schema::{GestureV1, InputEventV1, PointerButtonV1};

/// A high-level synthetic gesture (§9). Coordinates are logical px
/// **relative to the target window** — adapters own the scale conversion
/// (§9 "adapters convert once, they own scale factor").
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    /// Press and release at a position, `count` times at the same point
    /// (Playwright's clickCount: 1 = click, 2 = double-click).
    Click {
        x: f64,
        y: f64,
        button: PointerButton,
        count: u32,
    },
    /// Press, travel to another position through interpolated moves, release.
    Drag {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        button: PointerButton,
    },
    /// Wheel scroll by logical-px deltas.
    Wheel { dx: f64, dy: f64 },
    /// Press and release a key (the §8.4 key-name vocabulary).
    Key { key: String, modifiers: Vec<String> },
    /// Commit text in one step (§4.2).
    Type { text: String },
}

/// Synthesize the raw event sequence for `gesture` under `pacing` (§9).
/// Total: every gesture yields at least one event except a zero-count
/// click (which yields none — the session rejects that as a client bug).
pub fn synthesize(gesture: &Gesture, pacing: &GesturePacing) -> Vec<InputEvent> {
    match gesture {
        Gesture::Click {
            x,
            y,
            button,
            count,
        } => {
            // A zero-count click synthesizes nothing — the session rejects
            // the empty batch as a client bug before any injection.
            if *count == 0 {
                return Vec::new();
            }
            let mut events = Vec::with_capacity(1 + 2 * *count as usize);
            events.push(InputEvent::PointerMove { x: *x, y: *y });
            for _ in 0..*count {
                events.push(InputEvent::PointerDown {
                    x: *x,
                    y: *y,
                    button: *button,
                });
                events.push(InputEvent::PointerUp {
                    x: *x,
                    y: *y,
                    button: *button,
                });
            }
            events
        }
        Gesture::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => {
            let dx = to_x - from_x;
            let dy = to_y - from_y;
            // §9 ratchet: the longest axis is cut into ≤ ratchet-px steps.
            // A non-positive ratchet collapses to one move.
            let ratchet = if pacing.drag_ratchet_px > 0.0 {
                pacing.drag_ratchet_px
            } else {
                f64::INFINITY
            };
            let steps = ((dx.abs().max(dy.abs())) / ratchet).ceil().max(1.0) as usize;
            let mut events = Vec::with_capacity(steps + 2);
            events.push(InputEvent::PointerDown {
                x: *from_x,
                y: *from_y,
                button: *button,
            });
            for i in 1..=steps {
                if i == steps {
                    // The last move lands exactly on the target so the
                    // release coordinates are honest.
                    events.push(InputEvent::PointerMove { x: *to_x, y: *to_y });
                } else {
                    let t = i as f64 / steps as f64;
                    events.push(InputEvent::PointerMove {
                        x: from_x + dx * t,
                        y: from_y + dy * t,
                    });
                }
            }
            events.push(InputEvent::PointerUp {
                x: *to_x,
                y: *to_y,
                button: *button,
            });
            events
        }
        Gesture::Wheel { dx, dy } => vec![InputEvent::Wheel { dx: *dx, dy: *dy }],
        Gesture::Key { key, modifiers } => vec![
            InputEvent::KeyDown {
                key: key.clone(),
                modifiers: modifiers.clone(),
            },
            InputEvent::KeyUp {
                key: key.clone(),
                modifiers: modifiers.clone(),
            },
        ],
        Gesture::Type { text } => vec![InputEvent::TextInput { text: text.clone() }],
    }
}

impl From<PointerButtonV1> for PointerButton {
    fn from(b: PointerButtonV1) -> Self {
        match b {
            PointerButtonV1::Primary => PointerButton::Primary,
            PointerButtonV1::Secondary => PointerButton::Secondary,
            PointerButtonV1::Middle => PointerButton::Middle,
        }
    }
}

impl From<InputEventV1> for InputEvent {
    fn from(ev: InputEventV1) -> Self {
        match ev {
            InputEventV1::PointerDown { x, y, button } => InputEvent::PointerDown {
                x,
                y,
                button: button.into(),
            },
            InputEventV1::PointerUp { x, y, button } => InputEvent::PointerUp {
                x,
                y,
                button: button.into(),
            },
            InputEventV1::PointerMove { x, y } => InputEvent::PointerMove { x, y },
            InputEventV1::Wheel { dx, dy } => InputEvent::Wheel { dx, dy },
            InputEventV1::KeyDown { key, modifiers } => InputEvent::KeyDown { key, modifiers },
            InputEventV1::KeyUp { key, modifiers } => InputEvent::KeyUp { key, modifiers },
            InputEventV1::TextInput { text } => InputEvent::TextInput { text },
        }
    }
}

impl From<GestureV1> for Gesture {
    fn from(g: GestureV1) -> Self {
        match g {
            GestureV1::Click {
                x,
                y,
                button,
                count,
            } => Gesture::Click {
                x,
                y,
                button: button.into(),
                count,
            },
            GestureV1::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
            } => Gesture::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button: button.into(),
            },
            GestureV1::Wheel { dx, dy } => Gesture::Wheel { dx, dy },
            GestureV1::Key { key, modifiers } => Gesture::Key { key, modifiers },
            GestureV1::Type { text } => Gesture::Type { text },
        }
    }
}

// --- Codegen recording (§13 over §9) ----------------------------------------

/// The kit surface a recorded input command rides (§13): the input
/// vocabulary emits kit code against `app.mouse()` / `app.keyboard()`,
/// never against a locator (coordinates, not nodes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodegenSurface {
    Mouse,
    Keyboard,
}

impl CodegenSurface {
    /// The kit accessor the call chain starts from.
    #[must_use]
    pub fn accessor(self) -> &'static str {
        match self {
            Self::Mouse => "app.mouse()",
            Self::Keyboard => "app.keyboard()",
        }
    }
}

/// The kit-code call for a gesture (§13 raw-input recording): the method
/// call **suffix** (e.g. `click(10.0, 20.0)`) — callers compose the full
/// line with [`codegen_line`]. Returns the surface, the suffix, and a
/// warning when the gesture has no exact kit equivalent (a clickCount>2
/// click records as a single click, Playwright's recorded-code shape).
///
/// Floats render as explicit `f64` literals (Debug), strings as
/// `{:?}`-quoted literals; inputs are expected finite (wire clients send
/// finite coordinates — the adapter owns rejection).
#[must_use]
pub fn codegen_call(gesture: &Gesture) -> (CodegenSurface, String, Option<String>) {
    match gesture {
        Gesture::Click {
            x,
            y,
            button,
            count,
        } => {
            let count_warning = (*count > 2).then(|| {
                format!("clickCount {count} has no kit equivalent — recorded as a single click")
            });
            let (call, warning) = match (*count, button) {
                (1, PointerButton::Primary) => (format!("click({x:?}, {y:?})"), None),
                (1, b) => (
                    format!("click_button({x:?}, {y:?}, PointerButton::{b:?})"),
                    None,
                ),
                (2, PointerButton::Primary) => (format!("dblclick({x:?}, {y:?})"), None),
                (2, b) => (
                    format!("dblclick_button({x:?}, {y:?}, PointerButton::{b:?})"),
                    None,
                ),
                (_, PointerButton::Primary) => (format!("click({x:?}, {y:?})"), count_warning),
                (_, b) => (
                    format!("click_button({x:?}, {y:?}, PointerButton::{b:?})"),
                    count_warning,
                ),
            };
            (CodegenSurface::Mouse, call, warning)
        }
        Gesture::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => {
            let call = match button {
                PointerButton::Primary => {
                    format!("drag({from_x:?}, {from_y:?}, {to_x:?}, {to_y:?})")
                }
                b => format!(
                    "drag_button({from_x:?}, {from_y:?}, {to_x:?}, {to_y:?}, PointerButton::{b:?})"
                ),
            };
            (CodegenSurface::Mouse, call, None)
        }
        Gesture::Wheel { dx, dy } => (
            CodegenSurface::Mouse,
            format!("wheel({dx:?}, {dy:?})"),
            None,
        ),
        Gesture::Key { key, modifiers } if modifiers.is_empty() => {
            (CodegenSurface::Keyboard, format!("press({key:?})"), None)
        }
        Gesture::Key { key, modifiers } => {
            let mods = modifiers
                .iter()
                .map(|m| format!("{m:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            (
                CodegenSurface::Keyboard,
                format!("press_with({key:?}, [{mods}])"),
                None,
            )
        }
        Gesture::Type { text } => (
            CodegenSurface::Keyboard,
            format!("type_text({text:?})"),
            None,
        ),
    }
}

/// The kit-code call for one raw input event (the `Input` half of the
/// vocabulary): pointer primitives render as the kit's raw methods; the
/// keyboard primitives (`KeyDown` / `KeyUp` / `TextInput`) render as
/// `None` — they have no standalone kit form and in practice ride the
/// gesture vocabulary (`key` / `text` commands), which renders directly.
#[must_use]
pub fn codegen_event_call(ev: &InputEvent) -> Option<(CodegenSurface, String)> {
    match ev {
        InputEvent::PointerMove { x, y } => {
            Some((CodegenSurface::Mouse, format!("move_to({x:?}, {y:?})")))
        }
        InputEvent::PointerDown { x, y, button } => Some((
            CodegenSurface::Mouse,
            format!("down({x:?}, {y:?}, PointerButton::{button:?})"),
        )),
        InputEvent::PointerUp { x, y, button } => Some((
            CodegenSurface::Mouse,
            format!("up({x:?}, {y:?}, PointerButton::{button:?})"),
        )),
        InputEvent::Wheel { dx, dy } => {
            Some((CodegenSurface::Mouse, format!("wheel({dx:?}, {dy:?})")))
        }
        InputEvent::KeyDown { .. } | InputEvent::KeyUp { .. } | InputEvent::TextInput { .. } => {
            None
        }
    }
}

/// Compose the full recorded line (§13): `app.mouse().click(10.0, 20.0);`
/// — with the `in_window` chain when the command carried an explicit
/// window target (the M6 vocabulary; an implicit default target records
/// bare — the line replays against the session default at replay time).
#[must_use]
pub fn codegen_line(surface: CodegenSurface, call: &str, window: Option<&str>) -> String {
    match window {
        Some(w) => format!("{}.in_window({w:?}).{call};", surface.accessor()),
        None => format!("{}.{};", surface.accessor(), call),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_pacing() -> GesturePacing {
        GesturePacing::default()
    }

    #[test]
    fn click_sequence_is_move_then_press_release_per_count() {
        let events = synthesize(
            &Gesture::Click {
                x: 12.5,
                y: 30.0,
                button: PointerButton::Primary,
                count: 1,
            },
            &default_pacing(),
        );
        assert_eq!(
            events,
            vec![
                InputEvent::PointerMove { x: 12.5, y: 30.0 },
                InputEvent::PointerDown {
                    x: 12.5,
                    y: 30.0,
                    button: PointerButton::Primary
                },
                InputEvent::PointerUp {
                    x: 12.5,
                    y: 30.0,
                    button: PointerButton::Primary
                },
            ]
        );

        // count = 2 repeats the press at the same point (§9 "double-click").
        let events = synthesize(
            &Gesture::Click {
                x: 0.0,
                y: 0.0,
                button: PointerButton::Secondary,
                count: 2,
            },
            &default_pacing(),
        );
        assert_eq!(events.len(), 5);
        assert!(matches!(events[3], InputEvent::PointerDown { .. }));

        // count = 0 is the client-bug shape: no events (the session
        // rejects the empty synthesis before touching the adapter).
        let events = synthesize(
            &Gesture::Click {
                x: 0.0,
                y: 0.0,
                button: PointerButton::Primary,
                count: 0,
            },
            &default_pacing(),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn drag_ratchet_cuts_the_longest_axis_into_steps() {
        // 100 px horizontal @ 5 px/tick → 20 moves; the path starts one
        // ratchet from the press and ends exactly on the release point.
        let events = synthesize(
            &Gesture::Drag {
                from_x: 10.0,
                from_y: 10.0,
                to_x: 110.0,
                to_y: 10.0,
                button: PointerButton::Primary,
            },
            &default_pacing(),
        );
        assert_eq!(events.len(), 22);
        assert!(matches!(
            &events[0],
            InputEvent::PointerDown {
                x: 10.0,
                y: 10.0,
                ..
            }
        ));
        assert!(matches!(
            &events[1],
            InputEvent::PointerMove { x: 15.0, y: 10.0 }
        ));
        assert!(matches!(
            &events[20],
            InputEvent::PointerMove { x: 110.0, y: 10.0 }
        ));
        assert!(matches!(
            &events[21],
            InputEvent::PointerUp {
                x: 110.0,
                y: 10.0,
                ..
            }
        ));

        // Diagonal: the dominant axis paces the steps; both coordinates
        // interpolate (30,30) → (90,60): 60 px dominant @ 5 → 12 moves.
        let events = synthesize(
            &Gesture::Drag {
                from_x: 30.0,
                from_y: 30.0,
                to_x: 90.0,
                to_y: 60.0,
                button: PointerButton::Primary,
            },
            &default_pacing(),
        );
        assert_eq!(events.len(), 14);
        assert!(matches!(
            &events[1],
            InputEvent::PointerMove { x: 35.0, y: 32.5 }
        ));

        // Zero-length drag: one move at the (identical) target.
        let events = synthesize(
            &Gesture::Drag {
                from_x: 5.0,
                from_y: 5.0,
                to_x: 5.0,
                to_y: 5.0,
                button: PointerButton::Primary,
            },
            &default_pacing(),
        );
        assert_eq!(events.len(), 3);

        // A non-positive ratchet collapses to a single move (no division
        // by zero, no zero-step drag).
        let events = synthesize(
            &Gesture::Drag {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 100.0,
                to_y: 0.0,
                button: PointerButton::Primary,
            },
            &GesturePacing {
                drag_ratchet_px: 0.0,
            },
        );
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn keyboard_and_wheel_are_direct_sequences() {
        let events = synthesize(
            &Gesture::Key {
                key: "Enter".into(),
                modifiers: vec!["Shift".into()],
            },
            &default_pacing(),
        );
        assert_eq!(
            events,
            vec![
                InputEvent::KeyDown {
                    key: "Enter".into(),
                    modifiers: vec!["Shift".into()]
                },
                InputEvent::KeyUp {
                    key: "Enter".into(),
                    modifiers: vec!["Shift".into()]
                },
            ]
        );

        let events = synthesize(
            &Gesture::Type {
                text: "héllo".into(),
            },
            &default_pacing(),
        );
        assert_eq!(
            events,
            vec![InputEvent::TextInput {
                text: "héllo".into()
            }]
        );

        let events = synthesize(
            &Gesture::Wheel {
                dx: 0.0,
                dy: -120.0,
            },
            &default_pacing(),
        );
        assert_eq!(
            events,
            vec![InputEvent::Wheel {
                dx: 0.0,
                dy: -120.0
            }]
        );
    }

    #[test]
    fn wire_conversions_preserve_the_vocabulary() {
        // Every InputEventV1 shape converts losslessly.
        let cases = vec![
            InputEventV1::PointerDown {
                x: 1.0,
                y: 2.0,
                button: PointerButtonV1::Middle,
            },
            InputEventV1::PointerUp {
                x: 1.0,
                y: 2.0,
                button: PointerButtonV1::Secondary,
            },
            InputEventV1::PointerMove { x: 3.0, y: 4.0 },
            InputEventV1::Wheel { dx: 5.0, dy: -6.0 },
            InputEventV1::KeyDown {
                key: "a".into(),
                modifiers: vec!["Ctrl".into()],
            },
            InputEventV1::KeyUp {
                key: "a".into(),
                modifiers: vec![],
            },
            InputEventV1::TextInput {
                text: "héllo".into(),
            },
        ];
        for case in cases {
            let converted: InputEvent = case.clone().into();
            let expected = match case {
                InputEventV1::PointerDown { x, y, button } => InputEvent::PointerDown {
                    x,
                    y,
                    button: button.into(),
                },
                InputEventV1::PointerUp { x, y, button } => InputEvent::PointerUp {
                    x,
                    y,
                    button: button.into(),
                },
                InputEventV1::PointerMove { x, y } => InputEvent::PointerMove { x, y },
                InputEventV1::Wheel { dx, dy } => InputEvent::Wheel { dx, dy },
                InputEventV1::KeyDown { key, modifiers } => InputEvent::KeyDown { key, modifiers },
                InputEventV1::KeyUp { key, modifiers } => InputEvent::KeyUp { key, modifiers },
                InputEventV1::TextInput { text } => InputEvent::TextInput { text },
            };
            assert_eq!(converted, expected);
        }

        // The gesture conversion keeps shape and button.
        let gesture: Gesture = GestureV1::Drag {
            from_x: 0.0,
            from_y: 0.0,
            to_x: 10.0,
            to_y: 10.0,
            button: PointerButtonV1::Middle,
        }
        .into();
        assert_eq!(
            gesture,
            Gesture::Drag {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 10.0,
                to_y: 10.0,
                button: PointerButton::Middle
            }
        );
    }
}
