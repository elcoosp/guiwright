//! The Playwright-shaped selector model (architecture §8.1).
//!
//! Text matching follows Playwright semantics: a plain string is a
//! *substring*, *case-insensitive*, *whitespace-normalized* match unless
//! [`TextMatch::exact`] is set, in which case the match is exact and
//! case-sensitive (Playwright's `exact: true`).

use std::fmt::Write;

use accesskit::Role;

/// A text matching rule (Playwright `TextMatch` analog).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMatch {
    pub pattern: TextPattern,
    pub exact: bool,
    pub ignore_case: bool,
}

impl TextMatch {
    /// Playwright default: substring match, case-insensitive,
    /// whitespace-normalized.
    pub fn new(pattern: impl Into<TextPattern>) -> Self {
        Self {
            pattern: pattern.into(),
            exact: false,
            ignore_case: true,
        }
    }

    /// Exact, case-sensitive match (`exact: true`).
    pub fn exact(pattern: impl Into<TextPattern>) -> Self {
        Self {
            pattern: pattern.into(),
            exact: true,
            ignore_case: false,
        }
    }

    pub fn exact_ignoring_case(pattern: impl Into<TextPattern>) -> Self {
        Self {
            pattern: pattern.into(),
            exact: true,
            ignore_case: true,
        }
    }

    /// Case-sensitive substring match.
    pub fn contains_sensitive(pattern: impl Into<TextPattern>) -> Self {
        Self {
            pattern: pattern.into(),
            exact: false,
            ignore_case: false,
        }
    }

    /// Test a candidate string against this rule. Both sides are
    /// whitespace-normalized (Playwright normalizes both).
    pub fn matches(&self, candidate: &str) -> bool {
        let candidate = normalize(candidate);
        let needle = normalize(self.pattern.as_str());
        if self.ignore_case {
            let (candidate, needle) = (candidate.to_lowercase(), needle.to_lowercase());
            if self.exact {
                candidate == needle
            } else {
                candidate.contains(&needle)
            }
        } else if self.exact {
            candidate == needle
        } else {
            candidate.contains(&needle)
        }
    }
}

/// Collapse runs of whitespace to single spaces and trim the ends
/// (Playwright's text normalization).
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_ws {
                out.push(' ');
            }
            last_ws = true;
        } else {
            out.push(ch);
            last_ws = false;
        }
    }
    out.trim().to_string()
}

/// The text itself. Regex support lands with the codegen milestone
/// (architecture keeps the enum open).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPattern {
    Str(String),
}

impl TextPattern {
    pub fn as_str(&self) -> &str {
        match self {
            TextPattern::Str(s) => s,
        }
    }
}

impl From<&str> for TextPattern {
    fn from(s: &str) -> Self {
        TextPattern::Str(s.to_string())
    }
}

impl From<String> for TextPattern {
    fn from(s: String) -> Self {
        TextPattern::Str(s)
    }
}

impl From<&String> for TextPattern {
    fn from(s: &String) -> Self {
        TextPattern::Str(s.clone())
    }
}

impl From<&str> for TextMatch {
    fn from(s: &str) -> Self {
        TextMatch::new(s)
    }
}

impl From<String> for TextMatch {
    fn from(s: String) -> Self {
        TextMatch::new(s)
    }
}

impl From<&String> for TextMatch {
    fn from(s: &String) -> Self {
        TextMatch::new(s.as_str())
    }
}

/// Narrowing applied to the result set of the source selector
/// (`Locator.filter` / `first` / `nth` / `last`, §8.1 `Selector::Filter`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nth {
    First,
    Last,
    Index(usize),
}

/// Selector model (§8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// `getByRole(role, name?)` — name matches the accessible name
    /// (`labelled_by` aware, via the consumer's label resolution).
    Role { role: Role, name: Option<TextMatch> },
    /// `getByLabel(text)` — matches the node's accessible label.
    Label(TextMatch),
    /// `getByText(text)` — matches the node's label or value text.
    Text(TextMatch),
    /// `getByTestId(id)` — resolves through the test-id registry
    /// (`a11y_id` / `widget::Id` / `test_id` / `debug_selector`, §8.1).
    TestId(String),
    /// `getByValue(text)` — matches the node's value.
    Value(TextMatch),
    /// Chained descendant: resolve `chain[0]`, then search its subtree for
    /// `chain[1]`, and so on.
    All(Vec<Selector>),
    /// Narrow a source selector's result set.
    Filter {
        source: Box<Selector>,
        has_text: Option<TextMatch>,
        has_not: Option<Box<Selector>>,
        nth: Option<Nth>,
    },
}

impl Selector {
    /// `getByRole` without a name constraint.
    pub fn role(role: Role) -> Selector {
        Selector::Role { role, name: None }
    }

    /// `getByRole(role, name)`.
    pub fn role_named(role: Role, name: impl Into<TextMatch>) -> Selector {
        Selector::Role {
            role,
            name: Some(name.into()),
        }
    }

    pub fn label(name: impl Into<TextMatch>) -> Selector {
        Selector::Label(name.into())
    }

    pub fn text(text: impl Into<TextMatch>) -> Selector {
        Selector::Text(text.into())
    }

    pub fn test_id(id: impl Into<String>) -> Selector {
        Selector::TestId(id.into())
    }

    pub fn value(value: impl Into<TextMatch>) -> Selector {
        Selector::Value(value.into())
    }

    /// `locator.first()` sugar → [`Selector::Filter`] with [`Nth::First`].
    pub fn first(self) -> Selector {
        Selector::Filter {
            source: Box::new(self),
            has_text: None,
            has_not: None,
            nth: Some(Nth::First),
        }
    }

    /// `locator.last()` sugar.
    pub fn last(self) -> Selector {
        Selector::Filter {
            source: Box::new(self),
            has_text: None,
            has_not: None,
            nth: Some(Nth::Last),
        }
    }

    /// `locator.nth(i)` sugar.
    pub fn nth(self, index: usize) -> Selector {
        Selector::Filter {
            source: Box::new(self),
            has_text: None,
            has_not: None,
            nth: Some(Nth::Index(index)),
        }
    }

    /// `locator.filter(has_text=…)` sugar.
    pub fn has_text(self, text: impl Into<TextMatch>) -> Selector {
        Selector::Filter {
            source: Box::new(self),
            has_text: Some(text.into()),
            has_not: None,
            nth: None,
        }
    }

    /// `locator.filter(has_not=locator)` sugar.
    pub fn has_not(self, inner: Selector) -> Selector {
        Selector::Filter {
            source: Box::new(self),
            has_text: None,
            has_not: Some(Box::new(inner)),
            nth: None,
        }
    }

    /// Chained descendant locator sugar (`locator(locator)`).
    pub fn descendant_of(self, outer: Selector) -> Selector {
        let mut chain = match outer {
            Selector::All(chain) => chain,
            other => vec![other],
        };
        chain.push(self);
        Selector::All(chain)
    }

    /// Stable, pretty rendering used in errors and traces.
    pub fn render(&self) -> String {
        match self {
            Selector::Role { role, name: None } => format!("getByRole({role:?})"),
            Selector::Role {
                role,
                name: Some(name),
            } => format!("getByRole({role:?}, {})", render_match(name)),
            Selector::Label(m) => format!("getByLabel({})", render_match(m)),
            Selector::Text(m) => format!("getByText({})", render_match(m)),
            Selector::TestId(id) => format!("getByTestId({id:?})"),
            Selector::Value(m) => format!("getByValue({})", render_match(m)),
            Selector::All(chain) => {
                let parts: Vec<String> = chain.iter().map(Selector::render).collect();
                parts.join(" >> ")
            }
            Selector::Filter {
                source,
                has_text,
                has_not,
                nth,
            } => {
                let mut s = source.render();
                if let Some(t) = has_text {
                    let _ = write!(s, ".filter(hasText={})", render_match(t));
                }
                if let Some(sel) = has_not {
                    let _ = write!(s, ".filter(hasNot={})", sel.render());
                }
                match nth {
                    Some(Nth::First) => s.push_str(".first()"),
                    Some(Nth::Last) => s.push_str(".last()"),
                    Some(Nth::Index(i)) => {
                        let _ = write!(s, ".nth({i})");
                    }
                    None => {}
                }
                s
            }
        }
    }
}

fn render_match(m: &TextMatch) -> String {
    let inner = m.pattern.as_str();
    if m.exact && !m.ignore_case {
        format!("{inner:?}")
    } else {
        format!("{inner:?} (fuzzy)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playwrigh_default_is_substring_case_insensitive() {
        let m = TextMatch::new("Val");
        assert!(m.matches("Valider"));
        assert!(m.matches("  valide  "));
        assert!(m.matches("VALIDATION"));
        assert!(!m.matches("Annuler"));
    }

    #[test]
    fn exact_match_is_case_sensitive() {
        let m = TextMatch::exact("Save");
        assert!(m.matches("Save"));
        assert!(!m.matches("save"));
        assert!(!m.matches("Save all"));
    }

    #[test]
    fn whitespace_is_normalized_on_both_sides() {
        let m = TextMatch::exact("Hello   world");
        assert!(m.matches("Hello world"));
        let m = TextMatch::new("hello  world");
        assert!(m.matches("Hello\tWorld"));
    }

    #[test]
    fn render_is_stable_and_readable() {
        let s = Selector::role_named(accesskit::Role::Button, "Save").first();
        assert_eq!(s.render(), "getByRole(Button, \"Save\" (fuzzy)).first()");
    }

    proptest::proptest! {
        #[test]
        fn normalize_never_grows_or_leaves_ws(s in ".*") {
            let n = normalize(&s);
            proptest::prop_assert!(n.len() <= s.len());
            proptest::prop_assert_eq!(n.trim().to_string(), n);
        }
    }
}
