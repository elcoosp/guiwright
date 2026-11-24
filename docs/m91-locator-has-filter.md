# M91 — The `has` filter: the selector-valued positive filter lands end to end

**Status:** implemented, green end to end. The m34 record's remaining-gaps
list carried one named follow-on: *"No `filter({ has })` positive-locator
filter on the kit surface yet (`has_text` covers the string form; the
selector-valued positive filter is `All`-chain shaped and reachable via
`locator()` for the chain case — a distinct `has(...)` sugar lands when a
suite asks)."* This record pays it: the Playwright `locator.filter({ has })`
shape, kept by design out of m34 because the wire's `SelectorV1` carried no
positive-filter row and every earlier wire consumer had not asked.

---

## 1. Why now

The m34 filter surface shipped the inverted half (`has_not`) and the
descendant chain (`locator`), both as pure composition over machinery the
resolver already ran. The positive half was the one §8.1 narrowing no
existing row could speak: the `All(chain)` chain narrows to the
*descendant's* rows (`group >> Button` answers the buttons), while `has`
keeps the *ancestor's* rows (`group.has(button)` answers the group) — a
suite composing "the containers that hold a Save" could not reach it
without leaving the locator DSL, exactly the wall m34 removed for the
inverted half. This record closes the vocabulary: every `Selector` the
engine resolves is now reachable from the locator surface.

## 2. Design: the widening is its own variant, the walk is the shared one

Two decisions carry the milestone:

* **A new variant, not a `Filter` field (the M64 rule).** The wire is
  append-only and byte-pinned: adding a `has` field inside
  `SelectorV1::Filter` would reshape the `Filter` encodings every pinned
  client speaks. The widening rides its own appended variant,
  `SelectorV1::Has { source, has }` at the end of the table (discriminant
  7), so every encoding the pre-`has` clients pinned is unchanged and an
  old client answers the unknown variant with a clean decode error. The
  core `Selector::Has` mirrors the wire variant field-for-field, the
  convert tables map 1:1 in both directions, and the TS codec gains the
  `has` row (`schema.ts` table + union, `codec.ts` encoder) with the
  byte pin riding the golden vector.
* **No new walk — the subtree probe is `has_not`'s.** The resolver's
  `Has` arm resolves the source set and retains the matches whose subtree
  (the node included) matches the inner selector — the same
  `subtree_matches_selector` walk `has_text` reads and `has_not` negates,
  now with the positive verdict. The leaf rule carries over unchanged and
  is load-bearing in both directions: a leaf's subtree is itself, so
  `get_by_role(Button).has(&role(Button))` keeps every button (and
  `has_not` excluded them — the two filters read one walk). `matches_selector`
  routes `Has` through the same subtree recursion, so a `has`-filtered
  selector nests inside chains and other filters for free. Composition
  order is the filter shape (`Selector::has` wraps the receiver as the
  source), and the kit's `Locator::has(&Selector)` keeps the borrow rule
  (`&Selector`, cloned — every locator holds the `&mut App` it acts
  through).

## 3. Surface

- `crates/guiwright-core/src/selector.rs` — `Selector::Has { source, has }`
  (the variant + the `has(inner)` sugar in composition order, the render
  arm naming the positive filter: `.filter(has=…)`).
- `crates/guiwright-core/src/tree.rs` — the resolver's `Has` arm (source
  resolve + the subtree retain, document order kept) and the
  `matches_selector` subtree branch.
- `crates/guiwright-kit/src/app.rs` — `Locator::has(inner: &Selector)` (the
  rustdocs carry the leaf rule, the `locator()`/`has_not()` distinction
  and the composition semantics).
- `crates/guiwright-proto/src/schema.rs` — `SelectorV1::Has` appended at
  the table's end (the M64 rule: the `Filter` encodings stay byte-pinned).
- `crates/guiwright-proto/src/convert.rs` — both conversion arms.
- `crates/guiwright-kit` (tests/locator_has.rs) — the surface end-to-end.
- `clients/typescript/src/schema.ts` + `codec.ts` — the `has` row (table
  index 7 + the typed union + the encoder), byte-pinned by the golden.
- `bench/budgets.toml` + the crate copy — the `locator_has_filter` row
  (sample: one has-filtered group read on the button fixture — the source
  resolve plus the positive subtree probes), the `locator_filter` row's
  shape with the positive verdict.

## 4. Tests

- `crates/guiwright-core/tests/selectors.rs` (+3): the positive filter
  keeps the ancestor row and drops on an unmatched name constraint (the
  empty set read); the leaf rule at set width (every button keeps itself,
  document order `2, 3, 6, 7`; the label drops); the `has`-vs-chain
  distinction pinned against the same fixture (`All` chain answers
  `2, 3, 6, 7`, `has` answers `1`) with the render pinned.
- `crates/guiwright-kit/tests/locator_has.rs` (5): the surface end-to-end
  on the mock adapter — the group kept/dropped by its subtree's name
  constraint, the leaf pin mirrored against `has_not`, composition over
  the chain and with `has_not`/`first` (the `has` set's head is the
  ancestor — the exact chain distinction), the strict gate through a
  has-filtered act (one adapter action, the four-match chain refusal
  synthesizing nothing), and the two-window fixture's ambiguity refusal +
  window-pin scoping (each window's content group answers its own
  subtree). **Red→green recorded**: the test file fails to compile against
  the unfixed kit (11 `no method named has` errors), passes 5/5 after the
  variant + sugar land.
- `crates/guiwright-proto` — the convert round-trip test gains the
  `Has` shape; the golden vectors gain `resolve_has` (a `Resolve` request
  speaking the appended variant), blessed into
  `clients/typescript/test/golden.json`; the TS codec test builds the
  request and asserts byte-identity (the m91 pin joining the
  M30/M89/M90 pins).
- Example (`guiwright-kit/examples/locator_has.rs`): the run prints the
  ancestor-row answer, the empty set read, the leaf rule, the composed
  `has` + `has_not` read, the selector render and the strict-gated click —
  `cargo run -p guiwright-kit --example locator_has`.

## 5. What this does not claim

* No REPL row: the REPL's get-args vocabulary speaks no filter rows today
  (`has_not` included) — the m34 surface drew the same line, and the
  REPL filter grammar stays its own pass if a debugging session asks.
* No codegen rule: the cost model constructs selectors and never matches
  `Filter`/`All` variants, so a recorded line through a `has`-filtered
  locator renders via the selector's own render arm — no new tier.
* No schema-doc change: the published schema tables requests, responses
  and events; the selector variants ride inside request payload
  descriptions, so no tabled row moved — the vendored copy is unchanged
  and the byte pins (golden vectors + TS codec) carry the parity.
* No regex `TextPattern`: the m34 record's second standing note ("the
  enum stays open until the codegen milestone asks") keeps its own
  trigger — the codegen milestones shipped without asking, and this
  record does not cash it.
