# M43 — The REPL wait row (`wait <state> <get-args…>`)

**Status:** implemented. One REPL command over the m42 client-side wait
— the interactive half of the seam, the m35/m40 one-milestone-per-surface
pattern ("the protocol half first, the client rows after"): `wait
attached|visible|hidden|stable <get-args…>` rides
`ClientConn::wait_for` with the `timeout <ms>` budget. No wire change
(the wait composes existing domains byte-for-byte), no session change,
no budget row (the m35/m40 rule restated: the row is client-side
composition over the already-budgeted `client_wait_satisfied` /
`describe_all_wire` conversations — a second row would measure the same
round trips twice).

---

## 1. Why now

M42 gave the client seam the wait; a debugging session at the REPL had
no row for it. The gap was the m41 record's shape again: the embedded
kit has driven `wait_for(state)` since M21, the interactive surface
stops at snapshot reads (`get`/`describe`/`count`/`all`/`texts`/
`values`) — and a REPL user watching a late banner or a moving panel
had the same hand-rolled options the remote suite had: `snapshot`,
re-run, eyeball, repeat. The seam landed first (m42), so the row costs
one command arm plus the `Connection` impl that joins the dispatcher to
the seam.

## 2. Design: the seam's row, the REPL's vocabulary

* **The state word is the kit's vocabulary** — `attached`, `visible`,
  `hidden`, `stable` (the m21 `WaitState` names, lowercased at the
  prompt). Anything else is one error line naming the four, the session
  usable (the survival rule).
* **A positive state answers one ok line naming the satisfying row** —
  `wait visible — accesskit#2 "Save" (generation 0) rect=[16,16 →
  136,48]`, the `get` row's handle + generation + rect form, so the
  printed row aims the action vocabulary directly (the dump-row rule).
  `hidden` answers `wait hidden — satisfied by absence (no visible
  rows)`: absence is the satisfying fact, not an error.
* **A spent budget prints the typed timeout** — `wait visible timed out
  after 150 ms — getByRole(Button, "Invisible" (fuzzy)) (the set's rows
  carry no bounds yet)`: state, locator, last evaluation (the
  failures-name-names discipline at the interactive surface).
* **A read, not a resolve-store** (the m35/m40 rule, unchanged): `wait`
  never moves `last` — `get` → `wait` → `last` still aims the get's
  node.
* **The wait's subscription window restores the `events on` filter**
  (best effort): the m42 wait owns the event filter for its duration
  and drops it on return; the row re-subscribes the `events on` triple
  (tree/frame/log) when the stream was live, so the live-debug flow
  keeps flowing after a wait. A server without the log tail never had a
  live filter to restore — the guard keys on the REPL's own
  `events_subscribed` state, so the unsupported case is a no-op by
  construction.
* **The dispatcher joins the seam.** `Connection` (the CLI's
  transport-agnostic UDS/TCP/WS enum) implements `ClientConn` — the
  m42 sugar lands on every transport at once instead of a per-arm copy,
  the one-impl-for-every-door rule the four shipped clients ride.

## 3. Surface

- `crates/guiwright-cli/src/session.rs` — `impl ClientConn for
  Connection`: handshake/request/next_event/subscribe/unsubscribe/ping
  forwarded per transport arm; the dispatcher is now a seam consumer,
  not a vocabulary of its own.
- `crates/guiwright-cli/src/repl.rs` — the `wait` dispatch arm,
  `wait_command` (the state word validation, the `timeout <ms>` budget,
  the row/absence/timeout renders, the restore), and the command-list
  row.

## 4. Tests

- `crates/guiwright-cli/tests/repl_e2e.rs` —
  `repl_wait_rows_wait_on_the_client_seam` over a real UDS socket (the
  log tail enabled, the `events on` vocabulary's server shape): the
  satisfied row form, the absence answer, the typed timeout naming
  state/locator/last-evaluation at the 150 ms budget, the state-word
  validation, the `events on` stream still flowing after a wait (the
  restore), the read-not-store rule, and the help listing the row.
- Red→green: the suite failed against the unfixed REPL (unknown command
  — the m35 record's red shape), passed after the arm landed;
  re-run 5× for the event-flow timing (the drain is honest about what
  landed while the last command ran, so the test gives the restored
  filter a few frame ticks before its flush assertion).
- Red→green (the seam half): `Connection` had no `ClientConn` impl, so
  the compile itself was the red step; the impl is the green.

## 5. Remaining gaps (honest list)

- No TS row and no wire domain: the m42 record's honest list carries
  both (the TS `waitFor()` is its own milestone; the append-only
  `WaitFor` domain is reserved for evaluator states rows cannot answer).
- The REPL fallback shape has none to keep: a driver that predates M39
  rejects `DescribeAll`, the seam surfaces the typed rejection, and the
  row prints it as the error line — fail fast, the m41 rule, no
  per-row degradation to maintain.
- The restore is best effort by design: a stream that broke mid-wait
  prints the break at the next drain (the m12 warn shape), never
  silently re-arms.
