# M57 — The client seam's tree-snapshot row (`ClientConn::tree_snapshot`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`TreeSnapshotSummary`] carrier — the §7 summary read as typed sugar, the
one-milestone-per-surface pattern (m46→…→m56) carried to the summary read
domain. A downstream Rust consumer asks the whole tree's two facts — the
generation (`tree_seq`, the seq space the `TreeChanged` events report) and
the node count — with one call, instead of hand-building the
`RequestV1::TreeSnapshot` arm and destructuring the reply by field (the
exact shape `remote_session.rs` has carried since the raw-request level).
The same shape the TypeScript client has spoken since its M3 landing
(`treeSnapshot()`). No wire change: the read rides the `TreeSnapshot`
domain byte-for-byte as M0 blessed it; the schema, the golden vectors and
the variant tables are untouched. One budget row (`client_tree_snapshot`,
the pump + count round trip over the in-process door).

---

## 1. Why now

The §7 read vocabulary is the seam's longest arc, and it grew in two
directions: the M11 dump domain made the *rows* readable (sugared by m49 —
the whole tree, a subtree, per window root, each row walked depth-first),
while the M0-era summary read — the wire's oldest read arm, the one the
M11 dump docs call "the summary-only `TreeSnapshot`" — stayed raw. Every
other surface speaks it: the TypeScript client's `treeSnapshot()` has
returned `{ tree_seq, node_count }` since its M3 landing, and the REPL's
status rows render the same two facts. The m56 record completed the §9
input half (synthesis m48 + raw m56) and left the read half's summary arm
as the last unsugared read on the seam; the m51 rule — take the wire's
own vocabulary, no parallel client-side types — applies to it the same
way it applied to the clock, mock, inspect and video surfaces.

## 2. Design: the two summary facts, as sugar

* **One method, the wire's own read.** `tree_snapshot()` — no arguments,
  no narrowing, no client-side filtering: the arm is the M0-era
  whole-tree answer and the sugar refuses to invent a parallel
  vocabulary (the m51 rule). A client that needs the rows asks
  [`ClientConn::tree_dump`]; a client that needs the two summary facts
  asks this row.
* **The carrier, not the variant.** `TreeSnapshotSummary { tree_seq,
  node_count }` carries the `TreeSnapshot` payload as a named client-side
  type; never serialized itself, so the schema stays byte-pinned (the
  `InputReceipt` rule). Copy + `Eq` like the other receipt carriers — a
  read's answer is a value, comparable across calls.
* **The honesty is pinned as shipped.** The read is session-wide by
  design — the connection's window scope narrows locators, not
  whole-tree reads (the §16 v0.4 rule the dump rows carry): the wire
  test aims the connection at a window and still counts 8 nodes. And it
  is a read, not a mutation pump: the session pumps once and reports the
  tree as it is *now*, so back-to-back reads agree with each other and
  with the m49 dump walk's count at the same generation — the wire tests
  pin both.
* **No wire change and no new capability flags.** The read rides the
  `TreeSnapshot` domain byte-for-byte; the dispatch arm is the M0 one,
  untouched; every pre-existing door answer surfaces verbatim.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::tree_snapshot`
  and the `TreeSnapshotSummary` type.
- `crates/guiwright-driver/tests/wire_client_tree_snapshot.rs` — the
  conversations: the summary read naming the fixture's 8 nodes and the
  attach pump's generation, repeated reads staying stable and agreeing
  with the m49 dump row, and the window-scoped connection still reading
  the whole tree.
- `crates/guiwright-driver/examples/wire_client_tree_snapshot.rs` — the
  consumer example over the in-process door (the two stable reads, the
  dump row's agreeing count, the scoped read's whole tree).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_tree_snapshot` row.
- `docs/architecture.md` + `README.md` — the §15.7 row count (60), the
  parity-map M57 sentence, the status-table M57 row and the records
  range.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_tree_snapshot` —
  3/3 over the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named tree_snapshot` ×4 — the m51/m55/m56
  record's red shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_tree_snapshot` —
  the demo prints the summary read (`tree_seq 1 at 8 nodes`), the dump
  row's agreeing count (`8 nodes, agreeing with the summary`) and the
  scoped read (`still 8 nodes (the whole tree)`).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (the round trip rides the mock adapter's pump, the µs-scale
  floor the m56 input row shares).

## 5. Remaining gaps (honest list)

- No window-addressed overload (`tree_snapshot_on(window)`): the wire
  domain carries no window field and the read is session-wide by design —
  a multi-window suite aims the connection first, and the read reports
  the whole tree either way (the wire test pins it).
- The summary read stays the summary read: no growth toward "snapshot +
  rows in one call" — the m49 dump rows are the row-level answer, and the
  M0 arm's promise ("full snapshot rides the wire once traces land") was
  kept by the M11 dump domain, not by widening this one.
- The TS client is an unchanged surface the sugar mirrors (`treeSnapshot()`
  predates the row); the goldens and the schema are untouched
  byte-for-byte.
