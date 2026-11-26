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

    /// The regex probe (the m34 record's second standing note, cashed —
    /// the m93/m94 honest lists' first item): a JS-convention regex
    /// `test` — partial by default, **case-sensitive** unless the
    /// pattern carries its own inline flags (`(?i)save`), the
    /// [`TextMatch::exact`] flag anchoring it (`^…$`). The
    /// `ignore_case` field never applies to a regex — the pattern speaks
    /// its own case rule (the wire regex rows carry the pattern only).
    pub fn regex(pattern: impl Into<String>) -> Self {
        Self {
            pattern: TextPattern::Regex(pattern.into()),
            exact: false,
            ignore_case: false,
        }
    }

    /// The regex probe with [`TextMatch::exact`] set — the pattern is
    /// wrapped `^(?:…)$` so a full-match rule composes without editing
    /// the source (Playwright's `exact: true` reading at regex width).
    pub fn regex_exact(pattern: impl Into<String>) -> Self {
        Self {
            pattern: TextPattern::Regex(pattern.into()),
            exact: true,
            ignore_case: false,
        }
    }

    /// Test a candidate string against this rule. Both sides are
    /// whitespace-normalized (Playwright normalizes both).
    pub fn matches(&self, candidate: &str) -> bool {
        let candidate = normalize(candidate);
        match &self.pattern {
            TextPattern::Str(needle) => {
                let needle = normalize(needle);
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
            // The regex probe (the m34 standing note, cashed): the
            // candidate is normalized, the pattern is not (it is a
            // pattern, not literal text). An uncompilable source answers
            // the honest `false` — the callers validate at their own
            // boundary ([`TextMatch::try_regex`]), the resolver never
            // panics (the fuzz rule).
            TextPattern::Regex(source) => {
                if self.exact {
                    // The anchored form wraps the pattern (`^…$`), never
                    // the candidate — a full-match rule composed from a
                    // partial source.
                    match regex::Regex::new(&format!("^(?:{source})$")) {
                        Ok(re) => re.is_match(&candidate),
                        Err(_) => false,
                    }
                } else {
                    match regex::Regex::new(source) {
                        Ok(re) => re.is_match(&candidate),
                        Err(_) => false,
                    }
                }
            }
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

/// The text itself. The regex arm is the m34 record's second standing
/// note cashed (the m93/m94 honest lists' first item — *"the enum stays
/// open until the codegen milestone asks"*; the codegen milestones
/// shipped without asking, and the prompt-side gap the m93/m94 records
/// named — *"the REPL's `has_text` word speaks the fuzzy `TextMatch`
/// default only"* — is the consumer that arrived): the arm carries the
/// pattern **source** string only, compiled per match (the
/// [`TextMatch::matches`] arm owns the semantics; the resolve path's
/// string probes keep their §15.1 budget rows — a regex probe rides the
/// same resolve conversations, no row of its own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPattern {
    Str(String),
    Regex(String),
}

impl TextPattern {
    pub fn as_str(&self) -> &str {
        // Both arms carry the pattern source — the string probe's needle
        // and the regex probe's source share the accessor (the
        // `TextMatchV1` bridge reads it without knowing the arm).
        match self {
            TextPattern::Str(s) | TextPattern::Regex(s) => s,
        }
    }

    /// The regex arm's constructor.
    pub fn regex(source: impl Into<String>) -> Self {
        TextPattern::Regex(source.into())
    }

    /// The typed check the boundary callers compose ([`TextMatch::try_regex`],
    /// the REPL's `/…/` literal parse): is this source a regex the crate
    /// compiles? The error text is the regex crate's own — the
    /// failures-name-names rule.
    pub fn validate(source: &str) -> Result<(), String> {
        regex::Regex::new(source)
            .map(|_| ())
            .map_err(|e| e.to_string())
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

impl From<TextPattern> for TextMatch {
    fn from(pattern: TextPattern) -> Self {
        TextMatch::new(pattern)
    }
}

impl TextMatch {
    /// The validated regex probe ([`TextMatch::regex`] with the
    /// boundary check): an uncompilable source is the typed error, the
    /// message the regex crate itself printed — the REPL's `/…/` parse
    /// and the kit's programmatic builders compose it so a bad pattern
    /// never reaches the resolver silently.
    pub fn try_regex(pattern: impl Into<String>) -> Result<Self, String> {
        let source = pattern.into();
        TextPattern::validate(&source)?;
        Ok(TextMatch::regex(source))
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
    /// The selector-valued **positive** filter (the m34 record's named
    /// follow-on — Playwright's `filter({ has })`): keep the source's
    /// matches whose subtree (the node included) matches `inner`.
    /// Distinct from the `All(chain)` descendant chain, which narrows to
    /// the *descendant's* rows — `has` keeps the *ancestor* rows.
    Has {
        source: Box<Selector>,
        has: Box<Selector>,
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

    /// `locator.filter({ has })` sugar (the m34 record's named
    /// follow-on) — the selector-valued **positive** filter: keep only
    /// the matches whose subtree (the node included) matches `inner`,
    /// the exact inverse of [`Self::has_not`] (both read the same
    /// subtree walk with opposite verdicts). The inner selector is a
    /// full [`Selector`]: a role+name constraint, a test id, a nested
    /// chain — a text probe only when built as one. Distinct from
    /// [`Self::then_descendant`], which narrows to the descendant's
    /// rows; `has` keeps the ancestor's.
    pub fn has(self, inner: Selector) -> Selector {
        Selector::Has {
            source: Box::new(self),
            has: Box::new(inner),
        }
    }

    /// `locator.locator(child)` sugar (M34) — the chained-descendant
    /// narrowing in **composition order**: resolve `self`, then search
    /// each match's subtree for `inner` (§8.1 `Selector::All` chain,
    /// Playwright's `locator.locator()`).
    pub fn then_descendant(self, inner: Selector) -> Selector {
        let mut chain = match self {
            Selector::All(chain) => chain,
            other => vec![other],
        };
        chain.push(inner);
        Selector::All(chain)
    }

    /// Chained descendant locator sugar (`locator(locator)`): `self`
    /// resolves inside `outer`'s subtree — [`Self::then_descendant`] with
    /// the arguments flipped to match the "is a descendant of" reading.
    pub fn descendant_of(self, outer: Selector) -> Selector {
        outer.then_descendant(self)
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
            Selector::Has { source, has } => {
                let mut s = source.render();
                let _ = write!(s, ".filter(has={})", has.render());
                s
            }
        }
    }
}

fn render_match(m: &TextMatch) -> String {
    match &m.pattern {
        // The regex probe renders the JS literal shape — what you built
        // is what the error names (the failures-name-names rule).
        TextPattern::Regex(source) => format!("/{source}/"),
        TextPattern::Str(inner) => {
            if m.exact && !m.ignore_case {
                format!("{inner:?}")
            } else {
                format!("{inner:?} (fuzzy)")
            }
        }
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
    fn regex_probe_is_partial_and_case_sensitive_by_default() {
        let m = TextMatch::regex("Save");
        assert!(m.matches("Save"));
        assert!(
            m.matches("Save all"),
            "partial — a regex `test`, not a full match"
        );
        assert!(
            !m.matches("save"),
            "case-sensitive until the pattern says (?i)"
        );
        let m = TextMatch::regex("(?i)^save all$");
        assert!(
            m.matches("  SAVE   ALL  "),
            "inline flags + normalized candidate"
        );
    }

    #[test]
    fn regex_exact_anchors_the_pattern() {
        let m = TextMatch::regex_exact("Save");
        assert!(m.matches("Save"));
        assert!(
            !m.matches("Save all"),
            "anchored — the wrapped form full-matches"
        );
    }

    #[test]
    fn an_uncompilable_pattern_matches_nothing() {
        let m = TextMatch::regex("(unclosed[");
        assert!(!m.matches("anything"), "the honest false, never a panic");
        let err = TextMatch::try_regex("(unclosed[").unwrap_err();
        assert!(err.contains("unclosed character class"), "{err}");
        assert!(TextMatch::try_regex("^Save$").is_ok());
    }

    #[test]
    fn regex_render_is_the_literal_shape() {
        let s = Selector::text(TextMatch::regex("(?i)save|cancel"));
        assert_eq!(s.render(), "getByText(/(?i)save|cancel/)");
        let s = Selector::role_named(Role::Button, "Save").first();
        assert_eq!(s.render(), "getByRole(Button, \"Save\" (fuzzy)).first()");
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
