# M56 — The client seam's raw-input row (`ClientConn::input`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`InputReceipt`] carrier — the §9 raw-injection half as typed sugar, the
one-milestone-per-surface pattern (m46→…→m55) carried to the raw input
domain, the escape hatch the m48 gesture row's contract explicitly
names. A downstream Rust consumer injects a hand-built `InputEventV1`
batch with one call and awaits the frame it applied at — the same shape
the TypeScript client has spoken since M7 (`input(events)`). No wire
change: the batch rides the `Input` domain byte-for-byte as M0/M7
blessed it; the schema, the golden vectors and the variant tables are
untouched. One budget row (`client_input`, the inject + rendezvous pump
round trip over the in-process door).

---

## 1. Why now

The §9 story has two halves on the wire: synthesis (`Gesture`, sugared
by m48 — the driver builds the batch from its per-adapter pacing table)
and raw injection (`Input { events }` — the client owns every primitive,
in order). The m48 record drew the boundary precisely: "the raw
`RequestV1::Input` batch stays the un-synthesized escape hatch, exactly
where M7 left it." Every other surface speaks the raw half — the
TypeScript client's `input(events)` — but the Rust seam, the mirror
every downstream implementer sees, stopped at the raw `RequestV1::Input`
arm: a remote Rust suite needing raw fidelity (a driver-level regression
test, an IME probe, a wheel sequence the pacing table does not model)
hand-built the request and destructured `ResponseV1::InputQueued` by
field (the exact shape `input_gestures.rs`'
`raw_input_request_returns_input_queued` has carried since M7). The
m51 rule — take the wire's own vocabulary, no parallel client-side
types — was applied to the clock, mock, inspect and video surfaces; the
input row is the same promise on the §9 raw half, completing the §9
input half on the typed seam (synthesis m48 + raw m56).

## 2. Design: raw fidelity, as sugar

* **One method, the raw batch.** `input(events: Vec<InputEventV1>)` —
  the client hand-builds the batch from the wire's own §9 vocabulary
  (pointer/keyboard/wheel/IME/window primitives); the sugar adds no
  synthesis, no pacing, no re-ordering. The `GestureV1` row (m48) is the
  fidelity-soft counterpart; the pairing is now symmetric and typed on
  both halves.
* **The carrier, not the variant.** `InputReceipt { frame_seq }` carries
  the `InputQueued` payload as a named client-side type; never serialized
  itself, so the schema stays byte-pinned (the `ActionReceipt` rule).
  The frame seq is the §15.4 rendezvous — a client can await exactly
  that frame, the same affordance the action/gesture receipts carry.
* **The honesty is pinned as shipped.** The empty batch is an honest
  no-op that still pumps (the session's own rule): the receipt names the
  frame the pump applied at, never a synthesized sentinel — the wire
  tests pin it. The connection's window scope (§16 v0.4) steers the
  batch the way it steers gesture coordinates.
* **No wire change and no new capability flags.** The batch rides the
  `Input` domain byte-for-byte; every pre-M0-era door answer (the typed
  `Unsupported` for a transport that rejects it) surfaces verbatim.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::input` and the
  `InputReceipt` type.
- `crates/guiwright-driver/tests/wire_client_input.rs` — the
  conversations: a pointer batch queueing against a frame rendezvous
  (and the next batch landing on a later frame), and the empty batch's
  honest no-op pump.
- `crates/guiwright-driver/examples/wire_client_input.rs` — the consumer
  example over the in-process door (the pointer receipt, the empty
  batch's no-op pump).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_input` row.
- `docs/architecture.md` + `README.md` — the §15.7 row count (58), the
  parity-map M56 sentence, the status-table M56 row and the records
  range.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_input` — 2/2 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named input` ×3 — the m51/m55 record's red
  shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_input` — the demo
  prints the pointer batch's receipt (`applied at frame 4`) and the
  empty batch's no-op pump (`applied at frame 6`).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (the round trip rides the mock adapter's rendezvous pump, the
  µs-scale floor the gesture row shares).

## 5. Remaining gaps (honest list)

- No window-addressed overload (`input_on(window, events)`): the wire
  domain carries no window field — the connection's window scope steers
  the target (§16 v0.4), the same rule the gesture row rides; a
  multi-window suite aims the connection first (the REPL's `window use`
  row).
- The REPL's keyboard primitives ride the gesture vocabulary
  (`codegen_event_call`'s split, §13 over §9) and the TS helper is an
  unchanged surface the sugar mirrors; the goldens and the schema are
  untouched byte-for-byte.
