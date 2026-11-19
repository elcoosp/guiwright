//! The client-side content assertions (m54 — the kit's `expect` vocabulary's
//! content half over any door): the pure [`ExpectTracker`] pinned without a
//! wire, the same cases `crates/guiwright-proto/tests/wait_tracker.rs` pins
//! row-for-row for the m42 wait — satisfied-on-entry, pending-on-empty, the
//! m29/m31 source orders (label first vs value first, a value never
//! satisfying the other read silently), the Playwright text-match semantics
//! (substring + case-insensitive + whitespace-normalized unless `exact`),
//! and the mismatch reason naming what the last evaluation found.

use guiwright_proto::expect::{ExpectEval, ExpectKind, ExpectTracker};
use guiwright_proto::schema::{DescribedV1, NodeKeyV1, TextMatchV1};

fn row(id: u64, name: Option<&str>, value: Option<&str>) -> DescribedV1 {
    DescribedV1 {
        node: NodeKeyV1::Logical {
            adapter: "test".into(),
            id,
        },
        generation: 0,
        role: "TextInput".into(),
        name: name.map(Into::into),
        value: value.map(Into::into),
        bounds: Some([16.0, 128.0, 256.0, 156.0]),
    }
}

/// The fixture's Email input row shape: name "Email", value
/// "ada@example.dev" — the pair the m29/m31 orders were pinned against.
fn email() -> DescribedV1 {
    row(5, Some("Email"), Some("ada@example.dev"))
}

fn fuzzy(pattern: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: pattern.into(),
        exact: false,
        ignore_case: true,
    }
}

fn exact(pattern: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: pattern.into(),
        exact: true,
        ignore_case: false,
    }
}

/// The text assertion reads the m29 source order: the accessible name
/// first — a satisfied answer hands back the row itself (a directly
/// usable `RequestV1::Action` handle, the dump-row rule).
#[test]
fn text_assertion_reads_the_label_first() {
    let tracker = ExpectTracker::new(ExpectKind::Text(fuzzy("Email")));
    match tracker.evaluate(&[email()]) {
        ExpectEval::Satisfied(satisfied) => {
            assert_eq!(satisfied.name.as_deref(), Some("Email"));
            assert_eq!(satisfied.value.as_deref(), Some("ada@example.dev"));
        }
        ExpectEval::Pending { reason } => panic!("label satisfies the text read: {reason:?}"),
    }
}

/// A value never satisfies the name-first text read while the label is
/// present — the same wrong-datum rule the m46 derivation pinned.
#[test]
fn text_assertion_never_takes_the_value_while_the_label_rides() {
    let tracker = ExpectTracker::new(ExpectKind::Text(fuzzy("ada@example.dev")));
    assert!(
        matches!(tracker.evaluate(&[email()]), ExpectEval::Pending { .. }),
        "the value must not satisfy the text read"
    );
}

/// The value assertion reads the m31 source order: value first — the
/// form-dump flip, the two reads can never silently agree on the wrong
/// datum.
#[test]
fn value_assertion_reads_the_value_first() {
    let tracker = ExpectTracker::new(ExpectKind::Value(fuzzy("ada@example.dev")));
    match tracker.evaluate(&[email()]) {
        ExpectEval::Satisfied(satisfied) => {
            assert_eq!(satisfied.value.as_deref(), Some("ada@example.dev"));
        }
        ExpectEval::Pending { reason } => panic!("value satisfies the value read: {reason:?}"),
    }
}

/// The name never satisfies the value-first read while the value rides.
#[test]
fn value_assertion_never_takes_the_label_while_the_value_rides() {
    let tracker = ExpectTracker::new(ExpectKind::Value(fuzzy("Email")));
    assert!(
        matches!(tracker.evaluate(&[email()]), ExpectEval::Pending { .. }),
        "the label must not satisfy the value read"
    );
}

/// The fallbacks hold at the row edges: a nameless row's value answers
/// the text read; a valueless row's name answers the value read (the
/// `unwrap_or_default` shape the m31 rule established — except a missing
/// datum stays honest instead of answering the empty string here).
#[test]
fn the_fallback_order_holds_at_the_row_edges() {
    let nameless = row(6, None, Some("ada@example.dev"));
    let text = ExpectTracker::new(ExpectKind::Text(fuzzy("ada@example.dev")));
    assert!(matches!(
        text.evaluate(std::slice::from_ref(&nameless)),
        ExpectEval::Satisfied(_)
    ));

    let valueless = row(4, Some("Welcome, Ada"), None);
    let value = ExpectTracker::new(ExpectKind::Value(fuzzy("Welcome, Ada")));
    assert!(matches!(
        value.evaluate(&[valueless]),
        ExpectEval::Satisfied(_)
    ));

    // Neither fact riding the row: both reads stay pending — the empty
    // content can never satisfy a non-empty expectation.
    let bare = row(7, None, None);
    assert!(matches!(
        text.evaluate(std::slice::from_ref(&bare)),
        ExpectEval::Pending { .. }
    ));
    assert!(matches!(
        value.evaluate(&[bare]),
        ExpectEval::Pending { .. }
    ));
}

/// Playwright text semantics: substring, case-insensitive,
/// whitespace-normalized unless `exact` (the m29 match rule the kit's
/// text locators speak). `exact` is exact-after-normalization and
/// case-sensitive — the core `TextMatch` rule the row sets inherit.
#[test]
fn the_match_is_playwright_fuzzy_until_exact() {
    let tracker = ExpectTracker::new(ExpectKind::Text(fuzzy("welcome, ada")));
    let label = row(4, Some("Welcome,  Ada"), None);
    assert!(matches!(
        tracker.evaluate(&[label]),
        ExpectEval::Satisfied(_)
    ));

    let exact_tracker = ExpectTracker::new(ExpectKind::Text(exact("Welcome, Ada")));
    // Same text up to case: the exact flag is case-sensitive.
    let different_case = row(4, Some("welcome, ada"), None);
    assert!(matches!(
        exact_tracker.evaluate(&[different_case]),
        ExpectEval::Pending { .. }
    ));
    // Exact is full-string (not substring), after normalization.
    let substring = row(4, Some("Welcome, Ada and friends"), None);
    assert!(matches!(
        exact_tracker.evaluate(&[substring]),
        ExpectEval::Pending { .. }
    ));
    // Whitespace runs normalize on both sides before the comparison.
    let spaced = row(4, Some("Welcome,   Ada"), None);
    assert!(matches!(
        exact_tracker.evaluate(&[spaced]),
        ExpectEval::Satisfied(_)
    ));
}

/// The assertion pins the set's first row (document order — the same pin
/// the m42 wait and the m47 resolve speak); later rows never answer.
#[test]
fn the_set_head_pins_the_assertion() {
    let tracker = ExpectTracker::new(ExpectKind::Text(fuzzy("Cancel")));
    let rows = [row(2, Some("Save"), None), row(3, Some("Cancel"), None)];
    match tracker.evaluate(&rows) {
        ExpectEval::Pending { reason } => assert!(
            reason.contains("Save"),
            "the head's content names: {reason:?}"
        ),
        ExpectEval::Satisfied(_) => panic!("the second row must not answer"),
    }
}

/// An empty set stays pending — waitable (nodes stream in), never an
/// instant rejection (Playwright parity: the element has not arrived).
#[test]
fn an_empty_set_stays_pending() {
    let tracker = ExpectTracker::new(ExpectKind::Text(fuzzy("Save")));
    match tracker.evaluate(&[]) {
        ExpectEval::Pending { reason } => assert!(reason.contains("no match"), "{reason:?}"),
        ExpectEval::Satisfied(_) => panic!("an empty set cannot satisfy"),
    }
}

/// The mismatch reason names what the last evaluation found and what was
/// expected — the kit's "failures name names" discipline (§8.3), the
/// core `has text …` wording.
#[test]
fn the_mismatch_reason_names_expected_and_found() {
    let tracker = ExpectTracker::new(ExpectKind::Text(exact("Done")));
    let save = row(2, Some("Save"), None);
    match tracker.evaluate(&[save]) {
        ExpectEval::Pending { reason } => {
            assert!(reason.contains("Save"), "found content: {reason:?}");
            assert!(reason.contains("expected"), "expected side: {reason:?}");
        }
        ExpectEval::Satisfied(_) => panic!("mismatch must stay pending"),
    }
}
