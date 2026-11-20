# M68 — The identity read's state facts at handle width (`DescribeIdentityState` → `DescribedIdentityState`)

**Status:** implemented. The m67 record's named standing gap, cashed:
"No state flags on the identity row: the checked/focused/enabled facts
ride the dump rows and the kit assertion vocabulary (the m36 record's
split, unchanged); a state-fact widening lands when a consumer asks —
the same append-only move, its own domain." The state three ride the
wire at handle width — the `DescribeIdentityState` /
`DescribedIdentityState` pair (discriminants 22/25), the M36
`DescribeV1` handle pair in and the M68 `DescribedIdentityStateV1` row
out — and the `ClientConn::describe_identity_state` row carries it to
every seam impl, with the REPL `describe` row as its interactive face
(the m66 render's facts plus the state three, the m37/m40 fallback
ladder gaining one hop). **A wire change, append-only**: the M36/M64/M66
encodings stay byte-pinned (the widening lands as its own row — the m64
row-widening rule's third link, `DescribedV1` → `DescribedIdentityV1` →
`DescribedIdentityStateV1`, each projection keeping the narrower reads
byte-exact), so every pre-M68 client keeps its byte-exact
conversations; every `ClientConn` impl inherits the row. No budget row
(the m54/m62/m64/m66 rule carried forward: the read is the M36
`describe_wire` conversation with the m68 row answering — the
composition rides the already-budgeted conversations).

---

## 1. Why now

The seam has carried the identity fact set at handle width since m66
(`describe_identity`), and the kit's assertion vocabulary has answered
state questions one at a time since M20 (`to_be_checked` /
`to_be_focused` / `to_be_enabled`) — but a client holding one row (a
resolve-all row, a hit row, a dump row) asking "what does *this* node
say about itself **and its state**" either fired one assertion round
trip per state fact (three questions, three waits, three re-reads) or
opened the whole-tree dump walk (the M11 dump row carries none of the
state facts at all — the m67 record's exact wording). The m36 record
reserved the state facts' widening ("a state-fact widening lands when a
consumer asks"), the m67 record named it as the remaining identity-row
gap, and the state fixture was its natural consumer face: a debugging
session speaks "what's checked, is it focused, is it enabled" about one
node, not about a set and not one fact per round trip.

## 2. Design: the m64 row-widening rule's third link, one more discriminant

* **The request reuses the M36 `DescribeV1` handle pair verbatim** (the
  m66 payload-reuse rule, one more domain) — the dump-row handle plus
  the generation it was minted at, the [`RequestV1::Action`] guard's
  exact shape. Its frame differs from `describe_identity_save_handle`
  only in the variant byte (the golden vector pins the fact).
* **The reply is a new row, not a reshape** — the m64 record's
  row-widening rule's third link: `DescribedIdentityStateV1` carries
  the M64 row's seven fields plus `focused: bool`, `enabled: bool`,
  `checked: CheckedStateV1`, and the `From<&DescribedIdentityStateV1>`
  projection keeps the narrower row readable (the REPL render and any
  identity evaluation project off the wider row, the m64
  `From<&DescribedIdentityV1> for DescribedV1` pattern). The M64/M66
  encodings stay byte-pinned.
* **The checked fact is a four-way word, never an `Option<bool>`.** The
  core `Check::Checked` vocabulary is a tri-state beside a
  not-checkable fact: a mixed checkbox is not an unchecked one, and
  squeezing the tri-state into an Option would lie on the wire. The
  `CheckedStateV1` domain enum (`NotCheckable` / `Unchecked` /
  `Checked` / `Mixed`, the AccessKit `Toggled` mapping at fact width)
  is its own published domain table (the m68 schema row), the same
  published-contract treatment every payload enum gets.
* **The discriminants append (22 / 25)** — the pinning tests
  (`m68_describe_identity_state_pins`, the schemadoc variant-count
  probes, the golden manifest) fail the build on any reordering, one
  statement per milestone (the m36 pin split's rule); the M66 encodings
  stay byte-exact at 22/…, the widening landing at 25.
* **The boundary is the M36 arm's, verbatim.** Handle-addressed, so no
  window scope applies (a point is ambiguous, a selector the tree
  answers — the handle IS the node, the `Action` boundary); a pump runs
  first (consistency); the generation guard rejects a recycled id
  (`StaleGeneration`) and a dead one (`UnknownNode`) as the typed
  errors. The session read composes, it does not widen:
  `Session::describe_state` calls the untouched `Session::describe` body
  and reads the state three beside it (`tree.focus()` equality — the
  `Check::Focused` read's shape; `!node.is_disabled()` — the
  `Check::Enabled`/`Disabled` read; `node.toggled()` — the
  `Check::Checked` read), so every future identity-field change stays
  the one edit the m64 `described_row` factoring bought.
* **The REPL consumes the read with the documented ladder** (the
  m37/m40 degrade rule, one hop per appended widening): the `describe`
  row rides `DescribeIdentityState` first (the state three always
  named — state facts are total, an unfocused node is a fact, not a
  missing one), then falls back to the m66 identity read, then to the
  M36 read. The fail-fast boundary stays worded precisely: the *seam*
  row has no fallback (a driver older than m68 cannot decode the
  appended request at all — the append-only cost, the m41 rule); the
  fallback is the interactive surface's concession, exactly as the m66
  rows drew it.

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — the `DescribeIdentityState`
  request (discriminant 22, the M36 payload), the
  `DescribedIdentityState` reply (25, the M68 row), the
  `CheckedStateV1` vocabulary + the AccessKit `Toggled` mapping, the
  `m68_describe_identity_state_pins` test + the
  `describe_identity_state_domain_roundtrip`.
- `crates/guiwright-proto/src/schemadoc.rs` — the two variant-table
  rows + the `checked_state` domain table (the published schema
  contract), golden blessed, TS copy re-vendored.
- `crates/guiwright-proto/src/client.rs` — the
  `describe_identity_state` row (the typed sugar; every `ClientConn`
  impl inherits it).
- `crates/guiwright-driver/src/session.rs` — the
  `NodeStateDescription` carrier + the `Session::describe_state` read
  (the `describe` body composed, the state three beside it).
- `crates/guiwright-driver/src/server.rs` — the
  `DescribeIdentityState` arm (`describe_identity_state_response`, the
  M36 arm's boundary verbatim), the `described_identity_state_row`
  factoring (the m64/m66 mapping chain's one copy at state width).
- `crates/guiwright-cli/src/repl.rs` — the `describe` row riding the
  state read (the m37/m40/m66 ladder preserved, the state three named
  in the render).
- `crates/guiwright-viewer/src/live.rs` — the append-only
  `response_kind` vocabulary row.
- `crates/guiwright-core/src/test_util.rs` — the `with_state` fixture
  (the checkbox trio + the described button: every state fact has a
  node that carries it).
- `crates/guiwright-driver` — the `wire_client_identity_state` wire
  conversations + the `wire_client_identity_state` consumer example;
  the demo app serves the state fixture (the m64 demo-fixture move's
  shape: the cross-language e2e rides the same tree).

## 4. Tests

- Red→green: the compile was the red step (the m43/m64/m66 seam-half
  shape — `describe_identity_state` / `DescribedIdentityStateV1` /
  `CheckedStateV1` did not exist; the driver suite failed against the
  un-served variant, the append-only discipline breaking matches on
  purpose), the REPL row the runtime red (the state facts absent from
  the prompt's render), the rows the green.
- `crates/guiwright-driver/tests/wire_client_identity_state.rs` — the
  wire conversations over the in-process door: the tri-state trio
  (checked / unchecked / mixed riding the identity four), the disabled
  button's `enabled: false` + `NotCheckable` (a total read, never an
  error), the seeded focus naming the window root (`focused: true`)
  against an unfocused row (`focused: false`), the ghost handle's typed
  `UnknownNode`.
- The pins: `m68_describe_identity_state_pins` (discriminants 22/25 +
  the checked vocabulary's four indices), the
  `describe_identity_state_domain_roundtrip` (postcard both ways), the
  schema golden (blessed, variant counts 23/26 + the `checked_state`
  domain probe), the wire golden vectors
  `describe_identity_state_save_handle` /
  `described_identity_state_install_button` /
  `described_identity_state_mixed_checkbox` (blessed), the TS codec
  pins + the vendored-schema parity (`npm test`, 53 passing).
- The REPL conversation rides `repl_e2e.rs`'s describe row conversations
  (the ladder's facts are the answer either way).
- The demo conversation is runnable:
  `cargo run -p guiwright-driver --example wire_client_identity_state`
  prints the tri-state trio, the disabled button's `enabled false`,
  an unfocused row's `focused false`, and the ghost handle's typed
  rejection.

## 5. Remaining gaps (honest list)

- No TypeScript method: the mirror is its own milestone (the
  m45/m60/m63/m65/m67 one-milestone-per-surface pattern) — the codec,
  the variant tables and the byte pins already moved so the m69
  record's red step is the client method, not the codec.
- No state facts at *set* width: the `DescribeAllIdentity` rows stay
  identity-row shaped; a state-fact set widening lands when a consumer
  asks — the same append-only move, its own domain (the m64/m66
  set/handle split, carried unchanged).
- No kit assertion over the state read: the kit's
  `to_be_checked`/`to_be_focused`/`to_be_enabled` family keeps its
  core-wait semantics (waitable, Pending-until-satisfied); the state
  read is a total read, never a wait, so the kit surface is unchanged —
  a kit-side `describe_state` read lands when a consumer asks.
- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the
  m54/m59/m64/m67 rule, carried unchanged).
- No budget row: the m54/m62/m64/m66 rule carried forward (the read
  rides the budgeted `describe_wire` conversation shape; the payload is
  the shipped M64 row plus three scalars, no new cost to guard).
