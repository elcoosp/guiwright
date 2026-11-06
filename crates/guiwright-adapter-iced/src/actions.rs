//! [`SemAction`] → [`WidgetOp`] routing (§8.4 action translation, Level-2
//! flavor): the table that turns a Playwright-shaped action on a resolved
//! node into the framework-neutral widget operation the harness executes.
//!
//! Hover/scroll never reach this table — the session rides the real input
//! pipeline with resolved bounds (§8.4, driver `perform_action`); the
//! table answers `Unsupported` for them so a bounds-less call is loud, not
//! silent.

use accesskit::Role;

use guiwright_driver::adapter::{AdapterError, SemAction};

/// The widget operation the harness executes on the framework (iced:
/// `UserInterface::update` events / widget ops, §4.6).
#[derive(Clone, Debug, PartialEq)]
pub enum WidgetOp {
    /// Press the widget (buttons, checkboxes, radios).
    Press(u64),
    /// Replace the widget's content (text inputs).
    SetContent(u64, String),
    /// Move keyboard focus to the widget.
    Focus(u64),
    /// Remove keyboard focus from the widget.
    Blur(u64),
    /// The action needs no widget op (already handled, or a no-op here).
    None,
}

/// Route `action` for the node with `role` and framework `handle`.
///
/// The role comes from the resolution (the registry's mapping, §4.6), the
/// handle from the minted `NodeId` (engine `handle_of`).
pub fn route(role: Role, action: &SemAction, handle: u64) -> Result<WidgetOp, AdapterError> {
    match (role, action) {
        (
            Role::Button
            | Role::CheckBox
            | Role::RadioButton
            | Role::Switch
            | Role::DefaultButton
            | Role::MenuItem
            | Role::Tab,
            SemAction::Click | SemAction::DoubleClick | SemAction::Check | SemAction::Uncheck,
        ) => Ok(WidgetOp::Press(handle)),
        (
            Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::PasswordInput
            | Role::EmailInput
            | Role::NumberInput,
            SemAction::SetValue { text },
        ) => Ok(WidgetOp::SetContent(handle, text.clone())),
        (_, SemAction::SetValue { .. }) => Err(AdapterError::Unsupported(
            "set_value on a non-editable widget",
        )),
        (_, SemAction::Focus) => Ok(WidgetOp::Focus(handle)),
        (_, SemAction::Blur) => Ok(WidgetOp::Blur(handle)),
        // Rides the input pipeline (session, §8.4) — loud here.
        (_, SemAction::Hover) => Err(AdapterError::Unsupported(
            "hover rides the input pipeline (needs resolved bounds)",
        )),
        (_, SemAction::Scroll { .. }) => Err(AdapterError::Unsupported(
            "scroll rides the input pipeline (needs resolved bounds)",
        )),
        (_, SemAction::Press { .. }) => Err(AdapterError::Unsupported(
            "keyboard press on a widget target (fill via set_value instead)",
        )),
        // Role is #[non_exhaustive] — the catch-all keeps unknown roles
        // loud too (a click on a non-pressable widget is an app bug).
        (_, SemAction::Click | SemAction::DoubleClick | SemAction::Check | SemAction::Uncheck) => {
            Err(AdapterError::Unsupported("press on a non-pressable widget"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressable_roles_press_on_click() {
        for role in [
            Role::Button,
            Role::CheckBox,
            Role::RadioButton,
            Role::Switch,
            Role::Tab,
        ] {
            assert_eq!(
                route(role, &SemAction::Click, 7).expect("pressable"),
                WidgetOp::Press(7),
                "{role:?}"
            );
        }
    }

    #[test]
    fn check_uncheck_and_doubleclick_press() {
        assert_eq!(
            route(Role::CheckBox, &SemAction::Check, 3).expect("check"),
            WidgetOp::Press(3)
        );
        assert_eq!(
            route(Role::CheckBox, &SemAction::Uncheck, 3).expect("uncheck"),
            WidgetOp::Press(3)
        );
        assert_eq!(
            route(Role::Button, &SemAction::DoubleClick, 3).expect("dblclick"),
            WidgetOp::Press(3)
        );
    }

    #[test]
    fn editable_roles_set_content() {
        for role in [Role::TextInput, Role::SearchInput, Role::PasswordInput] {
            assert_eq!(
                route(role, &SemAction::SetValue { text: "hi".into() }, 9).expect("editable"),
                WidgetOp::SetContent(9, "hi".into()),
                "{role:?}"
            );
        }
    }

    #[test]
    fn set_value_on_a_button_is_rejected() {
        let err = route(Role::Button, &SemAction::SetValue { text: "hi".into() }, 9)
            .expect_err("not editable");
        assert!(err.to_string().contains("non-editable"), "{err}");
    }

    #[test]
    fn focus_blur_route_to_widget_ops() {
        assert_eq!(
            route(Role::Button, &SemAction::Focus, 5).expect("focus"),
            WidgetOp::Focus(5)
        );
        assert_eq!(
            route(Role::Button, &SemAction::Blur, 5).expect("blur"),
            WidgetOp::Blur(5)
        );
        assert_eq!(
            route(Role::Label, &SemAction::Focus, 5).expect("focusable label"),
            WidgetOp::Focus(5)
        );
    }

    #[test]
    fn hover_and_scroll_are_loud_errors_here() {
        let err = route(Role::Button, &SemAction::Hover, 1).expect_err("hover");
        assert!(err.to_string().contains("input pipeline"), "{err}");
        let err =
            route(Role::Button, &SemAction::Scroll { dx: 0.0, dy: 8.0 }, 1).expect_err("scroll");
        assert!(err.to_string().contains("input pipeline"), "{err}");
    }

    #[test]
    fn keyboard_press_is_rejected_at_adapter_level() {
        let err = route(
            Role::TextInput,
            &SemAction::Press {
                key: "Enter".into(),
                modifiers: vec![],
            },
            2,
        )
        .expect_err("press");
        assert!(err.to_string().contains("set_value"), "{err}");
    }
}
