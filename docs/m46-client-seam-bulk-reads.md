# M46 — The client seam's bulk reads (`ClientConn::all_text_contents` / `all_values`)

**Status:** implemented. One derivation module
(`crates/guiwright-proto/src/bulk.rs`) plus two default methods on the
[`ClientConn`] seam (`crates/guiwright-proto/src/client.rs`) — the m41
record's Rust-client half, closed in the same one-milestone-per-surface
pattern m40/m41/m43 established. A downstream Rust consumer of the seam
(the viewer's live feed, a custom recorder, any [`Handler`]-mirrored
client) derives the kit's m29/m31 source orders over the M39
describe-all round trip — the read the TypeScript client has carried as
`allTextContents()` / `allValues()` since m41 and the REPL as `texts` /
`values` since m40. No wire change: the reads compose `DescribeAll`
byte-for-byte as M39 blessed them; the schema, the golden vectors and
the variant tables are untouched. One budget row
(`client_text_contents`, the `describe_all_wire` round-trip shape plus
the per-row map).

---

## 1. Why now

M41 gave the TypeScript client the bulk reads as client-side
derivations over `describeAll()`; M40 gave the REPL its rows. The Rust
client seam — the §7/§16 surface every shipped client and every
downstream transport implements — stopped at the fact rows: a consumer
holding `describe_all()` rows had to re-derive the content rule by
hand, and the rule is exactly the kind of contract that drifts when it
is re-typed (the m29 order is *label first* — the read the text
locators match; the m31 order is *value first* — the form dump; the
`""`-for-neither rule keeps positions 1:1 with the walk). Three
surfaces carried three copies of one rule; the seam is the fourth and
the only one a downstream implementer sees.

## 2. Design: one rule, two orders, no fallback

* **The derivations are pure and shared.** `bulk.rs` holds
  `text_content(row)` and `value_content(row)` — the m29 and m31
  source orders over a `DescribedV1`, pinned in module tests the way
  the m42 tracker is pinned in `wait_tracker.rs`. The seam methods are
  default methods over `describe_all` (one round trip, no extra wire
  work), so every impl — UDS, TCP, WebSocket, duplex, downstream —
  inherits them without code.
* **Fail fast, not degrade.** A driver that predates M39 rejects
  `DescribeAll` with the typed `Unsupported`; the reads surface that
  rejection rather than falling back — the m41 rule and the wait
  module's boundary, restated for content (the REPL's per-row fallback
  is the interactive surface's concession, never this one's).
* **Snapshot semantics ride the walk.** No strict mode, no waiting, an
  empty set answers an empty list, hidden rows ride the set (the
  fixture's bounds-less "Invisible" button is the pinned case), and the
  connection's window scope narrows the read exactly like
  `describe_all`.

## 3. Surface

- `crates/guiwright-proto/src/bulk.rs` — the new module: the two
  source-order rules as pure functions + the module tests.
- `crates/guiwright-proto/src/lib.rs` — the module wired into the root
  (the crate doc's module list gains the `bulk` row).
- `crates/guiwright-proto/src/client.rs` — `ClientConn::all_text_contents`
  and `ClientConn::all_values` (the default methods over
  `describe_all`).
- `crates/guiwright-driver/tests/wire_texts_values.rs` — the
  conversations: the four-button set's label-first rows (hidden
  included, document order), the Email input answering `"Email"` to
  texts and `"ada@example.dev"` to values (the two orders can never
  silently agree on the wrong datum), the empty set's total answer, and
  the pre-M39 driver's typed rejection.
- `crates/guiwright-driver/examples/wire_texts_values.rs` — the
  consumer example over the in-process door.
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_text_contents` row.

## 4. Tests

- `cargo test -p guiwright-proto` — 70 lib tests green (the four
  module tests: label-first, value-first, the one-fact fallbacks, the
  neither-fact `""`), the integration suites untouched and green.
- `cargo test -p guiwright-driver --test wire_texts_values` — 4/4 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named all_text_contents` — the m35 record's
  red shape), passed after the derivations landed.
- `cargo run -p guiwright-driver --example wire_texts_values` — the
  demo prints the button set's texts, the input's text-vs-value pair
  and the empty set.

## 5. Remaining gaps (honest list)

- The REPL keeps its own render (the handle form beside the content)
  and its pre-M39 fallback — the interactive surface's documented
  concession; no seam change forces it onto the fail-fast read.
- No TypeScript change: the TS client has carried both reads since
  m41; the Rust seam is now at parity, nothing to port back.
- No wire/protocol surface: the reads are derivations over the M39
  domain, so the goldens and the schema are untouched byte-for-byte.
