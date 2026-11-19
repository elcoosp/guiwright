# M64 — The describe-all identity read (`DescribeAllIdentity` → `DescribedAllIdentity`) + the description-assertion rows (`expect_description`)

**Status:** implemented. The m62/m63 records' reserved wire change, cashed:
the description fact rides the wire as its own appended domain — the
`DescribeAllIdentity`/`DescribedAllIdentity` pair (discriminants 20/23),
the same non-strict walk as the M39 read with the rows carrying the
accessible description beside the identity four — and the kit's M38
`to_have_accessible_description` vocabulary rides any door as
`ClientConn::expect_description` (the m54 wake loop over the m64 read),
with the REPL `assert description` row as its interactive face (the m61
grammar's third identity fact). **A wire change, append-only**: the M39
encodings stay byte-pinned (`DescribedV1` is untouched — the widening
lands as its own row reusing the `DescribeAll` payload, so every pre-M64
client keeps its byte-exact conversations); every `ClientConn` impl
inherits the row. No budget row (the m54/m62 rule carried forward: the
identity read is the `describe_all_wire` walk with one optional field per
row, and the assertion is the `client_assert` loop with a different
predicate — the composition rides the already-budgeted conversations).

---

## 1. Why now

The kit has carried the identity family since M38 (`to_have_role` /
`to_have_accessible_name` / `to_have_accessible_description`); the m62
seam rows and the m63 TS rows carried role + name over the doors and
left the description reserved: the M39 `DescribedV1` row has role / name
/ value / bounds, no description fact, and reshaping that row would move
every byte-pinned encoding. The m63 record named the gap as "the wire
design is the next milestone's first decision" — this record is that
decision. The consumer that asked is the same one m62 named: a remote
suite wants "this control describes itself as X" over a door, and the
kit-side assertion cannot ride because the fact is not on the wire.

## 2. Design: the widening as its own domain, the m38 read at row width

* **The row is `DescribedIdentityV1`, not a reshaped `DescribedV1`.**
  Postcard structs encode fields in order with no field count, so adding
  a field to `DescribedV1` would break every older client's decode (the
  `from_bytes` trailing-bytes error) — the append-only discipline's
  whole point. The widening therefore lands as a new request/response
  pair reusing the `DescribeAllV1` payload (the m39 payload-reuse rule):
  the walk costs what `DescribeAll` costs, the rows carry one extra
  optional field, and `DescribedV1` keeps its byte-exact M39 encoding.
  A `From<&DescribedIdentityV1> for DescribedV1` projection names the
  relationship in code.
* **The discriminants append (20 / 23)** — the pinning tests
  (`m64_describe_all_identity_pins`, the schemadoc count check, the
  golden manifest) fail the build on any reordering, the same table
  every milestone since M3 has extended one statement per milestone.
* **The assertion composes the m54 loop over the wider rows** —
  `expect_identity_content` shares the wake (`next_wake`) and expiry
  (`expire_expect`) helpers with `expect_content`, re-reads via
  `describe_all_identity`, and evaluates through
  `ExpectTracker::evaluate_identity`: the description kind reads the
  description fact only (the name and the value never answer — the m38
  read), a missing description stays pending naming the missing fact
  (descriptions arrive with the node's restatement), the set's head
  pins, an empty set stays pending. The content and identity kinds
  evaluate over the identity rows too (one evaluator, the m62/m54
  verdicts projected — no second copy of the per-kind predicates); the
  m54 `evaluate` path is unchanged for the M39 rows, and a `Description`
  tracker misrouted onto them answers a documented pending (the honest
  misroute, never a silent never-match).
* **The session row carries the fact too** (`NodeDescription`
  gains `description`, filled from the consumer node's
  `description()`) — the kit's `describe()` / `describe_all()` rows
  surface it for free (an additive field, no kit API break), while the
  wire's M36 `Described` read stays byte-pinned: the session type is
  richer than the M36 wire row by design, the m64 read is where the
  fact crosses a door.
* **The REPL consumes the row** (`assert description`, the m61/m62
  grammar): the expectation rides the REPL's matcher posture
  (substring, case-insensitive), the satisfying row renders the m37
  facts with the description fact named (`render_identity`), the spent
  budget prints the seam's typed timeout verbatim (`have description`).
* **The fail-fast boundary, worded precisely**: a driver older than m64
  cannot decode the appended request at all (the postcard unknown-variant
  error — the append-only cost every appended domain has carried since
  M3), so there is no fallback derivation (the m41 rule); the REPL's
  per-row fallback concession does not exist here because there is no
  cheaper read to fall back to.

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — the `DescribeAllIdentity`
  request (discriminant 20), the `DescribedAllIdentity` reply (23), the
  `DescribedIdentityV1` row + the M39 projection, the
  `m64_describe_all_identity_pins` test + the round-trip pin.
- `crates/guiwright-proto/src/schemadoc.rs` — the two variant-table rows
  (the published schema contract), golden blessed, TS copy re-vendored.
- `crates/guiwright-proto/src/expect.rs` — the `Description` kind, the
  `ExpectIdentityEval` answers, `evaluate_identity` (the projection for
  the m62/m54 kinds, the description-only read for the m64 kind), the
  documented misroute on `evaluate`.
- `crates/guiwright-proto/src/client.rs` — the `describe_all_identity`
  read + the `expect_description` row + the `expect_identity_content`
  composition (the m54 loop's edges, shared helpers).
- `crates/guiwright-driver/src/session.rs` — `NodeDescription`
  carries `description` (`describe` / `describe_all` fill it).
- `crates/guiwright-driver/src/server.rs` — the
  `DescribeAllIdentity` arm (`describe_all_identity_response`, the M39
  arm's scope semantics), the `described_row` factoring (the identity
  mapping the two M36/M39 arms used to carry by hand, now one copy —
  the m64 mapping is the third call site).
- `crates/guiwright-cli/src/repl.rs` — the `assert description` row,
  the kinds usage string, the help row, the identity render.
- `crates/guiwright-viewer/src/live.rs` — the append-only
  `response_kind` vocabulary row.
- `crates/guiwright-core/src/test_util.rs` — the `describe_save_update`
  scripted restatement (the wake conversation's fixture half).
- `clients/typescript` — the variant tables + `DescribedIdentityNode`
  + the codec's encode/decode arms + the new request's byte pin (the
  schema mirror moves with the published contract; the client methods
  are the next milestone, the m45/m60 one-milestone-per-surface pattern).

## 4. Tests

- Red→green: the compile was the red step (the m43 seam-half shape —
  `expect_description` / `evaluate_identity` / `DescribedIdentityV1`
  did not exist), the REPL row the runtime red (`unknown kind
  \`description\``), the rows the green.
- `crates/guiwright-driver/tests/wire_client_description.rs` — the wire
  conversations over the in-process door: the satisfied forms (the
  fuzzy and exact postures answering the Install button's description,
  the full identity row handed back); the typed timeout naming `have
  description` with the found and expected sides; the missing-
  description pending (the Save button's absent fact named); the
  wake-and-re-read on a scripted restatement (the head row stays the
  Save button across it).
- `crates/guiwright-proto/tests/expect_tracker.rs` — the pure-evaluator
  pins: the description fact answers, the name/value never do, the
  mismatch names both sides, the head pins (and the empty set stays
  pending), the content/identity kinds project over the wider rows.
- The REPL conversations ride `repl_e2e.rs`
  `repl_description_rows_wait_on_the_client_seam`: the satisfied form
  (the description fact named in the render), the exact posture, the
  `have description` timeout at the prompt, the missing-fact pending,
  the kinds usage string, the wake-and-re-read on the scripted
  restatement, and the `events on` filter restored after the
  subscription window (the m43 rule).
- The pins: `m64_describe_all_identity_pins` (discriminants 20/23), the
  schema golden (blessed), the wire golden vector
  `describe_all_identity_buttons` (blessed), the TS codec pin + the
  vendored-schema parity (`npm test`, 46 passing).
- The demo conversation is runnable:
  `cargo run -p guiwright-driver --example wire_client_description`
  prints the description row (the identity four riding beside it), the
  restatement wake, and the typed timeout.

## 5. Remaining gaps (honest list)

- No TypeScript assertion row: the mirror is its own milestone (the
  m45/m60/m63 one-milestone-per-surface pattern) — the codec, the
  variant tables and the byte pin already moved so the m65 record's
  red step is the client method, not the codec.
- No single-node describe widening: the `Described` read keeps its M36
  encoding; the description fact rides the identity *set* read only. A
  handle-addressed widening (the m36 record's shape) lands when a
  consumer asks — the same append-only move, one more discriminant.
- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the m54/m59
  rule, carried unchanged).
- No budget row: the m54/m62 rule carried forward (the walk and the
  loop ride the budgeted `describe_all_wire` / `client_assert`
  conversations; the widening's payload delta is one optional field per
  row, inside the guard's headroom).
