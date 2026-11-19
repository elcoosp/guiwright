# M48 — The client seam's gesture rows (`ClientConn::gesture`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`GestureReceipt`] carrier — the §9 input half as typed sugar. A
downstream Rust consumer synthesizes pointer, wheel and keyboard
gestures with one call answering the frame the batch applied at — the
same vocabulary the TypeScript client has spoken since M7 (`gesture()`)
and the REPL since M7 (`mouse` / `key` / `text`). No wire change: the
sugar rides the `Gesture` domain byte-for-byte as M7 blessed it; the
schema, the golden vectors and the variant tables are untouched. One
budget row (`client_gesture`, the click round trip over the in-process
door).

---

## 1. Why now

The seam carried the aim-and-act loop (m47), the set and point reads
(m30/m32/m36/m39), the content derivations (m46) and the wait (m42) —
but the §9 input half stayed raw: driving a drag or a key from a Rust
consumer meant hand-building a `RequestV1::Gesture` and destructuring
`ResponseV1::GestureDone` by variant (the exact shape
`crates/guiwright-driver/examples/wire_gesture.rs` has carried since
M7). Three surfaces speak the vocabulary as typed rows; the seam — the
mirror every downstream implementer sees — carried it as a raw request.
The one-milestone-per-surface pattern (m40→m41→m46, m43→m45→m42,
m47) names this half next.

## 2. Design: the wire's own synthesis, as sugar

* **One method, the whole §9 vocabulary.** `gesture(cmd: GestureV1)`
  takes the wire's own gesture enum (click with Playwright's
  clickCount, the ratchet-interpolated drag, wheel, key, type) — no
  parallel client-side vocabulary to keep in sync, the same rule the
  m47 sugar set for `SemActionV1`.
* **The receipt, not the variant.** `GestureReceipt { frame_seq,
  detail }` carries the `GestureDone` payload (the §15.4 rendezvous
  frame plus the synthesis summary — `"clicked at (76, 32) — 3
  events"`, the same line the REPL's gesture rows render) as a named
  client-side type; never serialized, so the schema stays byte-pinned.
* **The window scope steers the coordinates.** The batch lands in the
  aimed window's pipeline (the M6 handoff the §9 grammar rides) — the
  same scope [`ClientConn::resolve`] aims the walk through, pinned by a
  two-window conversation in the tests.
* **The honesty is pinned as shipped.** A gesture that synthesizes
  nothing (a zero-count click) answers the wire's typed error naming
  the cause (`Internal`: "gesture synthesizes no events (click count
  0?)") — the M7 session-side client-bug rejection carried over, not
  re-mapped (the m47 record's contract rule). The raw
  `RequestV1::Input` batch stays the un-synthesized escape hatch,
  exactly where M7 left it.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::gesture` and
  the `GestureReceipt` type.
- `crates/guiwright-driver/tests/wire_client_gesture.rs` — the
  conversations: the happy click (receipt names the frame, the
  adapter's input log sees move→down→up tagged `main`), the whole
  vocabulary (double-click, drag, wheel, key, type — each answering its
  synthesis summary), the window-scoped click landing in the aimed
  window's pipeline, and the zero-count typed rejection.
- `crates/guiwright-driver/examples/wire_client_gesture.rs` — the
  consumer example over the in-process door (click, wheel, the typed
  rejection).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_gesture` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_gesture` — 4/4
  over the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named gesture` ×8 — the m35 record's red
  shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_gesture` — the
  demo prints the click's receipt (`frame 4`), the wheel's receipt and
  the typed zero-count rejection line.
- `cargo test -p guiwright-bench --lib` — the budgets parse with the
  new row bound (the copies-match check green).

## 5. Remaining gaps (honest list)

- The REPL keeps its `mouse` / `key` / `text` command split (the
  interactive affordance — parsed arguments rendered as rows); its rows
  already ride the same `Gesture` domain this sugar types, so no
  behavior change there.
- No TypeScript change: `gesture()` is the M3 surface the sugar
  mirrors; the goldens and the schema are untouched byte-for-byte.
- No per-gesture helper methods (`click_at` / `drag_between`): the TS
  client keeps the helpers on top of its raw `gesture()` and the kit
  keeps `MouseHandle`/`KeyboardHandle`; the Rust seam stays at the
  vocabulary level until a consumer asks (the same scope rule the m47
  sugar applied to `SemActionV1`).
