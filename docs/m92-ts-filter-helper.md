# M92 — The TypeScript filter pass: the typed `filter()` helper lands client-side

**Status:** implemented, green end to end. The m34 record's remaining-gaps
list named the third filter item: *"The wire carries the full filter
vocabulary unchanged (no new protocol surface here), so other-language
clients keep composing filters through the raw selector rows — a typed TS
`filter()` helper stays with the TS client's own pass."* This record pays
it: the TypeScript client's own filter pass, the m45/m60/m63 one-milestone-
per-surface pattern carried to the §8.1 filter vocabulary.

---

## 1. Why now

The wire has spoken the full filter vocabulary since before the goldens
(`Filter { has_text, has_not, nth }`, M34; the appended `Has` variant, the
m91 record), but the TS `Selector` union carried no filter rows at all — a
remote TS suite composing "the buttons without a Confirm" or "the group
that holds a Save" had to hand-assemble raw rows the codec could not even
encode. The kit's locator DSL grew its filter surface in m34/m91; the TS
client's own pass closes the same wall on the other-language side.

## 2. Design: one helper, the rows it already has

* **`filter(source, opts)` builds one `Filter` row** — `hasText` (a bare
  string rides the Rust `TextMatch` default: substring, case-insensitive;
  a full `TextMatchV1` passes as-is), `hasNot` (the inverted selector
  form) and `nth` (`"first"` / `"last"` / an index) — the field order the
  Rust enum serializes, Options as tags, `NthV1`'s three forms riding the
  new `encodeNth` table.
* **`has` rides its own row, not a `Filter` field (the M64 rule at
  selector width).** The wire's `Filter` payload never grew: the helper
  composes `filter(source, { has })` as `Has { source, has }` wrapping the
  source first, then the `filter` row narrowing the wrapped set — the
  same composition order the Rust kit's `has()`/`has_text()` sugar
  nests by value. No-options calls return the source unchanged; nesting
  composes like every other row.
* **Codec parity, then sugar.** `schema.ts` gains the `filter` row (the
  union + the `Nth` type; the `Has` row landed with m91) and
  `codec.ts` the `filter` encoder — the rows the helper speaks now
  encode byte-identically, pinned by the `resolve_filter` golden vector
  (a `Resolve` carrying the full payload: `has_text` + `has_not` + `nth`
  in one row, blessed into `clients/typescript/test/golden.json`).

## 3. Surface

- `clients/typescript/src/filter.ts` — the typed `filter()` helper
  (`FilterOpts`, the fuzzy `TextMatch` default, the `has`-row
  composition) — exported from the client's main surface
  (`client.ts`, beside the `Selector`/`Nth`/`TextMatchV1` type
  re-exports).
- `clients/typescript/src/schema.ts` — the `filter` row in the
  `Selector` union + the exported `Nth` type (the `Has` row and the
  `SelectorV1.Has` table entry rode m91).
- `clients/typescript/src/codec.ts` — the `filter` encoder + `encodeNth`
  (the `has` encoder rode m91).

## 4. Tests

- `clients/typescript/test/codec.test.ts` — the golden manifest gains the
  `resolve_filter` pin, and the byte-identity build composes the request
  **through the helper** (`filter(...)`), so the helper's output — not a
  hand-rolled row — pins the wire bytes (the m89/m90 sugar-pin pattern).
- `clients/typescript/test/client.test.ts` (+2) — the helper unit tests:
  the fuzzy `hasText` default and the exact passthrough, the three `nth`
  forms, the `has` row riding its own variant, the `has` + `hasText`
  composition order, nesting, and the no-options identity.
- `clients/typescript/test/e2e-full-stack.test.ts` (+1) — the
  cross-language leg against the real Rust demo server (the
  `gwp_demo_app` TCP door): `filter(Group, { has: Button })` resolves the
  ancestor row through the m91 variant, and
  `filter(Button, { hasText: "Save", nth: "first" })` resolves through
  the m34 row and clicks — the composed rows acting over the wire.
- No wire change: the `Filter` and `Has` encodings were pinned by the
  Rust side first (`golden_vectors.rs`), the goldens re-blessed
  (`GUIWRIGHT_BLESS=1`), and the TS codec test fails the build on drift.

## 5. What this does not claim

* No TS locator object: the client stays request-row shaped — `filter()`
  composes selector rows for the existing typed methods
  (`resolve`/`resolveAll`/`describeAll*`/`snapshot filtered`), it does not
  introduce a Playwright-style locator DSL on the TS side.
* No regex `TextPattern`: the m34 record's standing note keeps its own
  trigger (the codegen milestones shipped without asking; the m91 record
  does not cash it either).
* No REPL row: the REPL's get-args vocabulary speaks no filter rows
  (`has_not` included) — the m91 record drew the same line.
