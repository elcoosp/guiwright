# M25 — The count comparators

**Status:** implemented (the §16 assertions row's comparator half: the
M22 record reserved the one-sided count comparators as "a vocabulary
widening on the same waiter" — this is that widening, on the same
waiter, with no new machinery).

---

## 1. Why now

M22 shipped the set-size escape hatch as an *exact* comparison:
`to_have_count(n)` holds only on the precise number. Real suites want
the one-sided forms — "at least one row appeared" (`> 0`), "the list
finished shrinking" (`< n`), "more than one dialog is up" (`> 1`) — and
the m22 record explicitly reserved them instead of widening the
surface speculatively. The count waiter's M24-era shape (a `Box<dyn
Waiter>` behind the `Expectation` dispatcher, composable through the
`NotWaiter` decorator) made the widening a one-enum change: the
comparators are new *vocabulary*, not new machinery.

## 2. Design: one waiter, one enum, three comparisons

The load-bearing decision: **the comparison is data, not a method
family**. `CountWaiter`'s `expected: usize` became a
[`CountMatch`] — `Exact(n)` / `GreaterThan(n)` / `LessThan(n)` — with
`holds(found)` as the single comparison point. `CountWaiter::new`
keeps its signature (exact), `with_match` is the comparator-shaped
constructor, and every derived surface follows mechanically:

- `describe()` renders the comparison (`count 4`, `count > 3`,
  `count < 5`) — the exact form stays byte-identical with the M22
  header, so existing timeout errors do not move;
- `stuck_on()` names the found count whenever the comparison does not
  hold (`found 4`), the M22 convention unchanged;
- the mismatch stays waitable, never fail-fast — nodes stream in and
  out, exactly what a comparator waits on;
- the M24 decorator inverts the comparison for free
  (`not().to_have_count_greater_than(3)` is satisfied while the set
  sits at or below the bound) — the composition the decorator was
  built for;
- window pins scope the set before the comparison, exactly as the
  exact count does.

No wire, session or REPL surface changes: `Session::wait_count`
delegates to the generalized `wait_count_match`, and the builder gains
the two terminal methods. The remote/TypeScript vocabulary stays the
same deferred-half follow-on as every embedded assertion milestone.

## 3. Surface

- `crates/guiwright-core/src/wait.rs` — `CountMatch` (the comparison
  enum: `holds`, `describe`, `bound`) and the `CountWaiter`
  generalization (`with_match`, `comparison()`, `expected()` returning
  the comparator's bound).
- `crates/guiwright-core/src/expect.rs` — the shared
  `count_expectation` assembly; `ExpectBuilder::to_have_count` now
  delegates through it (byte-identical headers) and the two comparator
  methods ride the same path.
- `crates/guiwright-driver/src/session.rs` — `Session::wait_count_match`
  / `wait_not_count_match` (the comparator under the M24 decorator)
  and the session builder's `to_have_count_greater_than` /
  `to_have_count_less_than` through the shared `run_count` (window
  scope resolved at act time, as before).
- `crates/guiwright-kit/src/app.rs` — the `expect` surface's two
  comparator pass-throughs.

## 4. Tests

- `crates/guiwright-core/src/wait.rs` (inline) — the comparison table:
  greater-than above/below the bound, less-than below/above, zero
  matches below any positive bound, `stuck_on`/`expected` through the
  comparator, and the byte-identical exact description.
- `crates/guiwright-core/src/expect.rs` (inline) — the builder
  surface: both comparators passing, the timeout naming
  `count > 99` / `count < 3` and `found 4`, the exact header staying
  byte-identical through the shared path, and the negated composition
  in both directions.
- `crates/guiwright-driver/src/session.rs` (inline) — the session-loop
  surface: comparators on the loop, the timeout shape, and the negated
  comparator (`not (count > 3 …)`) timing out.
- `crates/guiwright-kit/tests/count_comparators.rs` — end-to-end on the
  mock adapter: immediate comparators, the widening wait completing on
  a scripted late button, the narrowing wait on a scripted removal, the
  timeout shape, the M24 composition, and window-scoped comparators
  through `app.window(handle)`.
- `crates/guiwright-kit/examples/count_comparators.rs` — the consumer
  example: immediate comparators, a scripted add and removal completing
  event-driven in both directions, the negated composition, and the
  timeout shape printed (`count > 99 — getByRole(Button) (found 3)`).
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `count_compare` row (sample: one `to_have_count_greater_than(3)`
  on the button fixture, riding `resolve_cached_index`), enforced by
  the CI guard.

## 5. Remaining gaps (honest list)

- No wire domain / TypeScript row / REPL command — the standing
  deferred-half of every embedded assertion milestone (M20–M24); count
  over GWP would ride a `Resolve` reply extension, no protocol change.
- The comparator vocabulary stops at the one-sided pair the M22 record
  reserved: `at_least`/`at_most` (inclusive forms) and a
  range/between shape stay unreserved — add them only when a suite
  asks, on the same `CountMatch`.
- `locator.all()` (enumerate the matches as locators) remains the
  other open m22 gap — a kit borrow-shape design question, untouched
  here.
