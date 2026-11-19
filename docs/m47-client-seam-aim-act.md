# M47 — The client seam's aim-and-act rows (`ClientConn::resolve` / `action`)

**Status:** implemented. Two default methods on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`ActionReceipt`] carrier — the §7 locate-and-act loop as typed sugar.
A downstream Rust consumer aims with one strict resolve (the driver's
own auto-wait inside `timeout_ms`) and acts on the returned row — the
dump-row rule (M11) made typable, the same vocabulary the TypeScript
client has spoken since M3 (`resolve()` / `action()`). No wire change:
both ride the `Resolve` and `Action` domains byte-for-byte as M0/M17
blessed them; the schema, the golden vectors and the variant tables are
untouched. One budget row (`client_resolve_act`, the
`resolve` + `action` pair over the in-process door).

---

## 1. Why now

The seam carried the set reads (`resolve_all`, `describe_all`), the
point reads (`hit_test`, `describe`) and the wait — but the act half
stayed raw: applying an action on a row the client already holds meant
hand-building a `RequestV1::Action { node, generation, action, force,
timeout_ms }` and destructuring the `ResponseV1` by variant. The
dump-row rule has promised "rows are directly usable as Action handles"
since M11; m30/m32/m36/m39 all restate it. The TypeScript client has
had `action(node, generation, action, timeoutMs, force)` since M3; the
Rust seam — the mirror every downstream implementer sees — carried the
promise as a doc comment instead of a method.

## 2. Design: the wire's own honesty, as sugar

* **`resolve` is the kit's `get` step over the wire.** Strict mode is
  the wire's own policy (§8.2): an ambiguous selector answers the typed
  `Rejected { code: ErrorCode::StrictModeViolation }`; a selector that
  never matches inside the budget answers `ErrorCode::Timeout` — the
  driver's `resolve_result` auto-wait (the §8.2 server-side attached
  gate, zero polling) is unchanged, and the connection's window scope
  narrows the resolve exactly like `resolve_all`.
* **`action` returns a receipt, not a variant.** `ActionReceipt {
  frame_seq, detail }` carries the `ActionDone` payload (the frame the
  action applied at — the §7 multiplexing rule — plus the adapter's
  detail string) as a named client-side type; never serialized, so the
  schema stays byte-pinned. The act-time gate is unchanged: `force`
  skips exactly stability + hit-test (M17), and the generation guard
  rejects a moved-on row with `ErrorCode::StaleGeneration`.
* **The dead-handle contract is pinned as it is, not as wished.** A
  fabricated (dead) handle fails the boundary's node lookup, which maps
  `CoreError::NoMatch` to `ErrorCode::StrictModeViolation` — the M0
  mapping this arm has carried since the dump-row rule made rows act
  handles, with the message naming the handle. The `Describe`
  boundary's `ErrorCode::UnknownNode` is that read's own contract; the
  two boundaries genuinely differ, and the record states it rather
  than silently re-mapping a pinned wire answer (a unification would be
  a behavior change on an M0 surface — its own decision, not sugar).

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::resolve`,
  `ClientConn::action` and the `ActionReceipt` type.
- `crates/guiwright-driver/tests/wire_resolve_act.rs` — the
  conversations: the resolve→act happy path (row aims the click, the
  receipt names the frame), the ambiguous locator's typed
  `StrictModeViolation`, the missing locator's `Timeout` inside the
  budget, the moved-on generation's `StaleGeneration` and the dead
  handle's pinned rejection.
- `crates/guiwright-driver/examples/wire_resolve_act.rs` — the
  consumer example over the in-process door (resolve, click, the typed
  strict-mode rejection).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_resolve_act` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_resolve_act` — 4/4 over
  the in-process door; the driver suite (76 lib + every integration
  file) green.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named resolve / action` — the m35 record's
  red shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_resolve_act` — the
  demo prints the resolved row, the click's receipt (`frame 6`) and the
  strict-mode rejection line.
- `cargo test -p guiwright-bench --lib` — the budgets parse with the
  new row bound (the copies-match check green).

## 5. Remaining gaps (honest list)

- The Action boundary's dead-handle code (`StrictModeViolation` via the
  `NoMatch` mapping) differs from the Describe boundary's
  (`UnknownNode`). Unifying them is a behavior change on an M0 surface
  that nothing has asked for yet — recorded here, pinned in
  `wire_resolve_act.rs`, left to its own decision.
- The REPL keeps its `get` + current-node store (the interactive
  affordance the seam has no opinion about); its rows already ride the
  same `Resolve`/`Action` domains this sugar types.
- No TypeScript change: `resolve()` / `action()` are the M3 surface the
  sugar mirrors; the goldens and the schema are untouched
  byte-for-byte.
