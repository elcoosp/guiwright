# M70 — The identity read's state facts at set width (`DescribeAllIdentityState` → `DescribedAllIdentityState`)

**Status:** implemented. The m68 record's named standing gap, cashed:
"No state facts at *set* width: the `DescribeAllIdentity` rows stay
identity-row shaped; a state-fact set widening lands when a consumer
asks — the same append-only move, its own domain." The state three ride
the wire at set width — the `DescribeAllIdentityState` /
`DescribedAllIdentityState` pair (discriminants 23/26), the M39/m64
`DescribeAllV1` walk payload in and the M68 `DescribedIdentityStateV1`
row out per match — and the `ClientConn::describe_all_identity_state`
row carries it to every seam impl, one round trip instead of one
`describe_identity_state` per row. **A wire change, append-only**: the
M39/M64/M66/M68 encodings stay byte-pinned (the widening lands as its
own row per match — the m64 row-widening rule's fourth link, `DescribedV1`
→ `DescribedIdentityV1` → `DescribedIdentityStateV1` → the set reply
carrying the state row, each projection keeping the narrower reads
byte-exact), so every pre-M70 client keeps its byte-exact conversations;
every `ClientConn` impl inherits the row. No budget row (the
m54/m62/m64/m66/m68 rule carried forward: the read is the M39
`describe_all_wire` walk with the m68 row answering per match — the
composition rides the already-budgeted conversations).

---

## 1. Why now

The seam has carried the state facts at handle width since m68
(`describe_identity_state`), and the set walk has carried the identity
row since m64 (`describe_all_identity`) — but a client auditing a set
asking "what is checked, focused, enabled in *this* group" either fired
one handle-width state round trip per row (N questions, N round trips
over the same match set the m39 walk already enumerates in one) or fell
back to the assertion vocabulary one fact at a time. The m68 record
reserved the set-width widening ("a state-fact set widening lands when a
consumer asks"), and the consumer that asks is the audit walk: a remote
suite sweeping a dialog's checkboxes (or a REPL-style batch read over
every row a `resolve_all` minted) wants the tri-state per row, document
order, in the one round trip the set read already costs.

## 2. Design: the m64 payload-reuse rule at set width, the m68 row per match

* **The request reuses the M39/m64 `DescribeAllV1` walk payload verbatim**
  (the m64 payload-reuse rule, one more domain) — the selector whose
  match set is enumerated, the [`RequestV1::ResolveAll`] payload's exact
  shape. Its frame differs from `describe_all_identity_buttons` only in
  the variant byte (the golden vector pins the fact).
* **The reply is the m68 row per match, not a reshape** — the m64
  row-widening rule's fourth link: `DescribedAllIdentityState` carries
  `rows: Vec<DescribedIdentityStateV1>` (the M68 row, unchanged), and
  the `From<&DescribedIdentityStateV1> for DescribedIdentityV1`
  projection keeps the narrower row readable off the wider one (the
  m64/m68 projection chain's fourth link). The M64/M66/M68 encodings
  stay byte-pinned.
* **The discriminants append (23 / 26)** — the pinning tests
  (`m70_describe_all_identity_state_pins`, the schemadoc variant-count
  probes, the golden manifest) fail the build on any reordering, one
  statement per milestone (the m36 pin split's rule); the M68 encodings
  stay byte-exact at 22/…, the widening landing at 23/26.
* **The boundary is the describe-all arm's, verbatim.** Selector-walked,
  so the connection's window scope narrows the walk exactly like the
  M39/M64 arms run it (the server's set/read/clear rendezvous); snapshot
  semantics are the m30 rule — the read never waits, an empty set
  answers with an empty list, never an error. No handle, so no
  generation guard: the rows' generations are current (freshly resolved,
  no stale-guard path).
* **The session read composes, it does not widen**:
  `Session::describe_all_state` calls the untouched
  `Session::describe_all` body and reads the state three beside each row
  (`tree.focus()` equality — the `Check::Focused` read's shape;
  `!node.is_disabled()` — the `Check::Enabled`/`Disabled` read;
  `node.toggled()` — the `Check::Checked` read as a total fact beside
  `NotCheckable`), so every future identity-field change stays the one
  edit the m64 `described_row` factoring bought. A tree that lost a walk
  row between resolve and read answers the typed `UnknownNode`, never a
  fabricated fact.
* **No REPL row.** The m68 handle-width read's interactive face is the
  REPL `describe` row (one handle, one prompt); a set-width walk has no
  single answer to print at the prompt, and the m64 set read's own
  interactive face rode the assertion grammar, not the walk — the seam
  row and the demo are this milestone's consumer faces (the
  one-milestone-per-surface discipline; a REPL surface lands when a
  consumer asks).

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — the `DescribeAllIdentityState`
  request (discriminant 23, the `DescribeAllV1` payload), the
  `DescribedAllIdentityState` reply (26, the m68 row per match), the
  `m70_describe_all_identity_state_pins` test + the
  `describe_all_identity_state_domain_roundtrip` (the projection pin
  riding it).
- `crates/guiwright-proto/src/schemadoc.rs` — the two variant-table rows
  (the published schema contract), golden blessed, TS copy re-vendored.
- `crates/guiwright-proto/src/client.rs` — the
  `describe_all_identity_state` row (the typed sugar; every `ClientConn`
  impl inherits it).
- `crates/guiwright-driver/src/session.rs` — the `Session::describe_all_state`
  read (the `describe_all` body composed, the state three beside each
  row, the typed `UnknownNode` on a drifted walk).
- `crates/guiwright-driver/src/server.rs` — the
  `DescribeAllIdentityState` arm (`describe_all_identity_state_response`,
  the M39/M64 arm's scope semantics verbatim), the m68
  `described_identity_state_row` mapping reused per row (one copy — the
  m64 factoring's rule).
- `crates/guiwright-viewer/src/live.rs` — the append-only
  `response_kind` vocabulary row.
- `crates/guiwright-driver` — the `wire_client_identity_state_all` wire
  conversations + the `wire_client_identity_state_all` consumer example.

## 4. Tests

- Red→green: the compile was the red step (the m43/m64/m66/m68 seam-half
  shape — `describe_all_identity_state` / `DescribedAllIdentityState` /
  the schema variants did not exist; the driver suite failed against the
  un-served variant, the append-only discipline breaking matches on
  purpose), the goldens the runtime red (the drift panic naming the
  bless flow), the rows the green.
- `crates/guiwright-driver/tests/wire_client_identity_state_all.rs` —
  the wire conversations over the in-process door: the checkbox trio's
  tri-state per row in document order (checked / unchecked / mixed
  riding the identity four), the handle-echo conversation (the resolve
  row's id and generation riding the set row, the m64 projection
  readable off it), the disabled button's `enabled: false` +
  `NotCheckable` at set width, the empty set's honest empty list.
- The pins: `m70_describe_all_identity_state_pins` (discriminants
  23/26), the `describe_all_identity_state_domain_roundtrip` (postcard
  both ways + the projection), the schema golden (blessed, variant
  counts 24/27 + the frontier probes), the wire golden vectors
  `describe_all_identity_state_checkboxes` /
  `described_all_identity_state_checkboxes` (blessed), the TS copy
  re-vendored byte-identical.
- The demo conversation is runnable:
  `cargo run -p guiwright-driver --example wire_client_identity_state_all`
  prints the checkbox walk's tri-state per row in document order, the
  disabled button's `enabled false`, and the empty set's honest empty
  list.

## 5. Remaining gaps (honest list)

- No TypeScript method: the mirror is its own milestone (the
  m45/m60/m63/m65/m67/m69 one-milestone-per-surface pattern) — the
  codec, the variant tables and the byte pins already moved so the next
  record's red step is the client method, not the codec.
- No kit-side read over the state rows: the kit's `describe` /
  `describe_all` keep their `NodeDescription` surfaces; a kit-side
  `describe_state` / `describe_all_state` lands when a consumer asks
  (the m68 record's kit note, carried unchanged).
- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the
  m54/m59/m64/m67/m68 rule, carried unchanged).
- No budget row: the m54/m62/m64/m66/m68 rule carried forward (the read
  rides the budgeted `describe_all_wire` conversation shape; the payload
  is the shipped M68 row per match, no new cost to guard).
