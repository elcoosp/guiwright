# M45 — The TypeScript wait row (`GwpClient.waitFor(selector, state, timeoutMs)`)

**Status:** implemented. One method on the TypeScript client
([`GwpClient.waitFor`](clients/typescript/src/client.ts)) plus the pure
row-set evaluator ([`wait.ts`](clients/typescript/src/wait.ts)) — the
m42/m43 records' named follow-up ("the TS `waitFor()` rides the same
composition — subscribe / `describeAll()` / `nextEvent` — and lands on
its own milestone when a remote TS suite asks"), closed exactly in the
m40/m41 one-milestone-per-surface pattern. A remote TypeScript suite
waits attached / visible / hidden / stable over any door — TCP,
WebSocket, whatever a downstream `GwpClient.over()` transport speaks —
the same composition the Rust [`ClientConn::wait_for`] rides since
m42. No wire change: the wait composes `subscribe`, `describeAll` and
the event channel byte-for-byte as M12/M39 blessed them; the schema,
the golden vectors and the variant tables are untouched. No budget row
(the m43/m44 rule: client-side composition over the already-budgeted
`client_wait_satisfied` / `describe_all_wire` conversations — a second
row would measure the same round trips twice).

---

## 1. Why now

M42 gave the Rust client seam the wait; M43 put it at the REPL; the
TypeScript client — the §16 other-language surface, carrying the same
vocabulary since M3 — stopped at the snapshot reads (M30/M32/M36/M39/
M41). The m43 record's honest list carried it open as its own
milestone; a remote TS suite watching a late banner had the same
hand-rolled `setTimeout` + re-read loop the Rust side had before m42 —
the exact anti-pattern §15.3 exists to prevent.

## 2. Design: the Rust seam's shape, TypeScript's idiom

* **The evaluator is pure and transport-blind.** `wait.ts` holds
  `WaitState` (the four names), `Eval` (the satisfying row or the
  pending reason) and `WaitTracker` — a row-for-row mirror of
  `crates/guiwright-proto/src/wait.rs`, its whole state machine pinned
  in `test/wait.test.ts` the way the Rust tracker is pinned in
  `proto/tests/wait_tracker.rs`. A *visible* row is one whose bounds
  are present with area > 0 (the §8.3 wire shape); `hidden` is
  satisfied by absence (Playwright parity); `stable` mirrors the core
  `StabilityTracker` (frame-fed only, identical bounds across two
  consecutive presented frames, reset on identity change §8.2 and on a
  changed rect).
* **The answers are handles.** The positive states resolve with the
  row that satisfied them — directly usable as an `action()` node (the
  dump-row rule, M11). `hidden` resolves `null`: absence is the
  satisfying fact. A spent budget rejects with the typed
  `GwpWaitTimeout`, naming the state, the locator (the kit's render
  vocabulary — `getByRole(Button, "Save")`, fuzzy matches saying so —
  mirrored from the core `Selector::render`) and the last evaluation.
* **Fail fast, not degrade.** A driver that predates M39 rejects
  `describeAll` with the typed `Unsupported`; the wait surfaces that
  rejection rather than degrading — the m41 rule, the Rust seam's
  rule, unchanged on the TS side.
* **The deadline races the event read harmlessly.** The Rust side's
  expiry was the m42 record's honest list (closed by m44's buffered
  read half); the TS client's `FrameReader` always buffered, so its
  `nextEventWithin` disarm-and-resume is safe by construction — the
  spent deadline disarms the event waiter, a later event lands in the
  buffer, and the connection keeps serving (the expiry test drives a
  `ping()` on the same door to pin it).
* **Subscription ownership.** The wait installs the
  `tree_changed` + `frame_presented` filter for its duration and
  drops it on return: on resolution the unsubscribe result
  propagates; on expiry the cleanup is best-effort (the Rust
  `expire()` shape — the timeout outcome is the answer, a failed
  cleanup surfaces at the next conversation).
* **Micro-debt rider:** the pushed-event queue bound — a bare `64`
  inline since M12 — is now the named `EVENT_QUEUE_CAPACITY` export
  (§15.6's number, matching the Rust side's constant), so the client
  and the driver agree on one bound in one place.

## 3. Surface

- `clients/typescript/src/wait.ts` — the new module: `WaitState`,
  `Eval`, `WaitTracker`, `renderSelector` (the kit's vocabulary for
  the timeout line).
- `clients/typescript/src/client.ts` — the `waitFor()` method (the
  composition), the typed `GwpWaitTimeout`, the private
  `nextEventWithin` (the deadline race with the disarm contract), and
  the `EVENT_QUEUE_CAPACITY` constant replacing the inline `64`.
- `clients/typescript/test/wait.test.ts` — the evaluator's whole
  state machine without a wire (the `wait_tracker.rs` cases,
  row-for-row).
- `clients/typescript/test/client.test.ts` — the conversations:
  satisfied-on-entry over the blessed M39 vector, the tree-wake
  re-read (empty set → a pushed `TreeChanged` → the re-read answers),
  and the typed expiry leaving the door healthy (a `ping()` on the
  same connection, the `subscribe_off` ack asserted server-side).
- `clients/typescript/scripts/e2e.ts` — the cross-language leg: the
  wait satisfied on entry against the real Rust demo server over TCP,
  the typed expiry naming state/locator, and the door still answering.
- `package.json` — `wait.test.ts` joins the `npm test` suite.

## 4. Tests

- `npm test` — 25/25: the 10 codec + client conversations that
  existed, 12 evaluator cases (attached-by-any-row bounds-less
  included, pending-on-empty, visible's bounds + area guard, hidden's
  absence, stable's baseline/confirm pair, the bounds-change reset,
  the identity-change re-pin, the render vocabulary) and 3 wire
  conversations (satisfied-on-entry byte-pinned to the blessed vector,
  the wake-and-re-read, the typed expiry + post-expiry ping + the
  server-side `subscribe_off` assertion).
- `npm run e2e` — the cross-language conversation against the real
  Rust demo app over TCP (and the M15 WS leg): the wait resolves with
  the laid-out Save row, a never-matching locator expires with the
  typed timeout naming `getByText(…)`, and the same door serves a
  `ping()` afterwards.
- Red→green: the conversation tests failed against the unfixed client
  (`waitFor is not a function` — the m35 record's red shape), passed
  after the seam landed; the evaluator tests were green at first run
  after one render-vocabulary expectation fix (`(fuzzy)` rides inside
  the `getByText(…)` parens, the core render's exact shape).

## 5. Remaining gaps (honest list)

- No wire `WaitFor` domain: the states that need the server's full
  §8.3 evaluator (receives-events, focused, checked) have no
  client-side answer from rows alone; the domain stays an append-only
  option the m39 record's rule reserves for when a suite asks (the
  m42 honest list, unchanged).
- No `waitFor` REPL parity note needed: the interactive half shipped
  as the m43 row; this record closes the last named surface.
- The deadline races the event read per-window (`nextEventWithin`
  disarms on expiry): a protocol error racing the expiry propagates
  from the next conversation, never silently swallowed — the same
  honesty the raw `nextEvent()` has.
