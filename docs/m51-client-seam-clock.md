# M51 — The client seam's clock row (`ClientConn::clock`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`ClockState`] carrier — the §14 virtual-clock half as typed sugar, the
one-milestone-per-surface pattern (m46→m47→m48→m49→m50) carried to
deterministic time. A downstream Rust consumer installs, fast-forwards,
pauses, resumes, sets and probes the virtual clock with one call taking
the wire's own `ClockV1` enum and answering the full state — the same
vocabulary the TypeScript client has spoken since §14 (`clock()` /
`fastForward()`) and the REPL since M4 (`clock install` / `ff` / `runfor`
/ `pause` / `resume` / `set` / `state`). No wire change: the command
rides the `Clock` domain byte-for-byte as §14 blessed it; the schema, the
golden vectors and the variant tables are untouched. One budget row
(`client_clock`, the fast-forward round trip over the in-process door).

---

## 1. Why now

The §14 deterministic-time story is one of the crate's headline promises
("the virtual clock makes time-dependent UI reproducible" — the README's
own words), and three surfaces speak it: the kit's clock handle
(`app.clock().fast_forward(dt)`), the REPL's `clock` family, and the
TypeScript client's `clock()` / `fastForward()` helpers. The Rust seam —
the mirror every downstream implementer sees — stopped at the raw
`RequestV1::Clock` arm, so a remote Rust suite wanting deterministic time
hand-built the request and destructured `ResponseV1::ClockState` by
variant (the exact shape `crates/guiwright-driver/examples/wire_clock.rs`
has carried since the clock domain landed). The m48 rule — take the
wire's own enum, no parallel client-side vocabulary — was applied to the
gesture surface; the clock row is the same promise on the §14 half.

## 2. Design: the §14 pacing service, as sugar

* **One method, the whole §14 vocabulary.** `clock(cmd: ClockV1)` takes
  the wire's own clock enum (install / fast-forward / run-for / pause /
  resume / set-time / state) — no parallel client-side vocabulary to keep
  in sync, the same rule the m48 sugar set for `GestureV1` and the m47
  sugar set for `SemActionV1`.
* **The carrier, not the variant.** `ClockState { virtual_time_ms, paused,
  steps }` carries the `ClockState` payload (virtual time in ms — `None`
  while the adapter still runs real time — plus the frame-sized jumps the
  servicing command applied) as a named client-side type; never serialized
  itself, so the schema stays byte-pinned (the `ActionReceipt` rule). The
  jumps ride the §14 ledger, so traces replay timing (§12 `clock.bin`) —
  the sugar changes nothing about that recording.
* **The chaining contract is the affordance.** Every reply carries the
  full state, so a client chains the next command off the state it just
  produced — the doc comment names it, the example demonstrates it, and
  the conversations pin it (set-time lands exactly; the probe echoes it
  on the next round trip).
* **The honesty is pinned as shipped.** The probe is total (a read, never
  a mutation: `virtual_time_ms` reads `None` before the clock is
  installed); a `run_for` on a paused clock answers the session's typed
  `ClockPaused` rejection (run-for paces real time — the §14 rule the
  session enforces, surfaced not re-mapped). No wire change: the command
  rides the `Clock` domain byte-for-byte as §14 blessed it.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::clock` and the
  `ClockState` type.
- `crates/guiwright-driver/tests/wire_client_clock.rs` — the
  conversations: the pre-install probe (virtual `None`, not paused), the
  install + fast-forward pair (the whole budget landing on the virtual
  clock, the frame-sized steps reported), the pause/resume flag flips
  echoing through the state, and the set-time + probe chain (the
  chaining contract).
- `crates/guiwright-driver/examples/wire_client_clock.rs` — the consumer
  example over the in-process door (install, fast-forward with its steps,
  pause/resume, the set + probe chain).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_clock` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_clock` — 4/4 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named clock` ×9 — the m35 record's red
  shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_clock` — the demo
  prints the install state (`virtual Some(0) ms`), the fast-forward (`100`
  ms in 6 frame-sized steps), the pause/resume flips and the set + probe
  chain (`Some(120000)` echoed).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the
  new row bound (the copies-match check green); `check_budgets` reports
  the row OK (median ≈ 21 µs against the 50 µs design target).

## 5. Remaining gaps (honest list)

- No convenience methods (`fast_forward(ms)` / `install()`): the TS
  client keeps `fastForward()` on top of its raw `clock()` and the kit
  keeps the clock handle; the Rust seam stays at the vocabulary level
  until a consumer asks (the same scope rule the m48 sugar applied).
- The mock adapter's clock is the §14 reference semantics; real
  framework adapters install their own time sources behind the same
  `ClockCmd` seam — nothing in this sugar touches the adapter contract.
- No TypeScript change: `clock()` / `fastForward()` are the §14 surfaces
  the sugar mirrors; the goldens and the schema are untouched
  byte-for-byte.
