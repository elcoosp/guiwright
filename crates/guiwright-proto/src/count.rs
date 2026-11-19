//! The client-side count assertion (m59 — the kit's count vocabulary
//! over any door): Playwright's `expect(locator).toHaveCount(n)` family
//! for remote suites — the M22 exact form and the M25 one-sided
//! comparators. The vocabulary and the set evaluation live here — a
//! pure, transport-blind core — while the wire composition (subscribe →
//! resolve-all re-reads → wake on the §7 event channel) rides
//! [`crate::client::ClientConn::expect_count`] and its comparator rows as
//! default methods, so every shipped client inherits them.
//!
//! Three boundaries keep this honest:
//!
//! * **No wire change.** The comparison below is a client-side
//!   vocabulary (never serialized): the assertion composes the existing
//!   `ResolveAll` and event domains byte-for-byte as M12/M30 blessed
//!   them. A driver that predates M30 rejects `ResolveAll` with the
//!   typed `Unsupported` and the assertion surfaces that rejection
//!   rather than degrading (the m41 fail-fast rule; the wait module's
//!   boundary, restated for counts).
//! * **The set is the truth.** Evaluation sees exactly what the M30
//!   `resolve_all` walk answers — the whole match set, strict mode never
//!   consulted (the m22 record's set read). The kit's count vocabulary
//!   composes this loop over `resolve_all` instead of the m54 content
//!   pair's describe-all: a count is a fact about the set, so the set
//!   read is the honest round trip (the m54 record's named gap, closed).
//!   Absence is an answer: an empty set is found 0 — satisfying
//!   `count 0` and every less-than (Playwright parity), unlike the
//!   content reads' empty-set pending.
//! * **Failures name names** (§8.3, the kit's discipline): the typed
//!   timeout carries the locator render, the budget and the last
//!   evaluation — the demanded comparison and the count the last
//!   evaluation found — never a bare "timed out". The core
//!   [`CountMatch::describe`] wording rides along (`count > 99`).

use crate::schema::ResolvedMatchV1;
use guiwright_core::wait::CountMatch;

/// The client-side state word the count assertion's typed timeout names
/// (the kit's "stuck on …" convention carried over the wire — the m54
/// `have text` / `have value` shape, count family).
pub const HAVE_COUNT_STATE: &str = "have count";

/// One evaluation step's answer: either the comparison holds — carrying
/// the count that satisfied it (the fact the assertion speaks, the m57
/// summary shape) — or the reason it is still pending, which becomes the
/// timeout error's "last evaluation" line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountEval {
    /// The comparison holds; `found` is the count that satisfied it at
    /// the generation the walk answered.
    Satisfied { found: usize },
    /// The world may still change so the assertion can pass; the reason
    /// names the demanded comparison and the count the last evaluation
    /// found.
    Pending { reason: String },
}

/// The set evaluator behind [`crate::client::ClientConn::expect_count`]
/// and its comparator rows. Feed it fresh resolve-all sets after every
/// wake and it answers the assertion; the evaluator carries no
/// cross-wake state (a count has no `Stable`-like frame discipline —
/// every wake re-decides the whole read, the m54 rule).
#[derive(Clone, Debug)]
pub struct CountTracker {
    comparison: CountMatch,
}

impl CountTracker {
    /// A tracker for one assertion, in the comparison it demands.
    pub fn new(comparison: CountMatch) -> Self {
        Self { comparison }
    }

    /// The comparison the assertion demands.
    pub fn comparison(&self) -> CountMatch {
        self.comparison
    }

    /// The static state word the typed timeout names.
    pub fn state(&self) -> &'static str {
        HAVE_COUNT_STATE
    }

    /// Evaluate one match set: the whole set answers (the m22 record's
    /// set read), an empty set is found 0 — an answer like any other,
    /// satisfying `count 0` and every less-than (absence is an answer).
    pub fn evaluate(&self, matches: &[ResolvedMatchV1]) -> CountEval {
        let found = matches.len();
        if self.comparison.holds(found) {
            CountEval::Satisfied { found }
        } else {
            CountEval::Pending {
                reason: format!(
                    "count is {found}, expected {}",
                    render_expected(self.comparison)
                ),
            }
        }
    }
}

/// The demanded comparison's render for the timeout's "expected" side
/// (the core's `count 4` / `count > 3` / `count < 5` vocabulary, the
/// exact form the kit's headers print).
fn render_expected(comparison: CountMatch) -> String {
    match comparison {
        CountMatch::Exact(n) => n.to_string(),
        CountMatch::GreaterThan(n) => format!("> {n}"),
        CountMatch::LessThan(n) => format!("< {n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolve-all row (the set's shape — a count is a fact about the
    /// rows' length, never their contents).
    fn row(id: u64) -> ResolvedMatchV1 {
        ResolvedMatchV1 {
            node: crate::schema::NodeKeyV1::AccessKit {
                tree: [0; 16],
                node: id,
            },
            generation: 0,
            rect: None,
        }
    }

    /// The exact form: the set's length answers; the mismatch names both
    /// sides (the demanded comparison and the count the last evaluation
    /// found).
    #[test]
    fn the_exact_form_answers_the_set_length() {
        let tracker = CountTracker::new(CountMatch::Exact(4));
        assert_eq!(
            tracker.evaluate(&[row(2), row(3), row(6), row(7)]),
            CountEval::Satisfied { found: 4 }
        );
        let pending = tracker.evaluate(&[row(2), row(3)]);
        assert_eq!(
            pending,
            CountEval::Pending {
                reason: "count is 2, expected 4".into()
            }
        );
    }

    /// Absence is an answer: an empty set is found 0 — satisfying
    /// `count 0` and every less-than, never a permanent pending (the
    /// boundary the content reads' empty-set pending does not share).
    #[test]
    fn an_empty_set_is_found_zero() {
        let exact = CountTracker::new(CountMatch::Exact(0));
        assert_eq!(exact.evaluate(&[]), CountEval::Satisfied { found: 0 });
        let under = CountTracker::new(CountMatch::LessThan(3));
        assert_eq!(under.evaluate(&[]), CountEval::Satisfied { found: 0 });
        let over = CountTracker::new(CountMatch::GreaterThan(0));
        assert_eq!(
            over.evaluate(&[]),
            CountEval::Pending {
                reason: "count is 0, expected > 0".into()
            }
        );
    }

    /// The comparators: greater-than and less-than answer the same
    /// found-count fact, mismatches name the comparison (`> 99` /
    /// `< 2`) the way the kit's headers print them.
    #[test]
    fn the_comparators_answer_the_same_found_fact() {
        let four = [row(2), row(3), row(6), row(7)];
        let more = CountTracker::new(CountMatch::GreaterThan(3));
        assert_eq!(more.evaluate(&four), CountEval::Satisfied { found: 4 });
        assert_eq!(
            more.evaluate(&[row(2), row(3)]),
            CountEval::Pending {
                reason: "count is 2, expected > 3".into()
            }
        );
        let fewer = CountTracker::new(CountMatch::LessThan(5));
        assert_eq!(fewer.evaluate(&four), CountEval::Satisfied { found: 4 });
        assert_eq!(
            fewer.evaluate(&[row(2), row(3), row(6), row(7), row(8)]),
            CountEval::Pending {
                reason: "count is 5, expected < 5".into()
            }
        );
    }

    /// The state word the typed timeout names is stable (the m54
    /// vocabulary rule — the REPL/TS mirrors quote it).
    #[test]
    fn the_state_word_is_stable() {
        assert_eq!(
            CountTracker::new(CountMatch::Exact(1)).state(),
            "have count"
        );
    }
}
