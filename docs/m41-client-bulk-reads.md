# M41 — The TypeScript client's bulk reads (`allTextContents()` + `allValues()`)

**Status:** implemented. Two `GwpClient` methods over the existing M39
`describeAll()` round trip — the m29/m31 kit readers' client half, the
m29 record's remote note answered ("a remote bulk read would extend the
`Resolve` reply … and stays open until a remote suite asks"; the M39
domain retired the schema question — the rows already carry the name
and the value, so the reads are client-side derivations over a
golden-pinned conversation). No wire change, no new vectors, no Rust
change: the methods ride `describeAll()` byte-for-byte as M39 blessed
it.

---

## 1. Why now

M39 gave the TypeScript client the set's *facts* (`describeAll()` —
role, name, value, bounds per row); the kit's *content* reads
(`all_text_contents()`, M29; `all_values()`, M31) had no client row.
A TypeScript suite asking "what does this set say" or "what does the
form carry" over a remote session had to walk the rows by hand — the
exact duplication the kit readers exist to prevent, and the same
REPL-half gap M40 closes on the interactive surface (the m35 record's
follow-up, one milestone per surface). The `ResolvedMatch` rows the
m29 note would have extended never needed extending: M39's rows carry
both facts, so the reads are one `map` over one round trip.

## 2. Design: the kit's source orders, verbatim

* **`allTextContents(selector)` is label-first** — the m29 source
  order: the accessible name, falling back to the value (the source
  order the text locators match). A node with neither fact reads as
  `""`, keeping positions 1:1 with the document order the walk
  established (the m31 rule).
* **`allValues(selector)` is value-first** — the m31 source order:
  what the form *carries*, falling back to the name (the `getByValue`
  source order). The demo fixture's Email input answers `"Email"` to
  one and `"ada@example.dev"` to the other — the embedded suite's
  pinned distinction, now cross-language.
* **No protocol change, no new vocabulary.** Both methods compose the
  M39 `describeAll()` semantics unchanged: non-strict, never waiting,
  an empty set answers with an empty list, the connection's window
  scope narrows the walk. The schema copy, the golden vectors and the
  variant tables are untouched — `codec.test.ts` still fails on any
  vendored drift, and the M41 unit test pins the derivation against
  the blessed `described_all_buttons` frame itself.

## 3. Surface

- `clients/typescript/src/client.ts` — the `allTextContents()` /
  `allValues()` methods (the `describeAll()` riders, the kit readers'
  source orders documented at the method).
- `clients/typescript/README.md` — the domain table's M36/M39 rows
  (the drift: the table stopped at M32 while the client shipped
  `describe()` and `describeAll()` in M36/M39) and the M41 rows.

## 4. Tests

- `clients/typescript/test/client.test.ts` — the bulk reads derive the
  kit source orders from the describe-all rows: a hand-rolled mock
  server answers `DescribeAll` (variant 19) with the *blessed*
  `described_all_buttons` frame replayed byte-for-byte, and both reads
  answer the rows' names in document order — `values` through the
  label fallback (the vector's rows carry no values, the m31 rule).
- `clients/typescript/scripts/e2e.ts` — the cross-language leg against
  the real Rust demo app over TCP: the full four-button walk
  label-first, the Email input split across the two source orders
  (`"Email"` vs `"ada@example.dev"`), the empty set total.
- Red→green: the unit test failed against the unfixed client
  (`client.allTextContents is not a function`), passed after the
  methods landed — 12/12 TS unit tests, the e2e conversation green
  through the TCP and WS doors.

## 5. Remaining gaps (honest list)

- No `waitFor` client row: the m21 record's named follow-up stays
  open — a client-side `waitFor(state)` composes the event channel
  with re-resolves, a different shape than the snapshot reads, and
  lands on its own milestone when a remote suite asks.
- No REPL parity for the TS `all()`-family visitor: the m26 read is a
  borrow-and-act shape the wire never carried (the m30 record's
  scoping note) — `describeAll()` rows remain the remote answer for
  set enumeration.
- The derivation is client-side by design: a driver that predates M39
  rejects `DescribeAll` with the typed `Unsupported`, and the reads
  surface that rejection rather than degrading — the REPL's per-row
  fallback (the m40 record) is the interactive surface's concession;
  a programmatic client fails fast instead.
