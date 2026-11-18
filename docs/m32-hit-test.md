# M32 — The hit-test wire domain (`HitTest` → `HitTest`)

**Status:** implemented. One append-only request/reply pair on the V1
envelope, the typed `ClientConn::hit_test` sugar, the REPL's `hit`
command, the kit's `App::hit_test` / `WindowHandle::hit_test` surface,
the TypeScript `hitTest()` client row with its golden vectors, one
budget row — the §8.3 "receives events" mechanism opened as a public
read.

---

## 1. Why now

The mechanism has been in the tree service since M0:
`TreeService::hit_test(point)` — the consumer's `node_at_point` walk
under the common filter — is exactly how the `ReceivesEvents`
actionability check decides whether the pointer really reaches the node
(§8.3). But it has been unreachable from every consumer surface since:
no session method, no wire domain, no REPL command, no kit method, no
client row. The only callers were the actionability check and its own
tests. A debugging question every GUI tool eventually asks — *what is
under this point?* — had no answer in the stack, while the answer
already existed one seam below it. This milestone opens that seam: one
total read over any door, the REPL's click-what-you-see loop, and the
viewer/ghost-cursor-shaped client row, with no protocol break and no
new dependency.

## 2. Design: one append-only pair, the point read's exact semantics

The load-bearing decision: **the domain speaks the tree's own bounds
space, and the read is total.** `RequestV1::HitTest { x, y }` carries
physical px in the same space the tree's bounds already speak — the
dump rows' `bounds`, the resolve rects — so a client can hit inside a
rect it just resolved without converting between logical and physical
(space conversions are the adapter's job, §9; the driver never owns a
scale factor, so a logical-px point here would be a silent lie). The
reply, `ResponseV1::HitTest { hit: Option<ResolvedMatchV1> }`, carries
one row with exactly the facts the resolve-all rows standardized
(`node`, `generation`, `rect`) — directly usable as an `Action` handle
(the dump-row rule, M11) — or `None`.

* **Append-only, byte-pinned.** The request appends at discriminant 17
  (after `ResolveAll`), the reply at 20 (after `ResolvedAll`) — every
  pre-M32 client keeps its byte-exact encodings, enforced by the
  pinning tests, the exhaustive schema probes and one golden pass.
* **Totality over the point.** Empty space answers `None` — never an
  error — and no timeout exists on the request: the read never waits
  (the snapshot-read rule M22 recorded, one point instead of a set).
  Strict mode is never consulted; hidden nodes cannot answer
  (`node_at_point` walks under the same `common_filter` the resolution
  pipeline applies).
* **The window scope rides the connection.** The arm runs the same
  scope discipline the resolve paths run — the connection's active
  target (§16 v0.4) narrows the answer, and a hit outside the aimed
  window reads `None` (a cross-window answer would be the one thing a
  debugging read must never do). A hit is a tree fact, not an
  actionability verdict: a disabled node under the point answers with
  its identity — the check vocabulary decides actionability on top.
* **Pump-first consistency.** The session hit-test pumps before it
  answers (the same rule `resolve` follows), so the read describes the
  tree as it is now.
* **The client sugar rides the trait.** `ClientConn::hit_test` is a
  default method in terms of `Self::request` — the `resolve_all`
  pattern — so every shipped client and downstream transport inherits
  the typed surface, and the TypeScript `hitTest()` speaks the
  identical conversation (codec golden vectors + cross-language e2e
  step).

## 3. Surface

- `crates/guiwright-core/src/tree.rs` — `TreeService::hit_test_scoped`
  (the point read under the §16 v0.4 scope rule; the unscoped
  `hit_test` mechanism is unchanged).
- `crates/guiwright-driver/src/session.rs` — `Session::hit_test`
  (pump-first, scope-aware, `Resolved`-shaped).
- `crates/guiwright-driver/src/server.rs` — the `HitTest` arm with the
  connection-scope discipline.
- `crates/guiwright-proto/src/schema.rs` — the `HitTest` request/reply
  pair (discriminants 17 / 20) + pinning + roundtrip tests.
- `crates/guiwright-proto/src/client.rs` — the `ClientConn::hit_test`
  sugar.
- `crates/guiwright-proto/src/schemadoc.rs` — the published-variant
  table rows (the schema-published contract).
- `crates/guiwright-cli/src/repl.rs` — the `hit <x> <y>` command + help
  row: a hit stores the current node (so `last` and the actions aim at
  it), a miss is an info line that leaves the current node untouched.
- `crates/guiwright-kit/src/app.rs` + `window.rs` — `App::hit_test`
  (whole tree) and `WindowHandle::hit_test` (the aimed subtree, the
  M6 aim-time validation intact); nothing recorded (a read without an
  action is not a replayable step — the `resolve()` rule).
- `crates/guiwright-viewer/src/live.rs` — `HitTest` in the wire-misuse
  vocabulary (the exhaustive variant-name table, M30 pattern).
- `clients/typescript/` — the vendored schema copy, codec encode/decode
  arms (the `decodeResolvedMatch` row helper factored out of the
  resolve-all path), the `hitTest()` client method, codec-test pins,
  the cross-language e2e step.

## 4. Tests

- `crates/guiwright-core/src/tree.rs` — the unscoped hit (Save's
  center), the empty-space `None`, the disabled node answering (a tree
  fact, not an actionability verdict), and the two-window fixture
  proving the scope filter (Editor's point under the Preview scope).
- `crates/guiwright-driver/src/session.rs` — the session read on the
  settled mock (identity + generation + bounds) and the miss.
- `crates/guiwright-driver/tests/wire_hit_test.rs` — the domain
  end-to-end over the in-process door: hit + miss, the row as an
  `Action` handle, the window-scope discriminator, the
  `ClientConn::hit_test` sugar.
- `crates/guiwright-proto` — discriminant pins (17 / 20), the
  postcard roundtrip test, the blessed wire golden
  (`hit_test_save_center` / `hit_test_hit` / `hit_test_miss`) and the
  blessed schema golden + re-vendored TS copy.
- `crates/guiwright-kit/tests/hit_test.rs` — the kit surface
  end-to-end: the app-level read (key + bounds pinned against
  `key_of`), the miss, the window handle scoping both directions
  (Editor's point through Preview reads `None`; Preview's own Save
  answers), the dead-handle aim-time failure.
- `crates/guiwright-cli/tests/repl_e2e.rs` — the REPL conversation:
  `hit 76 32` renders identity + generation + rect, stores the current
  node, `click` aims at it, the miss is an info line that does not
  disturb the current node.
- `clients/typescript/test/codec.test.ts` — the byte-identical
  `hit_test_save_center` encoding, the hit/miss decodes, the frontier
  slices extended (`HitTest` at 17 / 20).
- `clients/typescript/scripts/e2e.ts` — the demo server answers
  `hitTest(76, 32)` with the Save row and `hitTest(400, 220)` with
  `null` over TCP (and the M15 WS leg).
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `hit_test_wire` row (sample: one `HitTest` round-trip over the
  in-process door at the Save center, the miss probe in the warm-up),
  enforced by the CI guard.

## 5. Remaining gaps (honest list)

- No element-describe over the wire: the REPL renders the hit as the
  handle (`accesskit#2`) + rect — role/name would ride a
  node-describe domain that does not exist yet (the dump carries
  role/name, but a one-row read does not justify it).
- No hit *batch*: a viewer sweeping many points would open one round
  trip per point; no suite has asked (the set read shipped first for
  the same reason).
- Logical-px conversion stays adapter-side by design (§9): a client
  that wants to hit where a gesture would land converts once using the
  scale factor its own session metadata reports.
- The kit's `App::hit_test` reads the whole tree; a window pin rides
  [`WindowHandle::hit_test`], not a locator — the point read is not a
  selector and deliberately stays out of the locator DSL.
