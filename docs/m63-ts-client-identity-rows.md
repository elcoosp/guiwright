# M63 — The TypeScript client's identity assertions (`expectRole` / `expectName`)

**Status:** implemented. Two methods on the `GwpClient`
(`clients/typescript/src/client.ts`) — the m62 Rust seam's identity rows
mirrored on the other-language client, the m45/m60
one-milestone-per-surface pattern carried to the identity domain. A
TypeScript suite asserts what a control *is* with one call —
`expectRole(selector, role, timeoutMs)` / `expectName(selector, expected,
timeoutMs)` — and receives the satisfying row, the same conversation the
Rust `ClientConn::expect_role` / `expect_name` make. No wire change: the
assertions compose the existing `subscribe` / `describeAll()` / event
domains byte-for-byte as M12/M39 blessed them; the schema, the golden
vectors and the codec are untouched. The m62 record's named follow-up
closed (*"No TypeScript row: the TS mirror is its own milestone (the
m45/m60 one-milestone-per-surface pattern)"*), and the cross-language
e2e leg pins the rows against the real Rust server.

---

## 1. Why now

m62 put the kit's identity vocabulary (M38 `to_have_role` /
`to_have_accessible_name`) on the Rust seam and the REPL; the TS client —
every other seam row's mirror since M3 — had no identity rows, so a
TypeScript-driven remote suite could wait for a row's text (m54), a set's
size (m59/m60), but not for "this control is the structural kind I
expect" or "this label is what it says it is" over a door. The m60
precedent is exact: the count family mirrored the Rust rows with the
identical evaluator and a cross-language e2e leg; the identity rows get
the same shape, one milestone per surface, with the Rust rows already
green end-to-end.

## 2. Design: the mirror, row for row

* **The kinds ride the same `ExpectKind` union, never serialized** (the
  m62 rule carried to TS): `ExpectKind` gains `"role"` and `"name"`
  beside the m54 content pair, and the tracker's constructor narrows the
  payload per kind — the role carries the exact AccessKit debug name (a
  structural fact takes no matcher, no fuzz), the other kinds a
  `TextMatchV1`. No second evaluator: the identity arms live beside the
  content arms in `ExpectTracker` (the Rust `content_verdict` shape —
  one boundary, stated once).
* **The reads mirror the Rust rows.** The role compares the head row's
  `role` exactly — `role is "Button", expected "TextInput"` on a
  mismatch. The name reads the label-only fact (the m62 narrower read —
  the value never answers, the fact the text row cannot pin); a node
  with no accessible name stays pending (waitable), the reason naming
  which fact is missing (`the set's head carries no accessible name` —
  the m54 empty-content rule's shape at the single-row width).
* **The state words** are `have role` / `have name` (the kit's stuck-on
  convention, the m54 `have text` shape) — `expectState()` names them on
  the typed `GwpWaitTimeout`, beside the locator render and the last
  evaluation (failures name names, §8.3).
* **The composition is the m54 loop, untouched.** `expectRole()` /
  `expectName()` delegate to the private `expectContent()` — subscribe →
  describe-all entry read → wake-and-re-read on the m45 event race →
  `unsubscribe()` on every exit — with the payload widened to
  `TextMatchV1 | string`. No second loop to maintain; the m44 read-half
  contract makes a cancelled read harmless on the wire.
* **No wire change and no codec change.** The assertions ride the
  `DescribeAll` + event domains byte-for-byte; a driver that predates
  M39 rejects `describeAll()` with the typed `Unsupported` and the
  assertion surfaces that rejection (the m41 fail-fast rule). The
  goldens, the schema doc and the codec variant tables are untouched
  byte-for-byte. No budget row (the m61/m62 rule: the rows are
  client-side composition over the already-budgeted `describe_all_wire`
  / `client_assert` conversations — the identity rows are the same loop
  with a different predicate).

## 3. Surface

- `clients/typescript/src/expect.ts` — the `"role"` / `"name"` kinds,
  the per-kind evaluation, the narrowed constructor (the Rust `ExpectKind`
  arms mirrored row for row).
- `clients/typescript/src/client.ts` — `GwpClient::expectRole()` /
  `expectName()` + the `expectContent()` payload widening.
- `clients/typescript/test/expect.test.ts` — the identity pins (below);
  wired into `npm test` by the existing file entry.
- `clients/typescript/scripts/e2e.ts` — the cross-language leg: the
  satisfied role and label-only name forms against the real Rust demo
  server over TCP, the typed expiries naming `have role` / `have name`
  and both sides, the missing-name pending, the door healthy after.
- `clients/typescript/examples/identity_assertions.ts` — the consumer
  demo (runnable: `npm run example:identity`), plus the `tsconfig`
  include and the package script wiring.
- `docs/architecture.md` + `README.md` + `clients/typescript/README.md`
  — the parity-map M63 sentence, the status-table M63 row and the domain
  table's identity row.

## 4. Tests

- `npm test` (`clients/typescript`) — the six new tracker pins ride the
  expect suite: the role answering exactly (and never fuzzily — a
  structural fact), the both-sides mismatch reason, the label-only name
  read (the value never satisfying it), the missing-name pending naming
  the missing fact, the set-head pin, and the four stable state words.
- `npm run e2e` (`clients/typescript`) — the full cross-language
  conversation over a real TCP door, the m62/m63 leg green: `expectRole`
  found the Save row (the set's head), `expectName` answered the Email
  input's label with the value riding beside it, the typed expiries
  (`role is "Button", expected "TextInput"` / `name is "Save", expected
  "Ghost" (fuzzy)` / the unnamed group's missing-fact line), the door
  healthy after.
- Red→green: the pins failed against the unextended tracker (the state
  words and the identity arms did not exist), and the e2e leg failed
  against the unmirrored client (`client.expectRole is not a function` —
  the m60 red shape); the evaluator kinds and the client rows turned
  both green.
- The demo conversation is runnable:
  `npm run example:identity` (`clients/typescript`) prints the role row,
  the label-only name row and the typed timeout.

## 5. Remaining gaps (honest list)

- No description row: the M39 domain does not carry the description fact
  (`DescribedV1` has role / name / value / bounds), so
  `toHaveAccessibleDescription` cannot ride any door without a wire
  change — still reserved until a consumer asks (the append-only
  protocol rule, the m62 record's gap carried unchanged; the wire
  design is the next milestone's first decision).
- No negation on the client: `not()` is the kit decorator's composition
  (M24); the client's honest negative is the typed timeout (the m54/m59
  rule, carried unchanged).
- The Rust `ClientConn::expect_role` / `expect_name` rows are unchanged
  surfaces the mirror speaks to; the goldens and the schema are
  untouched byte-for-byte.
