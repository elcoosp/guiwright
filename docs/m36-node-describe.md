# M36 — The node-describe read (`Describe` → `Described`)

**Status:** implemented. One append-only request/reply pair over GWP —
`RequestV1::Describe(DescribeV1)` (discriminant 18) answers
`ResponseV1::Described(DescribedV1)` (discriminant 21) — the m32 record's
standing gap ("role/name would ride a node-describe domain that does not
exist yet; the dump carries role/name, but a one-row read does not
justify it"), named again by the m35 record ("the node-describe domain
that does not exist yet"). The domain now exists; the one-row read is
justified by two records' worth of REPL rows and viewer-style clients
that could only render the bare handle (`accesskit#2`).

---

## 1. Why now

The m32 hit-test read left an honest gap: the REPL renders the hit as
the handle + rect, and every client that holds a row — a resolve row, a
hit row, a dump row — can act on it but cannot ask *what it is*. The
dump answers content questions for whole subtrees (one walk, every
row); the one-node question — "what IS the node this handle points
at" — had no honest door: a dump is the whole tree for one row's facts,
and the m35 `all` rows render `accesskit#2` because per-row facts had no
read to ride. The debugging loop (hit → what is it → act) closed at the
second step. This milestone is that read: the identity facts — role,
accessible name, value, bounds — as one append-only domain, the same
facts a `SnapshotNodeV1` dump row carries, one node instead of a walk.

## 2. Design: handle-addressed, the Action boundary

The load-bearing decision: **`Describe` is handle-addressed and takes
the `Action` boundary, not the resolve scoping.** A point is ambiguous
(the `HitTest` connection scope narrows it), a selector is answered by
the tree (the resolve paths scope it) — but the handle IS the node, so
there is nothing to scope. The request carries the dump-row form
(`NodeKeyV1` + `generation`), exactly the `Action` payload's identity
half, and the boundary enforces exactly what acting on the row would:

* **The generation guard is the Action boundary's** (`§8.2` step 2): a
  handle minted before the node left and re-entered the tree is the
  typed `Error { code: StaleGeneration }`; a handle the tree does not
  know is `Error { code: UnknownNode }`. A client that can describe a
  row is a client that could act on it — the honesty rides the same
  guard, and a recycled id cannot slip through.
* **Snapshot semantics, pump for consistency.** The read never waits
  (no `timeout_ms`); one pump runs first so the answer matches the
  adapter's latest state, like `resolve`/`hit_test`.
* **The reply echoes the handle** and carries the current generation —
  the row is directly usable as the `Action` node without re-resolving
  (the dump-row rule, M11), and `bounds` restates the resolve rect in
  the same physical-px space.

Everything else is established vocabulary, not new semantics: `role` is
the AccessKit `Debug` name the `SelectorV1::Role` vocabulary and dump
rows speak, `name`/`value` are the accessible name and value the
selectors match, `bounds` are the window-relative physical rect the
resolve rects report.

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — the `Describe`/`Described`
  variants (append-only: 18/21, the pinning tests enforce the order),
  the `DescribeV1`/`DescribedV1` payload structs, the discriminant pins.
- `crates/guiwright-proto/src/schemadoc.rs` — the published variant
  tables (rows 18/21), the schema golden re-blessed and re-vendored to
  `clients/typescript/gwp-schema.json`.
- `crates/guiwright-proto/src/client.rs` — the typed
  `ClientConn::describe` sugar (the hit_test pattern): every shipped
  client and downstream transport inherits it.
- `crates/guiwright-driver/src/session.rs` — `Session::describe` +
  the `NodeDescription` record (the embedded half; exported from the
  crate root).
- `crates/guiwright-driver/src/server.rs` — the `Describe` arm (the
  generation guard at the boundary, the typed error mapping).
- `crates/guiwright-kit/src/app.rs` — `Locator::describe()`: resolves
  through the same strict-mode surface every locator action aims
  through (narrowing and the window pin ride it), then describes the
  match.
- `clients/typescript/src/schema.ts` / `codec.ts` / `client.ts` — the
  `Describe`/`Described` discriminants, the encode/decode rows, the
  typed `describe()` method.
- `crates/guiwright-bench` — the `node_describe` budget row (canonical +
  crate copy) and its sample: one describe round trip on the Save
  button's resolve row, the `hit_test_wire` shape carrying one row
  instead of a point walk.

## 4. Tests

- `crates/guiwright-driver/tests/wire_describe.rs` — the domain
  end-to-end over the in-process door: the resolve→describe conversation
  (role/name/bounds pinned against the fixture, the echoed handle), the
  value read on the Email input, the stale-handle rejection after the
  fixture's scripted removal (`StaleGeneration`), the ghost handle
  (`UnknownNode`), and the `ClientConn` sugar.
- `crates/guiwright-kit/tests/describe.rs` — the kit surface: role/name/
  value/bounds reads, strict-mode composition (the bare role locator
  fails, `.first()` reads document order), the window pin scoping, the
  missing-match failure.
- `crates/guiwright-proto/tests/golden_vectors.rs` — the byte-pinned
  request (`describe_save_handle`) and reply (`described_save_button`)
  vectors; the TS codec test decodes the blessed frames and the vendored
  schema pins the appended rows.
- `clients/typescript/scripts/e2e.ts` — the describe step in the
  cross-language e2e: the hit row mints the handle, the describe reads
  it over the real TCP door, the ghost handle rejects.
- Red→green: the domain test failed against the un-served variant
  (`RequestV1::Describe(_)` not covered — the append-only discipline
  breaking matches on purpose), then 4/4 after the schema, session and
  server arms landed.

## 5. Remaining gaps (honest list)

- The REPL rows still render the handle (`accesskit#2`) — the m32/m35
  records' named consumer lands as its own REPL pass (the m30→m35
  pattern: the protocol half first, the client rows after); the `hit`
  row and a `describe <get-args…>` command are that pass.
- No bulk describe: a client sweeping a whole set opens one round trip
  per row (the m32 batch note, unchanged — the dump answers whole-set
  questions).
- No per-row facts beyond the identity four (role/name/value/bounds):
  state flags (enabled/focused/checked) ride the dump rows and the
  assertion vocabulary; a describe widening lands when a client asks.
