# M65 — The TypeScript client's description assertions (`expectDescription()`)

**Status:** implemented. The m64 Rust rows mirrored on the other-language
client (the m45/m60/m63 one-milestone-per-surface pattern): the
`"description"` kind riding the same `ExpectTracker` evaluator over the
identity rows the appended `describeAllIdentity()` read answers, the m54
loop composition as `expectIdentityContent()` (the `expectContent()` loop
shape over the wider re-reads), the cross-language e2e leg against the
real Rust server, and the consumer demo (`npm run example:description`).
No wire change beyond the m64 domain the codec and variant tables already
carry (the m64 record moved them so this milestone's red step was the
client method, not the codec).

## 1. Surface
- `clients/typescript/src/expect.ts` — the `"description"` kind, the
  `have description` state word, the description-only evaluation (the
  name and the value never answer; a missing description stays pending
  naming the missing fact).
- `clients/typescript/src/client.ts` — `describeAllIdentity()` + the
  `expectDescription()` row + the `expectIdentityContent()` composition.
- `clients/typescript/test/description.test.ts` — the pure-evaluator
  pins (row-for-row against the m64 Rust pins).
- `clients/typescript/scripts/e2e.ts` — the description leg (the
  satisfied row, the typed expiry naming both sides, the missing fact
  named) against the demo app, which now serves the described fixture.
- `clients/typescript/examples/description_assertions.ts` — the runnable
  demo (`npm run example:description`).

## 2. Tests
- `npm test` — 51 passing (the description pins added).
- `npm run e2e` — the full conversation green including the m64/m65
  description leg over TCP (and the M15 WS leg byte-identical).
- `npm run example:description` — the demo conversation prints the
  satisfied row, the typed expiry and the missing fact.

## 3. Remaining gaps (honest list)
- No negation on the client: `not()` is the kit decorator's composition
  (M24); the client's honest negative is the typed timeout (the
  m54/m59/m63 rule, carried unchanged).
- The count-pin drift the described fixture introduces rides the e2e
  (the demo app's fixture moved from 8 to 9 nodes — the pins moved with
  it, the fixture contract byte-pinned by the snapshot row).
