# M22 — The count surface

**Status:** implemented (the §16 assertions row's set half: every
node-level assertion re-resolves through the strict-mode `ChecksWaiter`,
so a test that wanted to talk about the *size* of the match set had
nothing to say — and `resolve_all` had no consumer on the assertion or
kit surface).

---

## 1. Why now

Playwright ships `expect(locator).toHaveCount(n)` and `locator.count()`
as the escape hatch from strict mode: assert (or read) how many nodes
match without narrowing with `.first()`/`.nth()`. Guiwright had the
non-strict resolution path all along — `TreeService::resolve_all`
(document order, scope-aware, the same path filters and the codegen
uniqueness probe ride) — but the assertion engine hard-wired it away:
`Expectation` held a concrete `ChecksWaiter`, and every `to_*` assertion
strict-resolved one node. The vocabulary existed everywhere except where
tests drive it, the same shape M19 and M21 closed.

## 2. Design: the first selector-level waiter, over the same zero-polling loop

The load-bearing decision: **the count is a waiter, not a check**. Checks
are per-node predicates (`actionability::evaluate(tree, key, check)`);
the match-set size is a property of the *selector*, so it rides a new
`CountWaiter` implementing the existing `Waiter` trait — re-resolving
through `resolve_all` on every event and comparing the set size. The
session's wait loop is generalized (`wait_with` over `&mut dyn Waiter`)
so both waiter families share the identical cadence: free initial
evaluation, one consultation per pump-produced event, deadline-bounded —
no new polling, no new session surface.

- **A mismatched count is waitable, never fail-fast** (Playwright
  parity): nodes stream in and out, exactly what the assertion is for.
  Strict mode is never consulted — 4 buttons is a valid answer to
  `to_have_count(4)` where `to_be_visible()` would reject the same tree.
- **The `stuck_on` seam.** The M17 gate embeds what it is stuck on
  (`stuck on visible (node has no bounds yet)`); the count waiter wants
  the same quality bar, so `Waiter` gains a defaulted `stuck_on()` —
  timeout errors read `count 99 — getByRole(Button) (found 4)`. The
  default (`None`) keeps every existing header byte-identical.
- **No new session or wire surface** — embedded client-side work exactly
  like M19/M20/M21; the remote/TypeScript vocabulary stays a follow-on.
- **`locator.count()` is a read, not an action** — like `resolve()`, it
  records no codegen line (resolution without an action is not a
  replayable step); the sync read and the waitable assertion are
  separate methods, as in Playwright.

## 3. Surface

- `crates/guiwright-core/src/wait.rs` — `CountWaiter` (the selector-level
  waiter: `resolve_all` count, `found()`/`expected()` accessors, the
  `count N — {selector}` description) and the defaulted
  `Waiter::stuck_on` seam.
- `crates/guiwright-core/src/expect.rs` — `ExpectBuilder::to_have_count`
  and `Expectation` holding `Box<dyn Waiter>` (the seam that lets
  selector-level assertions ride the builder); `check_now` appends the
  stuck-on context to the Pending timeout, the M17 convention.
- `crates/guiwright-driver/src/session.rs` — the generalized
  `wait_with` loop (the former `wait_for` body), `Session::wait_count`,
  and the session builder's `to_have_count` (window-scoped, act-time
  validated, keeping the count waiter's own header).
- `crates/guiwright-kit/src/app.rs` — `Locator::count()` (the sync,
  window-scoped read over `Session::resolve_all`) and the `expect`
  surface's `to_have_count` pass-through.

## 4. Tests

- `crates/guiwright-core/src/wait.rs` (inline) — the waiter semantics:
  satisfied on the exact set, pending until a late-appearing node moves
  the count, satisfied on zero matches (`Satisfied(None)`), and
  `stuck_on`/`found` naming `found 4` while stuck.
- `crates/guiwright-core/src/expect.rs` (inline) — the whole match set is
  a valid answer (strict mode bypassed), zero counts, and the timeout
  error carrying `count 99`, the locator and `found 4`.
- `crates/guiwright-driver/src/session.rs` (inline) — `wait_count` on the
  session loop (immediate + zero + timeout shape) and the builder
  surface.
- `crates/guiwright-kit/tests/count_assertions.rs` — end-to-end on the
  mock adapter: immediate counts, the count completing on a scripted
  late-appearing diff (in), a scripted removal (out), the timeout naming
  expected count + locator + found, the sync read, window-scoped counts
  through `app.window(handle)`, and `count()` recording no codegen line.
- `crates/guiwright-kit/examples/count_assertions.rs` — the consumer
  example: immediate reads and assertions, a scripted add-then-remove
  pair completed event-driven, and the failure shape printed.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` — the
  `count_assert` row (sample: one `to_have_count(4)` on the button
  fixture, riding `resolve_cached_index`), enforced by the CI guard.

## 5. Remaining gaps (honest list)

- The count vocabulary stops at the embedded surface: no wire domain, no
  TypeScript client row, no REPL command — the same deferred-half shape
  M20/M21 shipped with; `count` over GWP would ride a `Resolve` reply
  extension, no protocol change.
- `to_have_count_greater_than` / `to_have_count_less_than` comparators
  stay out of scope — the exact-match assertion is the parity row; the
  comparators are a vocabulary widening on the same waiter.
- `locator.all()` (enumerate the matches as locators) is a kit surface
  question more than a core one — `resolve_all` already returns the
  full set; the kit locator's borrow-and-act shape wants a design pass
  before exposing it.
