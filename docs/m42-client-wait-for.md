# M42 — The client-side wait (`ClientConn::wait_for`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus a pure row-set evaluator
(`crates/guiwright-proto/src/wait.rs`) — the m21 record's named follow-up
("`waitFor` on the remote/TypeScript path would ride the existing
wait_for-shaped request over GWP; the client vocabulary has no `waitFor`
row yet — a clients-side follow-on, no protocol change"), closed exactly
in the shape the m41 record re-stated ("a client-side `waitFor(state)`
composes the event channel with re-resolves, a different shape than the
snapshot reads"). A remote suite waits attached / visible / hidden /
stable over any door — UDS, TCP, WebSocket or the in-process duplex —
with every [`ClientConn`] impl inheriting the method. No wire change: the
wait composes `Subscribe`, `DescribeAll` and the event channel
byte-for-byte as M12/M39 blessed them; the schema, the golden vectors and
the variant tables are untouched.

---

## 1. Why now

The wire `Resolve` auto-waits attached-only (the §8.2 server-side gate,
`resolve_result`); everything beyond that — will this become visible,
did this leave, has this stopped moving — a remote suite could only
answer by hand-rolled sleep-and-re-read loops, the exact anti-pattern
§15.3 exists to prevent. The embedded kit has driven `wait_for(state)`
since M21; the client vocabulary stopped at snapshot reads (M30/M32/M36/
M39/M41). The m41 record's honest list carried the row open because a
wait is not a snapshot: it needs wakes, and wakes mean the event
channel — a different shape than the M41 derivations, worth its own
milestone.

## 2. Design: the event channel + the describe-all rows, composed

* **The loop.** `wait_for(selector, state, timeout)` subscribes to
  `TreeChanged` + `FramePresented`, evaluates the state against a fresh
  `describe_all(selector)` row set, and races `next_event` against the
  deadline — a tree wake re-decides, a frame wake may confirm `Stable`,
  a `Log` record is never a wake. On exit (satisfied or expired) the
  wait unsubscribes: a consumer that never drains its events must not
  stay subscribed — the driver only pumps at frame cadence while
  subscribers exist (§7, M12). The ownership rule is documented at the
  method: a connection running its own long-lived subscription (the
  viewer's live feed) should not hand the same connection to a wait.
* **The rows are the truth.** Evaluation sees exactly what the M39 walk
  answers: a *visible* row is one whose bounds are present with area >
  0 — the wire shape of the §8.3 visible check (hidden nodes ride the
  non-strict walk but carry `bounds: None`, the fixture's "Invisible"
  button being the pinned case). `Hidden` is satisfied by absence (no
  visible row), Playwright parity. `Attached` is satisfied by any row —
  the attached-vs-visible distinction survives the wire.
* **`Stable` mirrors the core `StabilityTracker` exactly.** Frame-fed
  only (a tree wake re-pins but never confirms), bounds identical across
  two consecutive presented frames, reset on identity change (§8.2: the
  pinned row leaving the set re-pins to the set's head) and on a changed
  rect (the changed frame becomes the new baseline).
* **The answers are handles.** The positive states answer with the row
  that satisfied them — directly usable as a `RequestV1::Action` handle
  (the dump-row rule, M11); `Hidden` answers `Ok(None)` (absence is the
  satisfying fact). The timeout is the typed `GwpError::WaitTimeout`,
  naming the state, the locator (the kit's `getByRole(…)` render) and
  the last evaluation — the failures-name-names discipline over the
  wire. The variant is client-side only (`ErrorCode::Timeout` stays the
  driver's resolve/action answer), so the error vocabulary grew without
  a schema change.
* **Fail fast, not degrade.** A driver that predates M39 rejects
  `DescribeAll` with the typed `Unsupported`; the wait surfaces that
  rejection rather than degrading — the m41 rule (the REPL's per-row
  fallback is the interactive surface's concession, not this one's).

## 3. Surface

- `crates/guiwright-proto/src/wait.rs` — the new module: `WaitState`
  (the kit `WaitState`'s client-side mirror), `Eval` (satisfied row or
  the pending reason that becomes the timeout's last-evaluation line)
  and `WaitTracker` — the transport-blind evaluator, unit-pinned
  without a wire.
- `crates/guiwright-proto/src/client.rs` — the `wait_for` default
  method (every shipped and downstream client inherits it) and the
  `NodeKeyV1` import the `describe` signature now names directly (the
  fully-qualified form it carried since M36 rode the pre-M1 import
  band).
- `crates/guiwright-proto/src/lib.rs` — the module doc row, `pub mod
  wait`, the `WaitState` re-export, the `GwpError::WaitTimeout` variant.
- `crates/guiwright-bench/src/samples.rs` + `bench/budgets.toml` (+ the
  crate copy) — the `client_wait_satisfied` sample and its budget row:
  the satisfied-on-entry floor (subscribe + describe-all evaluation +
  unsubscribe cleanup), no wake and no re-read on this number.
- `crates/guiwright-driver/examples/wire_wait.rs` — the consumer
  example: a shared-script app reveals a "Saved!" banner a beat after
  attach; the client waits `visible`, prints the satisfying handle, and
  walks the `hidden` absence case.

## 4. Tests

- `crates/guiwright-proto/tests/wait_tracker.rs` — the evaluator's whole
  state machine without a transport: attached-by-any-row (bounds-less
  included), pending-on-empty, visible's bounds + area guard, hidden's
  absence, stable's baseline/confirm pair, the bounds-change reset and
  the identity-change re-pin, and the state names.
- `crates/guiwright-driver/tests/wire_wait.rs` — the conversation over
  the in-process door: satisfied-on-entry (the row + its bounds), the
  tree-grows wait through the shared-script channel (event-driven, no
  polling), hidden-by-absence, the typed timeout naming state/locator/
  last-evaluation, the stable wait across presented frames, the
  no-subscription-left-behind cleanup guarantee, and the window-target
  scope (both windows' `Save` answer through the aimed subtree; the
  other window's node expires pending).
- Red→green: both suites failed against the unfixed crate (`could not
  find wait in guiwright_proto`, `variant not found in GwpError`,
  12 wire-test errors), passed 10/10 + 7/7 after the seam landed;
  `check_budgets` binds the new row and exits 0.
- The bench row rides the canonical table + the crate copy (the
  equality test pins them together), 47 rows total.

## 5. Remaining gaps (honest list)

- No TypeScript row: the client-side wait is the Rust seam this pass;
  the TS `waitFor()` rides the same composition (subscribe /
  `describeAll()` / `nextEvent`) and lands on its own milestone when a
  remote TS suite asks — the m40/m41 one-milestone-per-surface pattern.
- The deadline races the event read the way the live feed's timeout
  already does (`live.rs`'s `timeout(next_event)`): a cancel landing
  mid-frame could desync the connection's framing. The honest fix is a
  cancel-safe client read half (a buffered `conn` read), a conn-level
  follow-up this pass does not open.
- No wire `WaitFor` domain: the states that need the server's full §8.3
  evaluator (receives-events, focused, checked) have no client-side
  answer from rows alone; the domain stays an append-only option the
  m39 record's rule reserves for when a suite asks.
