# M66 — The single-node identity describe read (`DescribeIdentity` → `DescribedIdentity`) + the `describe` row's description fact

**Status:** implemented. The m64 record's named standing gap, cashed:
"no single-node describe widening: the `Described` read keeps its M36
encoding; the description fact rides the identity *set* read only. A
handle-addressed widening (the m36 record's shape) lands when a consumer
asks — the same append-only move, one more discriminant." The identity
fact set rides the wire at handle width — the `DescribeIdentity` /
`DescribedIdentity` pair (discriminants 21/24), the M36 `DescribeV1`
handle pair in and the M64 `DescribedIdentityV1` row out — and the
`ClientConn::describe_identity` row carries it to every seam impl, with
the REPL `describe` row as its interactive face (the m37 row rendering
the description fact when the driver answers, the m37/m40 fallback rule
when it does not). **A wire change, append-only**: the M36
`Described`/`DescribedV1` encodings stay byte-pinned (the widening lands
as its own domain reusing the shipped payloads, the m64 payload-reuse
rule at handle width — so every pre-M66 client keeps its byte-exact
conversations); every `ClientConn` impl inherits the row. No budget row
(the m54/m62/m64 rule carried forward: the read is the M36
`describe_wire` conversation with the m64 row answering — the
composition rides the already-budgeted conversations).

---

## 1. Why now

The seam has carried the identity fact set at set width since m64
(`describe_all_identity` + `expect_description`), and the M36 single-node
read since M36 (`describe`) — but the handle-addressed read stayed at
the identity four: a client holding one row (a resolve-all row, a hit
row, a dump row) asking "what does *this* node say about itself" either
opened the whole-set walk (the m64 read costs what `DescribeAll` costs —
fine for a set, a walk for one node) or got the M36 encoding's identity
four with no description fact. The m36 record reserved the state-free
widening ("a describe widening lands when a client asks"), the m64
record named it as the remaining single-node gap, and the REPL `describe`
row was its interactive face: the prompt could print the description
fact only through an `assert description` *expectation*, never through
the read a debugging session speaks.

## 2. Design: the shipped payloads at handle width, one more discriminant

* **Both payloads reuse a shipped shape.** The request is the M36
  `DescribeV1` handle pair verbatim (the dump-row handle + the
  generation it was minted at — the [`RequestV1::Action`] guard's exact
  shape), the reply is the M64 `DescribedIdentityV1` row verbatim (the
  identity four plus the description fact, the request's handle echoed).
  No new payload struct, no new row type — the m64 payload-reuse rule
  carried to handle width, so a client that can act on a row can
  describe it at identity width.
* **The discriminants append (21 / 24)** — the pinning tests
  (`m66_describe_identity_pins`, the schemadoc variant-count probe, the
  golden manifest) fail the build on any reordering, one statement per
  milestone (the m36 pin split's rule); the M36 `Described` reply keeps
  its byte-exact encoding at 21/…, the widening landing at 24.
* **The boundary is the M36 arm's, verbatim.** Handle-addressed, so no
  window scope applies (a point is ambiguous, a selector the tree
  answers — the handle IS the node, the `Action` boundary); a pump runs
  first (consistency); the generation guard rejects a recycled id
  (`StaleGeneration`) and a dead one (`UnknownNode`) as the typed
  errors. The mapping is the m64 factoring's third call site: the m64
  arm's inline row mapping became `described_identity_row`, the one
  copy both identity arms speak (the m64 `described_row` rule — every
  future field change stays a one-line edit).
 * **The REPL consumes the read with the documented fallback** (the
  m37/m40 degrade rule): the `describe` row rides `DescribeIdentity`
  first, the M64 row rendered through `render_identity` (the m37 facts
  with the description fact named, the absent fact rendering nothing —
  never `null`); a driver that predates m66 degrades silently to the
  M36 read and the m37 shape. The fail-fast boundary stays worded
  precisely: the *seam* row has no fallback (a driver older than m66
  cannot decode the appended request at all — the append-only cost,
  the m41 rule); the fallback is the interactive surface's concession,
  exactly as the m40 rows drew it.

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — the `DescribeIdentity`
  request (discriminant 21, the M36 payload), the `DescribedIdentity`
  reply (24, the M64 row), the `m66_describe_identity_pins` test + the
  `describe_identity_domain_roundtrip`.
- `crates/guiwright-proto/src/schemadoc.rs` — the two variant-table rows
  (the published schema contract), golden blessed, TS copy re-vendored.
- `crates/guiwright-proto/src/client.rs` — the `describe_identity` row
  (the typed sugar; every `ClientConn` impl inherits it).
- `crates/guiwright-driver/src/server.rs` — the `DescribeIdentity` arm
  (`describe_identity_response`, the M36 arm's boundary verbatim), the
  `described_identity_row` factoring (the m64 arm's mapping, now one
  copy at both widths).
- `crates/guiwright-cli/src/repl.rs` — the `describe` row riding the
  identity read (the m37/m40 fallback preserved, the m37 facts render
  with the description fact named).
- `crates/guiwright-viewer/src/live.rs` — the append-only
  `response_kind` vocabulary row.
- `clients/typescript` — the variant tables + the codec's encode/decode
  arms + the new request/reply byte pins (the schema mirror moves with
  the published contract; the client method is the next milestone, the
  m45/m60/m63/m65 one-milestone-per-surface pattern).

## 4. Tests

- Red→green: the compile was the red step (the m43/m64 seam-half shape —
  `describe_identity` / `DescribedIdentity` did not exist; the driver
  suite failed against the un-served variant, the append-only
  discipline breaking matches on purpose), the REPL row the runtime
  red (the description fact absent from the prompt's render), the rows
  the green.
- `crates/guiwright-driver/tests/wire_client_describe_identity.rs` —
  the wire conversations over the in-process door: the satisfied form
  (a resolve-all row mints the handle, the identity four ride beside
  the fact), the absent description riding `None` (the Save button —
  a total read, never an error), the ghost handle's typed
  `UnknownNode`.
- The pins: `m66_describe_identity_pins` (discriminants 21/24), the
  `describe_identity_domain_roundtrip` (postcard both ways), the
  schema golden (blessed, variant counts 22/25), the wire golden
  vectors `describe_identity_save_handle` / `described_identity_install_button`
  (blessed), the TS codec pins + the vendored-schema parity
  (`npm test`, 51 passing).
- The REPL conversation rides `repl_e2e.rs`
  `repl_describe_row_rides_the_identity_read`: the described row
  rendering the fact at the prompt, the undescribed row rendering the
  m37 shape unchanged (the absent fact renders nothing).
- The demo conversation is runnable:
  `cargo run -p guiwright-driver --example wire_client_describe_identity`
  prints the identity read answering a resolve-all handle (the fact
  named beside the identity four), the undescribed node's `None`, and
  the ghost handle's typed rejection.

## 5. Remaining gaps (honest list)

- No TypeScript method: the mirror is its own milestone (the
  m45/m60/m63/m65 one-milestone-per-surface pattern) — the codec, the
  variant tables and the byte pins already moved so the m67 record's
  red step is the client method, not the codec.
- No state flags on the identity rows: the checked/focused/enabled
  facts ride the dump rows and the kit assertion vocabulary (the m36
  record's split, unchanged); a state-fact widening lands when a
  consumer asks — the same append-only move, its own domain.
- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the
  m54/m59/m64 rule, carried unchanged).
- No budget row: the m54/m62/m64 rule carried forward (the read rides
  the budgeted `describe_wire` conversation shape; the payload is the
  shipped M64 row, no new cost to guard).
