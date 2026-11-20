# M75 — The TypeScript client's waitable state assertions (`expectChecked()` / `expectEnabled()` / `expectFocused()`)

**Status:** implemented. The m73 Rust rows mirrored on the
other-language client (the m45/m60/m63/m65/m67/m69/m71
one-milestone-per-surface pattern): the pure `StateTracker` evaluator
mirroring the Rust seam row-for-row (`src/expect.ts`), the shared
`expectState()` composition (the m54 wake loop over the m70
`describeAllIdentityState()` re-reads, event-driven, no polling), the
typed `GwpWaitTimeout` naming `be checked` / `be enabled` / `be
focused` and the fact the last evaluation found, the cross-language e2e
leg against the real Rust server, and the consumer demo (`npm run
example:state-assertions`). **No wire change**: the assertion rides
existing domains byte-for-byte (the m68/m70 encodings stay
byte-pinned).

---

## 1. Why now

The m73 record named the gap: "No TypeScript mirror: `expectChecked()` /
`expectEnabled()` / `expectFocused()` are their own milestone — the
codec, the variant tables and the byte pins already moved so the next
record's red step is the client methods, not the codec." The m71 mirror
had carried the state walk to the TS client; the waitable face stayed
Rust-only. A remote TypeScript suite could read the tri-state but had
to hand-roll its own retry loop around it — exactly the hole the m54
record closed for content. This record closes it the same way: the
facts were already on the wire, the wake loop already existed, and the
composition is client-side only.

## 2. Design: the m63 evaluator's shape at state width

* **The kinds are a client-side vocabulary, never serialized** — the
  `StateKind` rule restated in TypeScript: `"checked"` / `"enabled"` /
  `"focused"` compose the existing `subscribe` +
  `describeAllIdentityState()` domains byte-for-byte. The published
  schema and the byte pins do not move.
* **The evaluator is pure and transport-blind** — `StateTracker` in
  `src/expect.ts` mirrors the Rust `StateTracker` row-for-row: no
  cross-wake state, every wake re-decides the whole read, the set's
  head pinned (document order), the empty set pending on every kind
  (the m54 empty-content rule's shape). The checked word is demanded
  exactly (`"not_checkable"` staying pending under a checked demand —
  the total fact, not an error), the enabled boolean answers both ways,
  the focused fact waits for the focus to arrive.
* **The composition is `expectIdentityContent`'s, one read wider** —
  the private `expectState()` loop: subscribe (`tree_changed` +
  `frame_presented`), evaluate the set's head, wake on the §7 event
  channel, re-read `describeAllIdentityState()` on every wake, drop the
  subscription window on both the satisfied and the expiry path, and
  propagate protocol failures as-is (the m41 fail-fast rule).
* **The failures name names** (§8.3) — the typed `GwpWaitTimeout`
  carries the state word (`be checked` / `be enabled` / `be focused`
  from `stateWord()`), the locator render and the last evaluation: the
  fact read and the fact wanted (`checked is "checked", expected
  "unchecked"`).

## 3. Surface

- `clients/typescript/src/expect.ts` — the `StateKind` vocabulary, the
  `stateWord()` timeout words, the `StateEval` answers and the
  `StateTracker` row-set evaluator (the `ExpectTracker` shape, one
  domain wider).
- `clients/typescript/src/client.ts` — the `expectChecked()` /
  `expectEnabled()` / `expectFocused()` rows and the shared
  `expectState()` composition beside `expectIdentityContent()`.
- `clients/typescript/test/state.test.ts` — the pure evaluator pins.
- `clients/typescript/scripts/e2e.ts` — the cross-language leg.
- `clients/typescript/examples/state_assertions.ts` — the consumer
  demo (`npm run example:state-assertions`).

## 4. Tests

- Red→green: the red step was the client methods (as the m73 record
  promised — the codec had moved with m71): the suite failed against
  the missing rows, the methods the green.
- `npm test` — 60 passing (was 54): the checked word exact (a mixed
  checkbox never answers a checked demand; `not_checkable` stays
  pending with the fact named), the enabled boolean both directions
  with the honest negative, the focused fact, the empty set pending on
  every kind, and the `to_be_*` timeout words.
- `npm run e2e` — the m73/m75 leg against the real Rust demo app over
  TCP: the tri-state demanded exactly on entry (checked / unchecked /
  mixed / the not-checkable total fact), the enabled pair both ways,
  the seeded focus answering the window row, the three typed expiries
  naming `be checked` / `be enabled` / `be focused` and the fact the
  last evaluation found, the empty set pending, and the door healthy
  after the spent assertions.
- The demo conversation is runnable: `npm run example:state-assertions`
  prints the tri-state asserted on entry, the enabled pair, the seeded
  focus and the typed timeout line.
- No budget row: the m54-family rule carried forward (the assertion
  rides the budgeted state-walk conversation shape; the composition
  adds no wire cost to guard).

## 5. Remaining gaps (honest list)

- No REPL row moved here: the `assert` grammar's state kinds are the
  m74 record (landed earlier in the same stream — the
  one-milestone-per-surface discipline kept the faces separate).
- No negation on the seam: the honest negative is the typed timeout
  (the m54-family rule, carried unchanged).
- No kit change: the m68 rule stands — the kit's `to_be_*` vocabulary
  keeps its embedded core-wait semantics.
