//! The client-side content assertions (m54 — the kit's `expect`
//! vocabulary's content half over any door): Playwright's
//! `expect(locator).toHaveText(...)` / `toHaveValue(...)` for remote
//! suites. The vocabulary and the row-set evaluation live here — a pure,
//! transport-blind core — while the wire composition (subscribe →
//! describe-all re-reads → wake on the §7 event channel) rides
//! [`crate::client::ClientConn::expect_text`] /
//! [`crate::client::ClientConn::expect_value`] as default methods, so
//! every shipped client inherits them.
//!
//! Three boundaries keep this honest:
//!
//! * **The kinds are a client-side vocabulary** (never serialized): the
//!   content and identity kinds (m54/m62) compose the existing
//!   `DescribeAll` and event domains byte-for-byte as M12/M39 blessed
//!   them. A driver that predates M39 rejects `DescribeAll` with the
//!   typed `Unsupported` and the assertion surfaces that rejection
//!   rather than degrading (the m41 fail-fast rule; the wait module's
//!   boundary, restated for content). The description kind (m64) is the
//!   one exception — its fact rides the appended
//!   `DescribeAllIdentity`/`DescribedAllIdentity` domain (the m62/m63
//!   records' reserved wire change), the same wake loop over the wider
//!   rows; the M39 encodings stay byte-pinned.
//! * **The rows are the truth.** Evaluation sees exactly what the M39
//!   `describe_all` walk answers, through the m46 derivation orders:
//!   the text kind reads the m29 order (accessible name first, value
//!   fallback) and the value kind reads the m31 order (value first,
//!   name fallback) — the two reads can never silently agree on the
//!   wrong datum. The assertion pins the set's first row (document
//!   order — the same pin the m42 wait and the m47 resolve speak);
//!   later rows never answer (the strict-mode set head).
//! * **Failures name names** (§8.3, the kit's discipline): the typed
//!   timeout carries the locator render, the budget and the last
//!   evaluation — expected and found — never a bare "timed out". The
//!   core `has text`/`value` reason wording rides along (`text is …,
//!   expected …`).

use crate::bulk::{text_content, value_content};
use crate::schema::{DescribedIdentityV1, DescribedV1, TextMatchV1};
use guiwright_core::selector::TextMatch;

/// The content-assertion vocabulary (§8 check vocabulary's content half,
/// the kit's `to_have_text` / `to_have_value` rows) — a client-side
/// vocabulary, never serialized: the assertion composes the existing
/// `DescribeAll` + event domains byte-for-byte (the m42 `WaitState`
/// rule).
#[derive(Clone, Debug, PartialEq)]
pub enum ExpectKind {
    /// The accessible name first, the value as the fallback — the m29
    /// source order the text locators match. The fixture's Email input
    /// answers `"Email"` here, never its value.
    Text(TextMatchV1),
    /// The value first, the accessible name as the fallback — the m31
    /// source order, a form dump wants what the form *carries*. The same
    /// Email input answers `"ada@example.dev"` here.
    Value(TextMatchV1),
    /// The head row's role, exact — the m62 identity row (the kit's
    /// `to_have_role` over any door, M38): the AccessKit debug name the
    /// describe rows print (`Button`, `TextInput`), the vocabulary the
    /// role locators speak. The role is a structural fact, so the
    /// comparison is exact — no matcher, no fuzz.
    Role(String),
    /// The label-only accessible name — the m62 identity row (the kit's
    /// `to_have_accessible_name` over any door, M38): narrower than the
    /// text kind's m29 order (the value never answers), the fact the
    /// text row cannot pin.
    Name(TextMatchV1),
    /// The accessible description, description-only — the m64 identity
    /// row (the kit's `to_have_accessible_description` over any door,
    /// M38): the third identity fact, the one the M39 describe-all rows
    /// left out. Narrower than every content kind — the name and the
    /// value never answer — and read off the m64 identity rows (the
    /// [`DescribedIdentityV1`] fact), so this kind evaluates through
    /// [`ExpectTracker::evaluate_identity`] only; the plain
    /// [`ExpectTracker::evaluate`] answers it with the misroute pending
    /// (the M39 rows carry no description fact to read).
    Description(TextMatchV1),
}

impl ExpectKind {
    /// The static state word the typed timeout names (the kit's
    /// "stuck on …" convention carried over the wire).
    pub fn state(&self) -> &'static str {
        match self {
            ExpectKind::Text(_) => "have text",
            ExpectKind::Value(_) => "have value",
            ExpectKind::Role(_) => "have role",
            ExpectKind::Name(_) => "have name",
            ExpectKind::Description(_) => "have description",
        }
    }
}

/// One evaluation step's answer: either the assertion holds — carrying
/// the row that satisfied it (a directly usable `RequestV1::Action`
/// handle, the dump-row rule, M11) — or the reason it is still pending,
/// which becomes the timeout error's "last evaluation" line.
#[derive(Clone, Debug)]
pub enum ExpectEval {
    /// The content matched. The row is the set's head at the generation
    /// the walk answered.
    Satisfied(DescribedV1),
    /// The world may still change so the assertion can pass; the reason
    /// names what the last evaluation found.
    Pending { reason: String },
}

/// The identity-row evaluation behind
/// [`crate::client::ClientConn::expect_description`] — the same two
/// answers as [`ExpectEval`], the satisfying answer carrying the full
/// [`DescribedIdentityV1`] row (the m64 row is a dump-row handle with
/// the description fact beside it, the dump-row rule).
#[derive(Clone, Debug)]
pub enum ExpectIdentityEval {
    /// The description (or the projected content/identity fact) matched.
    /// The row is the set's head at the generation the walk answered.
    Satisfied(DescribedIdentityV1),
    /// The world may still change so the assertion can pass; the reason
    /// names what the last evaluation found.
    Pending { reason: String },
}

/// The row-set evaluator behind [`crate::client::ClientConn::expect_text`]
/// and [`crate::client::ClientConn::expect_value`]. Feed it fresh
/// describe-all rows after every wake and it answers the assertion; the
/// evaluator carries no cross-wake state (content has no `Stable`-like
/// frame discipline — every wake re-decides the whole read).
#[derive(Clone, Debug)]
pub struct ExpectTracker {
    kind: ExpectKind,
}

impl ExpectTracker {
    /// A tracker for one assertion, in the kind it asserts.
    pub fn new(kind: ExpectKind) -> Self {
        Self { kind }
    }

    /// Evaluate one row set: the set's head answers by the kind's read
    /// (the m54 rule — document order, first row); an empty set stays
    /// pending (waitable — nodes stream in, Playwright parity).
    pub fn evaluate(&self, rows: &[DescribedV1]) -> ExpectEval {
        let Some(row) = rows.first() else {
            return ExpectEval::Pending {
                reason: "no match yet (the set is empty)".into(),
            };
        };
        match &self.kind {
            // The content pair (m54): the m46 derivation orders — the
            // text kind reads the m29 order (name first, value fallback),
            // the value kind the m31 flip.
            ExpectKind::Text(wanted) => {
                Self::content_verdict(row, &text_content(row), wanted, "text")
            }
            ExpectKind::Value(wanted) => {
                Self::content_verdict(row, &value_content(row), wanted, "value")
            }
            // The identity rows (m62 — the m38 vocabulary over the door):
            // the role compares exactly, the name reads the label-only
            // fact — a node with no name stays pending (waitable), the
            // reason naming which fact is missing.
            ExpectKind::Role(wanted) => {
                if row.role == *wanted {
                    ExpectEval::Satisfied(row.clone())
                } else {
                    ExpectEval::Pending {
                        reason: format!("role is {:?}, expected {:?}", row.role, wanted),
                    }
                }
            }
            ExpectKind::Name(wanted) => match row.name.as_deref() {
                Some(found) if TextMatch::from(wanted.clone()).matches(found) => {
                    ExpectEval::Satisfied(row.clone())
                }
                Some(found) => ExpectEval::Pending {
                    reason: format!("name is {found:?}, expected {}", render(wanted)),
                },
                None => ExpectEval::Pending {
                    reason: "the set's head carries no accessible name".into(),
                },
            },
            // The m64 kind never rides the M39 rows (they carry no
            // description fact to read): the misroute stays pending with
            // the honest reason instead of silently never matching.
            ExpectKind::Description(_) => ExpectEval::Pending {
                reason: "the description kind rides the m64 identity read \
                         (evaluate_identity), not the M39 describe-all rows"
                    .into(),
            },
        }
    }

    /// The identity-row evaluation (m64): one row set of the appended
    /// `DescribedAllIdentity` read — the description kind reads the
    /// description fact (the m38 read: `description()` only, a missing
    /// description stays pending, the reason naming the missing fact),
    /// and every other kind projects its row into the M39 shape and
    /// answers exactly as [`ExpectTracker::evaluate`] does (the identity
    /// four ride the wider row beside the description fact — one
    /// evaluator, no second copy of the per-kind verdicts).
    pub fn evaluate_identity(&self, rows: &[DescribedIdentityV1]) -> ExpectIdentityEval {
        let Some(row) = rows.first() else {
            return ExpectIdentityEval::Pending {
                reason: "no match yet (the set is empty)".into(),
            };
        };
        match &self.kind {
            ExpectKind::Description(wanted) => match row.description.as_deref() {
                Some(found) if TextMatch::from(wanted.clone()).matches(found) => {
                    ExpectIdentityEval::Satisfied(row.clone())
                }
                Some(found) => ExpectIdentityEval::Pending {
                    reason: format!("description is {found:?}, expected {}", render(wanted)),
                },
                None => ExpectIdentityEval::Pending {
                    reason: "the set's head carries no accessible description".into(),
                },
            },
            // The content and identity kinds project (the m62/m54
            // verdicts over the M39 row shape, the description fact
            // riding beside them); the satisfying answer stays the full
            // identity row.
            _ => match self.evaluate(&[DescribedV1::from(row)]) {
                ExpectEval::Satisfied(_) => ExpectIdentityEval::Satisfied(row.clone()),
                ExpectEval::Pending { reason } => ExpectIdentityEval::Pending { reason },
            },
        }
    }

    /// The content pair's verdict: the matcher decides, the pending
    /// reason names both sides (the failures-name-names discipline — the
    /// fact read, the wanted matcher).
    fn content_verdict(
        row: &DescribedV1,
        found: &str,
        wanted: &TextMatchV1,
        fact: &'static str,
    ) -> ExpectEval {
        if TextMatch::from(wanted.clone()).matches(found) {
            ExpectEval::Satisfied(row.clone())
        } else {
            ExpectEval::Pending {
                reason: format!("{fact} is {found:?}, expected {}", render(wanted)),
            }
        }
    }
}

/// The wanted matcher's render (the core's check vocabulary shape: the
/// exact pattern quoted bare, the fuzzy one suffixed — `actionability.rs`
/// renders the same words the tree rows print).
fn render(m: &TextMatchV1) -> String {
    if m.exact && !m.ignore_case {
        format!("{:?}", m.text)
    } else {
        format!("{:?} (fuzzy)", m.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email() -> DescribedV1 {
        DescribedV1 {
            node: crate::schema::NodeKeyV1::Logical {
                adapter: "test".into(),
                id: 5,
            },
            generation: 0,
            role: "TextInput".into(),
            name: Some("Email".into()),
            value: Some("ada@example.dev".into()),
            bounds: Some([16.0, 128.0, 256.0, 156.0]),
        }
    }

    /// The two source orders can never silently agree on the wrong datum
    /// (the m46 boundary, restated where it became waitable).
    #[test]
    fn the_orders_flip_between_the_kinds() {
        let text = ExpectTracker::new(ExpectKind::Text(TextMatchV1 {
            text: "ada@example.dev".into(),
            exact: false,
            ignore_case: true,
        }));
        assert!(matches!(
            text.evaluate(&[email()]),
            ExpectEval::Pending { .. }
        ));
        let value = ExpectTracker::new(ExpectKind::Value(TextMatchV1 {
            text: "ada@example.dev".into(),
            exact: false,
            ignore_case: true,
        }));
        assert!(matches!(
            value.evaluate(&[email()]),
            ExpectEval::Satisfied(_)
        ));
    }

    /// The empty content never satisfies a non-empty expectation: a row
    /// carrying neither fact stays pending on both reads.
    #[test]
    fn a_bare_row_stays_pending() {
        let bare = DescribedV1 {
            name: None,
            value: None,
            ..email()
        };
        let text = ExpectTracker::new(ExpectKind::Text(TextMatchV1 {
            text: "Save".into(),
            exact: false,
            ignore_case: true,
        }));
        assert!(matches!(
            text.evaluate(std::slice::from_ref(&bare)),
            ExpectEval::Pending { .. }
        ));
        let value = ExpectTracker::new(ExpectKind::Value(TextMatchV1 {
            text: "Save".into(),
            exact: false,
            ignore_case: true,
        }));
        assert!(matches!(
            value.evaluate(&[bare]),
            ExpectEval::Pending { .. }
        ));
    }
}
