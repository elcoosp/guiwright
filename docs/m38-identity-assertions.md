# M38 — The identity assertion family (`to_have_role` + `to_have_accessible_name` + `to_have_accessible_description`)

**Status:** implemented. Three assertion rows on the §8 check vocabulary —
the identity reads the §16 parity map's `toHaveRole` /
`toHaveAccessibleName` / `toHaveAccessibleDescription` columns — surfaced
through the same `Check` → `ExpectBuilder` → session `run` → kit
`Pending` chain every `to_*` rides. No wire change (assertions evaluate
client-side over the tree reads the protocol already serves), no new
waiter machinery (the family rides the existing `ChecksWaiter`), one
budget row (the `checked_assert` shape: a state check on an existing
waiter).

---

## 1. Why now

The M36/M37 describe pass taught the REPL to *print* a node's identity
facts (role/name/value/bounds); the assertion vocabulary can only
*assert* two of them, and fuzzily: `to_have_text` matches the label
**or** the value (M0's read), so "this input shows `ada@example.dev`"
and "this input is named Email" are indistinguishable to a test. The
m36 record's standing note reserved state reads for the assertion
vocabulary; Playwright's identity assertions (`toHaveRole`,
`toHaveAccessibleName`, `toHaveAccessibleDescription`) are the rows the
kit still lacked — the reads a GUI test asks most after visibility:
*what is this node, what is it called, what does it say about itself?*

## 2. Design: three checks, one shape, two deliberate narrowings

* **`Check::Role(accesskit::Role)`** — the node's AccessKit role equals
  the wanted role. The mismatch is **waitable, never fail-fast**: unlike
  the checked state there is no role set a node provably cannot expose
  (a framework re-render can restate the role), so there is no honest
  fail-fast arm. The failure wording reuses the describe/dump vocabulary
  (`role is Button, expected CheckBox`) — the M36 rows' Debug-name
  rendering, so assertion errors read the same words the tree rows
  print.
* **`Check::Name(TextMatch)` is name-only.** The M0 `HasText` check
  matches `label()` *or* `value()`; the identity read matches
  `label()` only. A text input valued `ada@example.dev` and labelled
  `Email` keeps the two assertions apart — the exact distinction
  `toHaveAccessibleName` draws. A missing name stays Pending (labels
  arrive with the node's restatement), never fail-fast.
* **`Check::Description(TextMatch)` is description-only** —
  `description()` on the consumer node. A missing description stays
  Pending for the same reason: descriptions arrive with restatements.
* **One assembly path.** All three ride the existing
  `ChecksWaiter::new(locator, checks)` + `Expectation` pipeline —
  builder methods hand the check list and the operation header
  (`role — getByRole(…)`, `accessible name — …`), the `not()` decorator
  (M24) inverts them for free, strict mode and window scoping apply
  unchanged, and the timeout error names the assertion, the locator and
  the tree (the §8.3 shape).

## 3. Surface

- `crates/guiwright-core/src/actionability.rs` — the `Check::Role` /
  `Check::Name` / `Check::Description` variants, their `describe()`
  strings, the `evaluate` dispatch arms and the three `*_status`
  helpers (`role_status`, `name_status`, `description_status`), plus
  the shared `role_debug_name` renderer.
- `crates/guiwright-core/src/expect.rs` — the `ExpectBuilder`
  `to_have_role` / `to_have_accessible_name` /
  `to_have_accessible_description` methods.
- `crates/guiwright-driver/src/session.rs` — the session
  `ExpectBuilder`'s three rows (the `run(checks, name)` path, window
  scope and negation unchanged).
- `crates/guiwright-kit/src/app.rs` — the kit `Pending` passthroughs.
- `crates/guiwright-core/src/test_util.rs` — the `with_description()`
  fixture (the M38 extension band, `NodeId(14)`: `button "Install"`,
  description `"Installs the latest build"`; the base keeps its
  byte-stable 8-node shape, the `with_checkboxes`/`with_readonly`
  id-allocation rule extended by one slot).
- `crates/guiwright-kit/examples/identity_assertions.rs` — the consumer
  example (the three rows, the name-≠-value failure shape, the `not()`
  composition).

## 4. Tests

- `crates/guiwright-core/src/wait.rs` (unit, the §8 check rows) —
  role pass/pending, name pass, name-never-reads-the-value, the
  nameless-node Pending arm (evaluated directly on the extension-band
  key), description pass on the described fixture, description-absent
  Pending.
- `crates/guiwright-kit/tests/identity_assertions.rs` (end-to-end on
  the mock) — the role row, the wrong-role timeout naming the assertion
  and locator, the name row (exact + fuzzy), the name-never-reads-the-
  value timeout, the description row, the description-absent timeout
  naming assertion + locator, the `not()` composition across all three
  rows.
- Red→green: both suites failed against the unfixed vocabulary (no
  `Check::Role` variant, no builder methods), passed after the
  implementation landed — 100/100 core lib tests, 7/7 kit identity
  tests.

## 5. Budget

- `bench/budgets.toml` + the packaged crate copy — the
  `identity_assert` row (p50 100 µs / p95 500 µs, the `checked_assert`
  shape): one strict resolve plus the consumer-node compare through the
  zero-polling wait, completing on the initial evaluation; no new
  waiter machinery to cost.
- `crates/guiwright-bench/src/samples.rs` — the `identity_assert`
  sample; `crates/guiwright-bench/src/budgets.rs` — the registry entry.

## 6. Remaining gaps (honest list)

- No `to_contain_text` row: the default `TextMatch` is already
  substring + case-insensitive (§8.1), so `to_have_text` covers it —
  a distinct assertion would duplicate semantics, not vocabulary.
- No regex patterns: `TextPattern` stays string-shaped until the
  codegen milestone (the selector.rs note, unchanged).
- The description row reads the consumer's `description()` only —
  the labelled-by composition the *name* resolution applies (§4.8) is
  the consumer's job upstream; a node whose description lives in a
  `labelled_by` graph answers through the same path AccessKit defines.
