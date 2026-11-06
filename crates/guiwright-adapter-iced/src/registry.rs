//! Widget-kind → AccessKit [`Role`] registry (§4.6 "adds role semantics
//! via a contributed widget-kind registry (button/text-input/checkbox map
//! to `Tag`/widget type introspection)").
//!
//! The registry is data-driven so apps extend it at startup
//! ([`RoleRegistry::register`]) without touching the adapter — custom
//! widget kinds keep first-class semantics for locators.

use std::collections::HashMap;

use accesskit::Role;

/// The default kind table (the iced widget vocabulary a test app
/// exercises, §4.6; unknown kinds resolve to [`Role::Unknown`]).
pub fn defaults() -> HashMap<&'static str, Role> {
    HashMap::from([
        ("application", Role::Application),
        ("window", Role::Window),
        ("container", Role::Group),
        ("pane", Role::Group),
        ("button", Role::Button),
        ("checkbox", Role::CheckBox),
        ("toggle", Role::Switch),
        ("radio", Role::RadioButton),
        ("text", Role::Label),
        ("text_input", Role::TextInput),
        ("multiline", Role::MultilineTextInput),
        ("password", Role::PasswordInput),
        ("search", Role::SearchInput),
        ("slider", Role::Slider),
        ("progress", Role::ProgressIndicator),
        ("scrollable", Role::ScrollView),
        ("image", Role::Image),
        ("pick_list", Role::ComboBox),
        ("combo_box", Role::EditableComboBox),
        ("splitter", Role::Splitter),
    ])
}

/// Widget-kind registry (§4.6).
#[derive(Clone, Debug)]
pub struct RoleRegistry {
    kinds: HashMap<&'static str, Role>,
}

impl RoleRegistry {
    pub fn new() -> Self {
        Self { kinds: defaults() }
    }

    /// Contribute an app-defined kind (§4.6 "role registry contribution").
    /// Registered kinds override defaults.
    pub fn register(&mut self, kind: &'static str, role: Role) {
        self.kinds.insert(kind, role);
    }

    pub fn role_of(&self, kind: &str) -> Role {
        self.kinds.get(kind).copied().unwrap_or(Role::Unknown)
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

impl Default for RoleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_the_iced_widget_set() {
        let reg = RoleRegistry::new();
        assert_eq!(reg.role_of("button"), Role::Button);
        assert_eq!(reg.role_of("text_input"), Role::TextInput);
        assert_eq!(reg.role_of("checkbox"), Role::CheckBox);
        assert_eq!(reg.role_of("text"), Role::Label);
        assert_eq!(reg.role_of("container"), Role::Group);
        assert_eq!(reg.role_of("pick_list"), Role::ComboBox);
    }

    #[test]
    fn unknown_kinds_resolve_to_unknown() {
        let reg = RoleRegistry::new();
        assert_eq!(reg.role_of("fancy_widget"), Role::Unknown);
    }

    #[test]
    fn apps_contribute_kinds_and_override_defaults() {
        let mut reg = RoleRegistry::new();
        reg.register("fancy_widget", Role::Legend);
        assert_eq!(reg.role_of("fancy_widget"), Role::Legend);
        reg.register("button", Role::Switch);
        assert_eq!(reg.role_of("button"), Role::Switch, "registered wins");
    }
}
