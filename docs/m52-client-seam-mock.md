# M52 — The client seam's mock row (`ClientConn::mock`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`MockReceipt`] carrier — the §14 state-seam half as typed sugar, the
one-milestone-per-surface pattern (m46→…→m51) carried to the mock
domain, the §14 half the clock row (m51) left as the wire's raw arm. A
downstream Rust consumer installs a canned override, clears it, calls a
registered seam and resets app state with one call taking the wire's own
`MockV1` enum and answering the id + detail the session produced — the
same vocabulary the TypeScript client has spoken since M3 (`mock()` /
`callSeam()` / `resetState()`) and the REPL since M4 (`mock install` /
`clear` / `call` / `reset`). No wire change: the command rides the
`Mock` domain byte-for-byte as §14 blessed it; the schema, the golden
vectors and the variant tables are untouched. One budget row
(`client_mock`, the seam call round trip over the in-process door).

---

## 1. Why now

The §14 story has two halves on the wire: deterministic time (`Clock`,
sugared by m51) and state seams (`Mock` — install / clear / call /
reset-state). Every other surface speaks the second half — the kit's
`app.mock()` handle, the REPL's `mock` rows, the TypeScript client's
`mock()` / `callSeam()` / `resetState()` — but the Rust seam, the mirror
every downstream implementer sees, stopped at the raw `RequestV1::Mock`
arm: a remote Rust suite wanting fixture parity with Playwright contexts
hand-built the request and destructured `ResponseV1::MockDone` by field
(the exact shape `remote_session.rs`'s spike test has carried since the
mock domain landed). The m51 rule — take the wire's own enum, no
parallel client-side vocabulary — was applied to the clock surface; the
mock row is the same promise on the §14 state-seam half, completing
§14's two domains on the typed seam.

## 2. Design: the §14 seam service, as sugar

* **One method, the whole §14 mock vocabulary.** `mock(cmd: MockV1)`
  takes the wire's own enum (install / clear / call / reset-state) — no
  parallel client-side vocabulary to keep in sync, the same rule the
  m48 sugar set for `GestureV1` and the m51 sugar set for `ClockV1`.
* **The carrier, not the variant.** `MockReceipt { id, detail }` carries
  the `MockDone` payload as a named client-side type; never serialized
  itself, so the schema stays byte-pinned (the `ActionReceipt` rule).
  The wire reuses the `id` slot for the fixture scope on `ResetState`
  replies (there is no seam id to name — the session ran its configured
  reset seam); the carrier surfaces the field verbatim and its rustdoc
  names the reuse, so a consumer renders `mock seed-42: …` exactly as
  the REPL's rows do instead of guessing at the field.
* **The honesty is pinned as shipped.** Install/clear are bookkeeping on
  the session's override table; a `call` runs the registered hook inside
  the adapter's runner context (§14 "state writes need the runner's
  context") and pumps a frame so any UI effect settles; an unknown seam
  id answers the session's typed rejection naming the id and the
  registered set; a hook that fails surfaces its own error text through
  the same typed rejection. The canned override wins over the real hook
  exactly where §14 put it — the sugar surfaces, never re-maps.
* **No wire change and no new capability flags.** The command rides the
  `Mock` domain byte-for-byte; a driver that predates the mock domain
  answers the typed `Unsupported` and the sugar surfaces that rejection
  like every other pre-domain door.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::mock` and the
  `MockReceipt` type.
- `crates/guiwright-driver/tests/wire_client_mock.rs` — the
  conversations: the canned override winning until cleared (install →
  reset → clear → reset), the runner-context call, the fixture-scope
  reset (the reply naming the scope), the unknown-seam typed rejection,
  and the failing hook's surfaced error.
- `crates/guiwright-driver/examples/wire_client_mock.rs` — the consumer
  example over the in-process door (install, the canned reset, clear +
  the real hook, the direct call, both typed rejections).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_mock` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_mock` — 5/5 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named mock` ×8 — the m51 record's red shape),
  passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_mock` — the demo
  prints the install ack, the canned reset (`seed-42: canned`), the
  clear + real-hook reset (`state reset to seed-42`), the direct call
  (`3 users seeded`), and both typed rejections (the unknown id naming
  the registered set; `db offline` riding the rejection verbatim).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (median ≈ 20 µs against the 50 µs design target, the runner
  context attached — `run_in` is loud without one, the mock's own
  contract).

## 5. Remaining gaps (honest list)

- No convenience methods (`call_seam(id, scope)` / `reset_state(scope)`):
  the TS client keeps `callSeam()` / `resetState()` on top of its raw
  `mock()` and the kit keeps the `app.mock()` handle; the Rust seam stays
  at the vocabulary level until a consumer asks (the same scope rule the
  m48/m51 sugars applied).
- The REPL's `mock` rows and the TS helpers are unchanged surfaces the
  sugar mirrors; the goldens and the schema are untouched byte-for-byte.
- The mock adapter is the §14 reference semantics for `run_in`; real
  framework adapters execute the same compiled task behind the same
  seam — nothing in this sugar touches the adapter contract.
