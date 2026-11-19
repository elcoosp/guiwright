# M60 — The TypeScript client's count assertion (`expectCount`)

**Status:** implemented. Three methods on the `GwpClient`
(`clients/typescript/src/client.ts`) — the m59 Rust seam's count rows
mirrored on the other-language client, the m45 one-milestone-per-surface
pattern (the m54 cross-language shape) carried to the count domain. A
TypeScript suite asserts a set's size with one call — `expectCount()`,
`expectCountGreaterThan()`, `expectCountLessThan()` — and receives the
found count, the same conversation the Rust
`ClientConn::expect_count` family makes. No wire change: the assertion
composes the existing `subscribe` / `resolveAll()` / event domains
byte-for-byte as M12/M30 blessed them; the schema, the golden vectors
and the codec are untouched. The m59 record's named follow-up closed
(*"No TypeScript row yet: the TS client's `expectText()` /
`expectValue()` has no `expectCount()` mirror"*), and the cross-language
e2e leg pins the family against the real Rust server.

---

## 1. Why now

m59 put the kit's count vocabulary (M22 exact + M25 comparators) on the
Rust seam; the TS client — every other seam row's mirror since M3 — had
no `expectCount()`, so a TypeScript-driven remote suite hand-composed
the subscribe / `resolveAll()` / wake loop the m45 wait pattern
publishes. The m54 precedent is exact: the content pair landed with the
Rust and TypeScript halves riding the identical evaluator and a
cross-language e2e leg; the count family gets the same shape, one
milestone per surface (the m41→m46 and m42→m45 patterns), with the Rust
rows already green end-to-end.

## 2. Design: the mirror, row for row

* **The evaluator mirrors the Rust tracker.** `src/count.ts` —
  `CountCompare` (the client-side vocabulary, never serialized), the
  `holds` predicate, the `renderExpected` wording and the `CountTracker`
  class mirror `crates/guiwright-proto/src/count.rs` row for row: the
  whole set answers, an empty set is found 0 (absence is an answer —
  satisfying `count 0` and every less-than), the mismatch reason names
  both sides ("count is 4, expected 99" / "> 99" / "< 2"). The state
  word `have count` is the m54 `have text` / `have value` shape.
* **The composition mirrors the m54 loop over the set read.** The
  private `expectCountWith()` composes `subscribe(["tree_changed",
  "frame_presented"])` → `resolveAll()` entry read → wake-and-re-read on
  the m45 event race → `unsubscribe()` on every exit — the m54 loop
  riding `resolveAll()` instead of `describeAll()` (the m59 rule: a
  count is a fact about the set, so the M30 set read is the honest
  round trip).
* **The answers match the Rust rows.** The satisfied assertion resolves
  with the found `number`; a spent budget rejects with the typed
  `GwpWaitTimeout` naming the assertion, the locator render and the
  last evaluation. The subscription window drops on success, expiry and
  protocol failure (the m44 read-half contract makes a cancelled read
  harmless on the wire).
* **No wire change and no codec change.** The assertion rides the
  `ResolveAll` + event domains byte-for-byte; a driver that predates
  M30 rejects `resolveAll()` with the typed `Unsupported` and the
  assertion surfaces that rejection (the m41 fail-fast rule). The
  goldens, the schema doc and the codec variant tables are untouched
  byte-for-byte.

## 3. Surface

- `clients/typescript/src/count.ts` — the `CountTracker` evaluator (new
  module: vocabulary, set evaluation, the state word, unit-pinned).
- `clients/typescript/src/client.ts` — `GwpClient::expectCount()` /
  `expectCountGreaterThan()` / `expectCountLessThan()` + the private
  `expectCountWith()` composition.
- `clients/typescript/test/count.test.ts` — the tracker's unit pins
  (the Rust module's cases row for row); wired into `npm test`.
- `clients/typescript/scripts/e2e.ts` — the cross-language leg: the
  exact form and the comparator satisfied against the real Rust demo
  server over TCP, the typed expiry naming `have count` and both sides
  of the comparison, the door healthy after the spent assertion.
- `docs/architecture.md` + `README.md` — the parity-map M60 sentence
  and the status-table M60 row.

## 4. Tests

- `npm test` (`clients/typescript`) — 40/40: the five new tracker pins
  ride the existing suites (the exact form, both-sides reasons, absence
  as an answer, the comparators, the stable state word).
- `npm run e2e` (`clients/typescript`) — the full cross-language
  conversation over a real TCP door, the m59/m60 leg green: `expectCount`
  found 4, the comparator found 4, the typed expiry
  (`have count … count is 4, expected 99`), the door healthy after.
- Red→green: the e2e leg failed against the unmirrored client
  (`client.expectCount is not a function` — the red shape), and the
  unit pins went green with the tracker module.

## 5. Remaining gaps (honest list)

- No negation on the client: `not()` is the kit decorator's composition
  (M24); the client's honest negative is the typed timeout (the m54
  record's rule, carried unchanged).
- No count row in the REPL: the kit, the seam and the TS client carry
  the family; the REPL's `count` read (m35) stays a read, an
  `expect count` row would be the m43 interactive-half pattern's
  follow-up.
- The Rust `ClientConn::expect_count` rows are unchanged surfaces the
  mirror speaks to; the goldens and the schema are untouched
  byte-for-byte.
