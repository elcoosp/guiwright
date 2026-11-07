//! **Contribution 1b (§4.7)** — role metadata on floem core views.
//!
//! Floem has no accessibility layer, so the logical tree needs role
//! semantics from somewhere (§4.7 "role metadata on core views
//! (button/text-input/checkbox) so the logical tree can carry
//! AccessKit-compatible roles even without AccessKit"). The registry is
//! data-driven: apps extend it at startup without touching the adapter,
//! exactly like the iced widget-kind registry (§4.6).

use std::collections::HashMap;

use accesskit::Role;

/// The default kind table — the floem core view vocabulary a test app
/// exercises (§4.7; unknown kinds resolve to [`Role::Unknown`]).
pub fn defaults() -> HashMap<&'static str, Role> {
    HashMap::from([
        ("app", Role::Application),
        ("window", Role::Window),
        ("container", Role::Group),
        ("stack", Role::Group),
        ("vstack", Role::Group),
        ("hstack", Role::Group),
        ("grid", Role::Group),
        ("button", Role::Button),
        ("label", Role::Label),
        ("text", Role::Label),
        ("rich_text", Role::Label),
        ("checkbox", Role::CheckBox),
        ("radio", Role::RadioButton),
        ("toggle_button", Role::Switch),
        ("text_input", Role::TextInput),
        ("textarea", Role::MultilineTextInput),
        ("password", Role::PasswordInput),
        ("slider", Role::Slider),
        ("progress", Role::ProgressIndicator),
        ("scroll", Role::ScrollView),
        ("image", Role::Image),
        ("svg", Role::Image),
        ("dropdown", Role::ComboBox),
        ("dropdown_select", Role::ComboBox),
        ("tabs", Role::TabList),
        ("tooltip", Role::Tooltip),
    ])
}

/// Floem view-kind → AccessKit [`Role`] registry (§4.7 contribution 1b).
#[derive(Clone, Debug)]
pub struct RoleRegistry {
    kinds: HashMap<&'static str, Role>,
}

impl RoleRegistry {
    pub fn new() -> Self {
        Self { kinds: defaults() }
    }

    /// Contribute an app-defined view kind. Registered kinds override
    /// defaults.
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
    fn defaults_cover_the_floem_core_view_set() {
        let reg = RoleRegistry::new();
        assert_eq!(reg.role_of("button"), Role::Button);
        assert_eq!(reg.role_of("text_input"), Role::TextInput);
        assert_eq!(reg.role_of("checkbox"), Role::CheckBox);
        assert_eq!(reg.role_of("label"), Role::Label);
        assert_eq!(reg.role_of("vstack"), Role::Group);
        assert_eq!(reg.role_of("slider"), Role::Slider);
        assert_eq!(reg.role_of("dropdown_select"), Role::ComboBox);
    }

    #[test]
    fn unknown_kinds_resolve_to_unknown() {
        let reg = RoleRegistry::new();
        assert_eq!(reg.role_of("custom_view"), Role::Unknown);
    }

    #[test]
    fn apps_contribute_kinds_and_override_defaults() {
        let mut reg = RoleRegistry::new();
        reg.register("kanban_card", Role::Article);
        assert_eq!(reg.role_of("kanban_card"), Role::Article);
        reg.register("button", Role::MenuItem);
        assert_eq!(reg.role_of("button"), Role::MenuItem, "registered wins");
    }
}
