//! Conversions between wire types (V1) and [`guiwright_core`] types.
//!
//! The wire schema is deliberately decoupled from core types so it can be
//! versioned independently; these impls are the (total) bridge.

use accesskit::Role;
use guiwright_core::selector::{Nth, Selector, TextMatch, TextPattern};

use crate::schema::{NthV1, SelectorV1, TextMatchV1};

impl From<TextMatch> for TextMatchV1 {
    fn from(m: TextMatch) -> Self {
        TextMatchV1 {
            text: m.pattern.as_str().to_string(),
            exact: m.exact,
            ignore_case: m.ignore_case,
        }
    }
}

impl From<TextMatchV1> for TextMatch {
    fn from(v: TextMatchV1) -> Self {
        TextMatch {
            pattern: TextPattern::Str(v.text),
            exact: v.exact,
            ignore_case: v.ignore_case,
        }
    }
}

impl From<Selector> for SelectorV1 {
    fn from(s: Selector) -> Self {
        match s {
            Selector::Role { role, name } => SelectorV1::Role {
                role: format!("{role:?}"),
                name: name.map(Into::into),
            },
            Selector::Label(m) => SelectorV1::Label(m.into()),
            Selector::Text(m) => SelectorV1::Text(m.into()),
            Selector::TestId(id) => SelectorV1::TestId(id),
            Selector::Value(m) => SelectorV1::Value(m.into()),
            Selector::All(chain) => SelectorV1::All(chain.into_iter().map(Into::into).collect()),
            Selector::Filter {
                source,
                has_text,
                has_not,
                nth,
            } => SelectorV1::Filter {
                source: Box::new((*source).into()),
                has_text: has_text.map(Into::into),
                has_not: has_not.map(|s| Box::new((*s).into())),
                nth: nth.map(Into::into),
            },
        }
    }
}

impl From<SelectorV1> for Selector {
    fn from(v: SelectorV1) -> Self {
        match v {
            SelectorV1::Role { role, name } => Selector::Role {
                role: parse_role(&role),
                name: name.map(Into::into),
            },
            SelectorV1::Label(m) => Selector::Label(m.into()),
            SelectorV1::Text(m) => Selector::Text(m.into()),
            SelectorV1::TestId(id) => Selector::TestId(id),
            SelectorV1::Value(m) => Selector::Value(m.into()),
            SelectorV1::All(chain) => Selector::All(chain.into_iter().map(Into::into).collect()),
            SelectorV1::Filter {
                source,
                has_text,
                has_not,
                nth,
            } => Selector::Filter {
                source: Box::new((*source).into()),
                has_text: has_text.map(Into::into),
                has_not: has_not.map(|s| Box::new((*s).into())),
                nth: nth.map(Into::into),
            },
        }
    }
}

impl From<Nth> for NthV1 {
    fn from(n: Nth) -> Self {
        match n {
            Nth::First => NthV1::First,
            Nth::Last => NthV1::Last,
            Nth::Index(i) => NthV1::Index(i),
        }
    }
}

impl From<NthV1> for Nth {
    fn from(v: NthV1) -> Self {
        match v {
            NthV1::First => Nth::First,
            NthV1::Last => Nth::Last,
            NthV1::Index(i) => Nth::Index(i),
        }
    }
}

/// Role names on the wire are the `Debug` names (`Button`, `TextInput`, …).
fn parse_role(name: &str) -> Role {
    // Roles are a closed set in AccessKit; unknown names degrade to
    // `Unknown` rather than erroring, so clients can request roles from a
    // newer toolkit without breaking older drivers.
    match name {
        "Window" => Role::Window,
        "Group" => Role::Group,
        "GenericContainer" => Role::GenericContainer,
        "Button" => Role::Button,
        "DefaultButton" => Role::DefaultButton,
        "Label" => Role::Label,
        "TextRun" => Role::TextRun,
        "TextInput" => Role::TextInput,
        "MultilineTextInput" => Role::MultilineTextInput,
        "SearchInput" => Role::SearchInput,
        "EmailInput" => Role::EmailInput,
        "NumberInput" => Role::NumberInput,
        "CheckBox" => Role::CheckBox,
        "Switch" => Role::Switch,
        "RadioButton" => Role::RadioButton,
        "List" => Role::List,
        "ListItem" => Role::ListItem,
        "ListBoxOption" => Role::ListBoxOption,
        "Menu" => Role::Menu,
        "MenuItem" => Role::MenuItem,
        "MenuListOption" => Role::MenuListOption,
        "Dialog" => Role::Dialog,
        "Pane" => Role::Pane,
        "Image" => Role::Image,
        "Link" => Role::Link,
        "Table" => Role::Table,
        "Row" => Role::Row,
        "Cell" => Role::Cell,
        "TreeItem" => Role::TreeItem,
        "Paragraph" => Role::Paragraph,
        _ => Role::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::Role;

    #[test]
    fn all_selector_shapes_roundtrip() {
        let selectors = vec![
            Selector::role(Role::Button),
            Selector::role_named(Role::Button, "Save"),
            Selector::label("Email"),
            Selector::text("Welcome, Ada"),
            Selector::test_id("save-btn"),
            Selector::value("ada@example.dev"),
            Selector::role(Role::Button).first(),
            Selector::role(Role::Button).last(),
            Selector::role(Role::Button).nth(2),
            Selector::role(Role::Window).has_text("Ada"),
            Selector::role(Role::Group).has_not(Selector::role(Role::TextInput)),
            Selector::role_named(Role::Button, "Save").descendant_of(Selector::role(Role::Window)),
            Selector::All(vec![
                Selector::role(Role::Group),
                Selector::role(Role::Button),
            ]),
        ];
        for s in selectors {
            let wire: SelectorV1 = s.clone().into();
            let bytes = postcard::to_allocvec(&wire).unwrap();
            let decoded: SelectorV1 = postcard::from_bytes(&bytes).unwrap();
            let back: Selector = decoded.into();
            assert_eq!(s, back, "roundtrip of {}", s.render());
        }
    }

    #[test]
    fn unknown_role_degrades_gracefully() {
        let wire = SelectorV1::Role {
            role: "FutureRole".into(),
            name: None,
        };
        let core: Selector = wire.into();
        match core {
            Selector::Role { role, .. } => assert_eq!(role, Role::Unknown),
            other => panic!("unexpected {other:?}"),
        }
    }
}
