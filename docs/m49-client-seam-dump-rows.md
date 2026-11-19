# M49 — The client seam's tree-dump rows (`ClientConn::tree_dump` / `tree_dump_under`)

**Status:** implemented. Two default methods on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`TreeDump`] carrier — the m13 record's dump-row half, closed in the
one-milestone-per-surface pattern m46/m47/m48 established. A downstream
Rust consumer reads the whole logical tree — or the depth-first walk
rooted at one resolved subtree — as typed rows answering the dump-row
rule (M11), the same vocabulary the TypeScript client has spoken since
M11 (`treeDump()`) / M13 (`treeDumpUnder()`) and the REPL since M11
(`snapshot` / `snapshot under`). No wire change: both ride the
`Snapshot` domain byte-for-byte as M11/M13 blessed them; the schema,
the golden vectors and the variant tables are untouched. One budget row
(`client_tree_dump`, the resolve + subtree walk over the in-process
door).

---

## 1. Why now

The dump-row rule has promised "rows are directly usable as handles"
since M11, and the dump is the wire's richest read — identity facts per
node, `children` counts rebuilding the hierarchy without parent
pointers. The REPL has printed the rows since M11/M13 and the
TypeScript client has carried `treeDump()` / `treeDumpUnder()` since
M11/M13; the Rust seam — the mirror every downstream implementer sees —
stopped at the raw `RequestV1::Snapshot` arm, so a consumer wanting
typed rows hand-built the request and destructured
`ResponseV1::TreeDump` by variant (the exact shape
`crates/guiwright-driver/examples/wire_tree_dump.rs` has carried since
M11). The m47 rule — the dump-row promise made typable — was applied to
the resolve/action pair; the dump rows are the same promise on the
read-heavy half.

## 2. Design: the M11 contract, as a carrier

* **Two reads, one carrier.** `tree_dump()` is the full walk (the M11
  contract: depth-first per window root, disjoint by construction —
  deliberately not window-scoped); `tree_dump_under(selector,
  timeout_ms)` is the M13 filtered arm: the driver's auto-wait resolve
  inside the budget (the connection's window scope narrowing the match)
  then the walk rooted at it — a leaf locator answers one row, itself.
  Both rebuild the wire reply into [`TreeDump`] `{ tree_seq,
  node_count, focus, nodes }`, field-for-field the M11/M13 payload.
* **The carrier, not the variant.** `TreeDump` is a client-side type
  mirroring the reply's fields — never serialized itself, so the schema
  stays byte-pinned (the `ActionReceipt` rule; it derives `PartialEq`
  and not `Eq` because the rows are `PartialEq` only, mirroring
  `SnapshotNodeV1` exactly).
* **The resolve's honesty rides the filtered arm.** A locator that
  never matches inside the budget answers the typed `Timeout` naming
  the stuck check; an ambiguous one answers `StrictModeViolation` (§8.2
  strict mode is the wire's own policy) — pinned by conversations, not
  re-mapped. The full walk stays total: a whole-tree dump is never an
  error.
* **No wire change.** Both rides are byte-for-byte the M11/M13
  domains; the golden vectors and the discriminant tables are
  untouched.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::tree_dump`,
  `ClientConn::tree_dump_under` and the `TreeDump` type.
- `crates/guiwright-driver/tests/wire_client_dump.rs` — the
  conversations: the full walk over the four-button fixture (the row
  vec matching `node_count`, the Save row's facts, the hidden button
  riding the walk bounds-less), the leaf locator's one-row subtree, the
  missing locator's typed `Timeout` inside the budget, and a dump row
  feeding [`ClientConn::describe`] on the next line (the dump-row rule,
  made typable).
- `crates/guiwright-driver/examples/wire_client_dump.rs` — the
  consumer example over the in-process door (the full walk, the
  subtree, the typed timeout).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_tree_dump` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_dump` — 4/4 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named tree_dump` ×2, `tree_dump_under` ×3 —
  the m35 record's red shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_dump` — the
  demo prints the full walk (`8 nodes`, the Save row's bounds), the
  Save subtree (`1 row(s)`) and the typed timeout line.
- `cargo test -p guiwright-bench --lib` — the budgets parse with the
  new row bound (the copies-match check green).

## 5. Remaining gaps (honest list)

- No client-side hierarchy rebuild helper: the rows carry `children`
  counts and the M11 contract documents the rebuild; a convenience
  tree-builder stays unasked-for sugar (the m48 record's scope rule).
- The full walk is session-wide by design — a consumer wanting one
  window's whole subtree uses `tree_dump_under` with that window's root
  selector (the M13 shape); no new scope plumbing was added.
- No TypeScript change: `treeDump()` / `treeDumpUnder()` are the
  M11/M13 surfaces the sugar mirrors; the goldens and the schema are
  untouched byte-for-byte.
