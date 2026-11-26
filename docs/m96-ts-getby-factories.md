# M96 — The TS `getBy*` locator factories: the kit's sugar mirror on the other-language client

**Status:** implemented. The m95 record's named TS pass paid in full:
"the TS surface's `RegExp` acceptance — `filter({ hasText: /re/ })`, the
`getBy*` factories — is its own pass (the m45/m60/m63
one-milestone-per-surface pattern)". The record's filter half shipped in
the m95 stream itself (the typed builders + the `RegExp` `hasText`
fold); this record pays the factory half: `getByRole` / `getByText` /
`getByLabel` / `getByValue` / `getByTestId` on the TypeScript client,
mirroring the Rust kit's `get_by_*` sugars
(`crates/guiwright-kit/src/app.rs`), composing the wire's existing rows
client-side. No wire change, no locator object (the m92 record's line —
the client stays request-row shaped): a factory returns the same
`Selector` row a hand-built one carries, so `filter()`, the `all`
chains and every typed method take it unchanged.

---

## 1. Why now

The m95 record's honest list names the factory half as its own pass;
the m45/m60/m63/m65/m67/m69/m71 pattern made every TS surface its own
milestone. The kit has spoken the five sugars since M0
(`get_by_role(role, name)` / `get_by_label` / `get_by_text` /
`get_by_test_id` / `get_by_value`), the core's selector model names the
same five families in its crate docs, and the TS client answered every
hand-built row — the last parity gap between the two clients'
composition ergonomics. The m95 regex rows made the gap concrete: the
TS client could compose a regex probe through the raw builders
(`textRegex(/re/)`), but the Playwright-shaped spelling
(`getByText(/re/)`) had no home.

## 2. Design: the factories compose rows, the acceptance mirrors `Into<TextMatch>`

Three decisions carry the milestone:

* **The acceptance mirrors the kit's `impl Into<TextMatch>`.** A bare
  string rides the fuzzy default (substring, case-insensitive — the
  Rust `TextMatch::new` words) on the byte-pinned string rows; a full
  `TextMatchV1` rides verbatim (the caller's own words — the m92
  `filter()` convention); a `RegExp` rides the m95 regex rows through
  the shared `toRegexPattern` fold (the pattern's own inline flags
  decide case — the wire regex rows carry the pattern only, one shape
  per language; the JS-only flags keep the typed refusal).
* **`getByRole`'s `exact` option is Playwright parity for strings.**
  `exact: true` rides the Rust `TextMatch::exact` words (case-sensitive,
  whole-string — `ignore_case` drops). A `RegExp` name matches as-is —
  Playwright's own rule, and the wire's: the regex rows carry no exact
  flag, anchoring rides the pattern's own `^…$`. No name composes the
  unnamed row (the kit's `role_locator` shape — the whole bucket, strict
  mode at act time).
* **No new row, no locator object.** The five factories emit rows the
  M0 encodings already pin — the wire never moved, the golden manifest
  gained only pins for shapes no earlier vector carried
  (`resolve_role_unnamed`, `resolve_role_named_exact`, `resolve_text`,
  `resolve_label`, `resolve_value` — blessed through the Rust-side
  harness, the m92 `resolve_filter` pin pattern). The client stays
  request-row shaped: factories feed `resolve` / `resolveAll` /
  `describeAll*` / the `action` aim-and-act split, they do not hold the
  client or carry act/assert methods.

## 3. Surface

- `clients/typescript/src/getby.ts` — the five factories, the
  `TextProbe` acceptance (`string | TextMatchV1 | RegExp`), the
  `exact` option on `getByRole`; the module docs carry the m92 line
  and the wire discipline.
- `clients/typescript/src/client.ts` — the factory re-exports beside
  the filter helpers (one import surface for one probe concern).
- `clients/typescript/test/getby.test.ts` — the composition pins (the
  kit-mirrored string rows, the `exact` parity, the `RegExp` fold
  through the shared builders, the composition with `filter()`) and
  the byte pins: eleven factory-composed rows against the golden
  vectors (the five new pins plus `resolve_role_named`,
  `resolve_test_id`, and the m95 regex rows' four resolve pins).
- `crates/guiwright-proto/tests/golden_vectors.rs` — the five new
  request vectors (`getby_requests()`), blessed into
  `clients/typescript/test/golden.json` (49 → 54; the TS side fails
  the build together with the Rust side on drift).
- `clients/typescript/test/e2e-full-stack.test.ts` — the cross-language
  leg against the real Rust demo server: the named/exact/unnamed role
  reads, the label/text/value probes (string and `RegExp` forms), the
  aim-and-act click, and the honest empty read (the mock door carries
  no test ids — they ride the logical-tree path — so the set read
  answers zero, the total read, never an error).
- `clients/typescript/examples/getby_factories.ts` — the consumer demo
  (`npm run example:getby`).

## 4. Tests

- `clients/typescript/test/getby.test.ts` — five pins: the string rows
  (kit-mirrored shapes), the `exact` parity, the `RegExp` acceptance
  and refusal, the `filter()` composition, and the byte identity
  against eleven golden vectors. **Red→green recorded**: the first run
  fails at the import (`Cannot find module '../src/getby.ts'`), the
  module landing turns 4/5 green; the byte pin stays red until the
  Rust vectors are blessed (the two clients' shared drift gate), then
  5/5.
- `npm test` — 83 passing (the five factory pins join the enumeration).
- `npm run e2e` — the full conversation green including the m96
  factory leg over TCP (the build is `--locked`, the CI discipline).
- `npm run example:getby` — the demo conversation prints the named /
  exact / unnamed role reads, the three probes and the honest empty.
- No wire change, no budget row (the m54-family rule): the factories
  ride the existing resolve / resolve-all / describe-all conversations;
  no §15.1 row moves.

## 5. Remaining gaps (honest list)

- No TS locator *object*: the client stays request-row shaped (the
  m92 record's line, unchanged here) — a factory returns a `Selector`
  row; a Playwright-style locator class holding the client and carrying
  act/assert methods stays out of scope until a remote TS suite asks.
- No `getByPlaceholder` / `getByTitle` / `getByAltText` families: the
  wire's a11y substrate carries no placeholder/title/alt vocabulary —
  the honest cap is the same one `get_attribute` names (the parity
  table's a11y-substrate rule).
- The regex `exact` composition stays the pattern's own words: a
  whole-string regex probe writes its own `^…$` (the wire rows carry
  the pattern only — the m95 record's one-shape-per-language rule,
  unchanged here).
