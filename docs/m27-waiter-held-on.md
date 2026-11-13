# M27 — The waiter satisfied-state accessor (`held_on`)

**Status:** implemented (the m24 record's open gap: "the decorator's
`stuck_on` is the default `None` — naming *what still holds* in the
negated timeout (`count still 4`) wants a satisfied-state accessor on
the `Waiter` trait"). One trait method, three implementations, two
wire-ups — no new waiter, no new machinery.

---

## 1. Why now

M24 shipped the negation decorator and was honest about its timeout
shape: `not (visible — getByRole(…))` names the inversion and the
locator, but not *why* the negation is still pending — which, for a
negated assertion, is always the same reason: the positive form
currently holds. The m24 record reserved the fix as a trait-level
accessor rather than a decorator-internal hack, because the information
lives in the inner waiters (`ChecksWaiter` knows which checks passed;
`CountWaiter` knows the count that holds), not in the decorator. The
M26 `all()` work touched the same seam from the kit side; this closes
the waiter side.

## 2. Design: one accessor, mirrored to `stuck_on`

The load-bearing decision: **the accessor mirrors `stuck_on`** — same
trait, same `Option<String>` shape, opposite tense. `stuck_on` names
what a *pending* waiter is waiting on (`found 4`); `held_on` names what
a *holding* waiter is holding (`visible`, `count 4`). Both default to
`None` — a waiter that cannot name anything leaves the header alone.

- **`ChecksWaiter`** keeps the condition that the last evaluation
  satisfied — the check vocabulary joined the way `describe` renders it
  (`visible`, or `visible, enabled` for a multi-check sequence). The
  state is set at the two satisfaction points (checks complete;
  absence-satisfies-hidden, where the condition that holds IS the
  hidden form) and cleared at the top of every event, so it always
  describes the *last* evaluation.
- **`CountWaiter`** names the count itself when the comparison holds —
  `count {found}` — for the exact form and the comparator forms alike
  (what still holds is the size, whichever side of the bound it stands
  on). Cleared whenever the comparison does not hold or the resolution
  errors.
- **`NotWaiter`** delegates: the decorator pends exactly while the
  inner form holds, so its satisfied state IS the still-holds context
  of the inversion — `self.inner.held_on()`.
- **Core `check_now`** (the single-shot evaluation path) appends
  `(still …)` to the timeout header when `stuck_on` is `None` and
  `held_on` is `Some` — a positive assertion pends because nothing
  holds (the stuck count rides, the M22 shape), a negated one pends
  because something does (`not (visible — …) (still visible)`).
- **Driver `wait_with`** (the session loop) gains the same suffix rule,
  plus an `operation` override: the `ExpectBuilder`'s negated path now
  hands its assertion-shaped header (`not (visible — …)`) *into* the
  loop instead of rewriting the timeout after the fact, so the suffix
  the decorator names survives under the inversion. The positive path
  keeps its post-hoc header rewrite byte-identical (a pending check has
  no stuck/still suffix to preserve), and the count paths keep the
  waiter-owned headers (`count 4 — getByRole(…) (found 4)`) they have
  always carried.

No new waiter, no protocol change: `wait_for`, `wait_count_match` and
`wait_not_count_match` keep their public signatures (they pass no
override — their headers were already assertion-shaped), and only
`wait_not_checks` gains the header parameter (one in-repo caller: the
`ExpectBuilder`'s negated branch).

## 3. Surface

- `crates/guiwright-core/src/wait.rs` — `Waiter::held_on` (the trait
  accessor, rustdoc'd against the m24 record) and the three
  implementations (`ChecksWaiter`'s `held` state +
  `holding_condition()`, `CountWaiter`'s `held` state, `NotWaiter`'s
  delegation).
- `crates/guiwright-core/src/expect.rs` — `Expectation::check_now`'s
  still-clause (the stuck-or-held rule).
- `crates/guiwright-driver/src/session.rs` — `wait_with`'s `operation`
  override + stuck-or-held suffix, `wait_not_checks`'s header
  parameter, and `ExpectBuilder::run` handing the negated header down.

## 4. Tests

- `crates/guiwright-core/src/wait.rs` (inline) — the accessor table:
  checks that pass right now, the absent-hidden form, none-while-
  pending, the count that holds (exact and comparator) and none on a
  mismatch, and the decorator's delegation.
- `crates/guiwright-core/src/expect.rs` (inline) — the timeout shapes:
  a negated `check_now` timeout naming `(still visible)` /
  `(still count 4)`, and a positive timeout carrying no still clause
  (the stuck count rides instead).
- `crates/guiwright-kit/tests/negated_assertions.rs` — end-to-end
  through the session loop: the negated timeout embedding
  `(still visible)` and the negated count timeout embedding
  `(still count 4)` under the `not (…)` inversion header.

## 5. Remaining gaps (honest list)

- No budget row: the accessor is error-path formatting, not a hot
  path — the `auto_wait_settled_frame` row's precedent ("verified by
  assertions, not a timing guard") applies, and the guard exists to
  catch hot-path regressions, not to time string appends.
- The vocabulary stops at naming what holds; a negated timeout does
  not yet re-print the tree summary's per-node detail — the summary
  already rides the error's `last` field (§8.3) for consumers that
  want it.
- A future soft-assertions or polled-expectation surface still needs
  the decorator's retry semantics reviewed against it (the m24 note
  stands, accessor included).
