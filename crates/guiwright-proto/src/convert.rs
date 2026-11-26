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

/// The probe's wire classification: a string pattern rides the
/// byte-pinned `TextMatchV1` arms, a regex pattern rides the appended
/// regex rows (the M64 rule at selector width — the widening is its own
/// variant, never a payload reshape).
enum Probe<'a> {
    Str,
    Regex(&'a str),
}

fn probe(m: &TextMatch) -> Probe<'_> {
    match &m.pattern {
        TextPattern::Str(_) => Probe::Str,
        TextPattern::Regex(source) => Probe::Regex(source),
    }
}

fn str_match(m: &TextMatch) -> TextMatchV1 {
    // Only called for `Probe::Str` — the string pattern through the
    // byte-pinned struct.
    TextMatchV1 {
        text: m.pattern.as_str().to_string(),
        exact: m.exact,
        ignore_case: m.ignore_case,
    }
}

fn regex_match(pattern: &str) -> TextMatch {
    // The wire regex rows carry the pattern only: the case rule rides
    // the pattern's inline flags, the probe is partial (the JS `test`
    // convention the regex family pins).
    TextMatch::regex(pattern)
}

impl From<Selector> for SelectorV1 {
    fn from(s: Selector) -> Self {
        match s {
            Selector::Role { role, name } => match name {
                Some(m) => match probe(&m) {
                    Probe::Regex(pattern) => SelectorV1::RoleNameRegex {
                        role: format!("{role:?}"),
                        pattern: pattern.to_string(),
                    },
                    Probe::Str => SelectorV1::Role {
                        role: format!("{role:?}"),
                        name: Some(str_match(&m)),
                    },
                },
                None => SelectorV1::Role {
                    role: format!("{role:?}"),
                    name: None,
                },
            },
            Selector::Label(m) => match probe(&m) {
                Probe::Regex(pattern) => SelectorV1::LabelRegex {
                    pattern: pattern.to_string(),
                },
                Probe::Str => SelectorV1::Label(str_match(&m)),
            },
            Selector::Text(m) => match probe(&m) {
                Probe::Regex(pattern) => SelectorV1::TextRegex {
                    pattern: pattern.to_string(),
                },
                Probe::Str => SelectorV1::Text(str_match(&m)),
            },
            Selector::TestId(id) => SelectorV1::TestId(id),
            Selector::Value(m) => match probe(&m) {
                Probe::Regex(pattern) => SelectorV1::ValueRegex {
                    pattern: pattern.to_string(),
                },
                Probe::Str => SelectorV1::Value(str_match(&m)),
            },
            Selector::All(chain) => SelectorV1::All(chain.into_iter().map(Into::into).collect()),
            Selector::Filter {
                source,
                has_text,
                has_not,
                nth,
            } => {
                // The regex `has_text` rides its own row (the M64 rule):
                // it wraps the source as the `HasTextRegex` row, and any
                // `nth`/`has_not` beside it compose that row as the
                // *source* of the string-`Filter` — the fold's own
                // composition, the byte-pinned `Filter` payload never
                // reshaped.
                let regex_has_text = has_text.as_ref().and_then(|m| match probe(m) {
                    Probe::Regex(pattern) => Some(pattern.to_string()),
                    Probe::Str => None,
                });
                let (source, has_text) = if let Some(pattern) = regex_has_text {
                    (
                        Box::new(SelectorV1::HasTextRegex {
                            source: Box::new((*source).into()),
                            pattern,
                        }),
                        None,
                    )
                } else {
                    (Box::new((*source).into()), has_text)
                };
                SelectorV1::Filter {
                    source,
                    has_text: has_text.map(|m| str_match(&m)),
                    has_not: has_not.map(|s| Box::new((*s).into())),
                    nth: nth.map(Into::into),
                }
            }
            Selector::Has { source, has } => SelectorV1::Has {
                source: Box::new((*source).into()),
                has: Box::new((*has).into()),
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
            // The appended regex rows (the M64 rule at selector width):
            // each reconstructs the core probe the regex arm answers —
            // the pattern only, the case rule riding the pattern's
            // inline flags, the probe partial (the JS `test`
            // convention).
            SelectorV1::TextRegex { pattern } => Selector::Text(regex_match(&pattern)),
            SelectorV1::LabelRegex { pattern } => Selector::Label(regex_match(&pattern)),
            SelectorV1::ValueRegex { pattern } => Selector::Value(regex_match(&pattern)),
            SelectorV1::RoleNameRegex { role, pattern } => Selector::Role {
                role: parse_role(&role),
                name: Some(regex_match(&pattern)),
            },
            SelectorV1::HasTextRegex { source, pattern } => Selector::Filter {
                source: Box::new((*source).into()),
                has_text: Some(regex_match(&pattern)),
                has_not: None,
                nth: None,
            },
            SelectorV1::All(chain) => Selector::All(chain.into_iter().map(Into::into).collect()),
            SelectorV1::Filter {
                source,
                has_text,
                has_not,
                nth,
            } => {
                // A `Filter` row with no narrowing word is the fold's
                // identity wrapper (the regex `has_text` composes its
                // `HasTextRegex` row *as this row's source*): decoding
                // collapses it so the core shape the kit built comes
                // back — semantics and render identical either way.
                if has_text.is_none() && has_not.is_none() && nth.is_none() {
                    return (*source).into();
                }
                Selector::Filter {
                    source: Box::new((*source).into()),
                    has_text: has_text.map(Into::into),
                    has_not: has_not.map(|s| Box::new((*s).into())),
                    nth: nth.map(Into::into),
                }
            }
            SelectorV1::Has { source, has } => Selector::Has {
                source: Box::new((*source).into()),
                has: Box::new((*has).into()),
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
        // The m89 select vocabulary: the choice widget's role rides the
        // wire beside the appended `SelectOption` action variant.
        "ComboBox" => Role::ComboBox,
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
            Selector::role(Role::Group).has(Selector::role(Role::Button)),
            Selector::role_named(Role::Button, "Save").descendant_of(Selector::role(Role::Window)),
            Selector::All(vec![
                Selector::role(Role::Group),
                Selector::role(Role::Button),
            ]),
            // The appended regex rows (the M64 rule at selector width):
            // each probe shape roundtrips through its own variant, the
            // byte-pinned string rows untouched.
            Selector::text(TextMatch::regex("(?i)save|cancel")),
            Selector::label(TextMatch::regex("^Wel.* Ada$")),
            Selector::value(TextMatch::regex(r"^\S+@\S+\.\S+$")),
            Selector::role_named(Role::Button, TextMatch::regex("^(Save|Cancel)$")),
            Selector::role(Role::Button).has_text(TextMatch::regex("^Save$")),
            Selector::role(Role::Button)
                .has_text(TextMatch::regex("^Save$"))
                .first(),
            Selector::role(Role::Group)
                .then_descendant(Selector::role_named(Role::Label, TextMatch::regex("Ada"))),
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
