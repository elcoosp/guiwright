//! The bulk-content derivations (m46 — the m41 record's Rust-client
//! half): the source-order rules the kit's `all_text_contents()` (m29)
//! and `all_values()` (m31) established, carried client-side over the
//! M39 `DescribeAll` rows — the same rules the TypeScript client's
//! `allTextContents()` / `allValues()` implement (m41), now shared by
//! every [`crate::client::ClientConn`] impl through the seam's default
//! methods.
//!
//! One boundary, stated once: **no wire change and no fallback**. The
//! reads ride the `DescribeAll` domain byte-for-byte as M39 blessed it;
//! a driver that predates M39 answers the typed `Unsupported` and the
//! read surfaces that rejection rather than degrading (the m41
//! fail-fast rule — the REPL's per-row fallback is the interactive
//! surface's concession, never this one's; the wait module's boundary,
//! restated for content).
//!
//! The rules themselves, one function each:
//!
//! * **Text** ([`text_content`], the m29 order): the accessible name
//!   first, the value as the fallback — the source order the text
//!   locators match (the core `HasText` check reads label-or-value with
//!   the label first). The fixture's Email input answers `"Email"`
//!   here, never its value.
//! * **Values** ([`value_content`], the m31 order): the value first,
//!   the accessible name as the fallback — a form dump wants what the
//!   form *carries* (the `getByValue` source order). The same Email
//!   input answers `"ada@example.dev"` here, so the two reads can
//!   never silently agree on the wrong datum.
//! * A node carrying neither fact reads as the empty string, keeping
//!   `Vec` positions 1:1 with the document order the set read
//!   established (the m31 rule — the kit's `unwrap_or_default()`).

use crate::schema::DescribedV1;

/// The row's text content (the m29 order): the accessible name first,
/// the value as the fallback, `""` when neither fact rides the row.
pub fn text_content(row: &DescribedV1) -> String {
    row.name
        .clone()
        .or_else(|| row.value.clone())
        .unwrap_or_default()
}

/// The row's value content (the m31 order): the value first, the
/// accessible name as the fallback, `""` when neither fact rides the
/// row — [`text_content`] with the source order flipped for form
/// semantics.
pub fn value_content(row: &DescribedV1) -> String {
    row.value
        .clone()
        .or_else(|| row.name.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: Option<&str>, value: Option<&str>) -> DescribedV1 {
        DescribedV1 {
            node: crate::schema::NodeKeyV1::Logical {
                adapter: "test".into(),
                id: 1,
            },
            generation: 0,
            role: "TextInput".into(),
            name: name.map(Into::into),
            value: value.map(Into::into),
            bounds: Some([16.0, 16.0, 136.0, 48.0]),
        }
    }

    /// The m29 order pinned: label first, value never satisfies the
    /// text read — the fixture's Email input answers `"Email"`.
    #[test]
    fn text_content_reads_the_label_first() {
        assert_eq!(
            text_content(&row(Some("Email"), Some("ada@example.dev"))),
            "Email"
        );
    }

    /// The m31 order pinned: value first, label the fallback — the same
    /// row answers `"ada@example.dev"` to the value read.
    #[test]
    fn value_content_reads_the_value_first() {
        assert_eq!(
            value_content(&row(Some("Email"), Some("ada@example.dev"))),
            "ada@example.dev"
        );
    }

    /// The fallback half of each order: a value-less row answers its
    /// label (both orders agree — the fixture's buttons), a label-less
    /// row answers its value (the m31 read alone).
    #[test]
    fn the_fallbacks_cover_the_one_fact_rows() {
        let labeled = row(Some("Save"), None);
        assert_eq!(text_content(&labeled), "Save");
        assert_eq!(value_content(&labeled), "Save");
        let valued = row(None, Some("GW-001"));
        assert_eq!(text_content(&valued), "GW-001");
        assert_eq!(value_content(&valued), "GW-001");
    }

    /// Neither fact: the empty string — positions stay 1:1 with the
    /// document order the set read established (the m31 rule).
    #[test]
    fn neither_fact_reads_as_the_empty_string() {
        assert_eq!(text_content(&row(None, None)), "");
        assert_eq!(value_content(&row(None, None)), "");
    }
}
