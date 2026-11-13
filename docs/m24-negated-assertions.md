# M24 — The negated assertion surface

**Status:** implemented (the §16 assertions row's negation half: every
`to_*` assertion asserted that a condition *holds* — a test that wanted
to assert the opposite ("this dialog is gone", "this row is not the one
I deleted", "the count has left 4") had no way to say it. Playwright's
`expect(locator).not.to*` modifier was the one assertion-grammar gap
left on the row).

---

## 1. Why now

The assertion engine grew one waiter family per milestone (node checks,
the M20 tri-state, the M22 selector-level count) — every one of them
*positive*. The M22 record's `Box<dyn Waiter>` seam made `Expectation`
a dispatcher over any waiter; a decorator waiter is the composition the
seam was waiting for: negation is not a new check, it is a new way to
combine the existing ones. Playwright's `not` modifier is the parity
row's last missing grammar, and the negative forms users actually need
(`not.toBeVisible` after a close, `not.toHaveCount` while a list
mutates) are exactly the waitable-in-both-directions shapes the engine
already models.

## 2. Design: the first waiter decorator, not a new check family

The load-bearing decision: **negation is a wrapper, not a check**. A
`Check::NotVisible` row would duplicate every existing check's inverse
and would mis-place the semantics — the M20/M22 lesson is that the
waitable unit is the waiter. `NotWaiter` wraps any `Box<dyn Waiter>`
and inverts the *waitable* outcome on every event:

- inner **`Satisfied`** → outer **`Pending`** — the condition the
  assertion rejects still holds, so the negated form keeps waiting;
- inner **`Pending`** → outer **`Satisfied(None)`** — the world does
  not satisfy the positive condition right now, so the negation holds,
  with no node to hand back (the same `Satisfied(None)` shape the
  negated-existence and count assertions use);
- inner **`Failed`** → outer **`Failed`** — fail-fast stays fail-fast.
  A strict-mode violation is a *resolution* failure, not a state
  mismatch, so `not` must not launder it into a pass (Playwright
  parity); and a role that can never expose a checked state cannot
  pass under negation either (the M20 rule).

Because the decorator composes, the whole existing vocabulary is
negated at once — `not().to_be_visible()`, `not().to_be_checked()`,
`not().to_have_count(4)` — including the window-scoped surface (the
pin rides the builder, upstream of the waiter).

- **The timeout header names the inversion** (`not (visible —
  getByRole(Button, "Save"))`): the M17/M22 convention that an error
  names what was being waited for, kept for the negated form. The
  decorator's own `stuck_on` stays the default `None`: the useful
  context ("still visible", "count still 4") is the *inner* waiter's
  satisfied state, which the `Waiter` trait does not expose — extending
  the trait for it stays out of scope, the header already names the
  inversion and the locator.
- **No new session or wire surface** — the driver wraps its waiter
  construction (`wait_not_checks` / `wait_not_count` over the same
  `wait_with` loop, M22's generalization); the remote/TypeScript
  vocabulary stays a follow-on like every embedded assertion milestone.
- **`.not()` is set-once** (idempotent, documented): Playwright's
  modifier is a property, and double negation is not part of the
  surface.

## 3. Surface

- `crates/guiwright-core/src/wait.rs` — `NotWaiter` (the decorator:
  the three-way inversion, the `not (…)` description, the manual
  `Debug` rendering through the inner waiter's description).
- `crates/guiwright-core/src/expect.rs` — `ExpectBuilder::not()` (the
  set-once flag) and the negated wiring of both waiter families; the
  operation header gains the `not (…)` prefix under negation.
- `crates/guiwright-driver/src/session.rs` — the `negated` flag on the
  session builder (`ExpectBuilder::not()`), `Session::wait_not_checks`
  / `wait_not_count` (the decorator over the same zero-polling loop,
  window-scoped and act-time-validated like their positive twins).
- `crates/guiwright-kit/src/app.rs` — the `expect` surface's `not()`
  pass-through (window-pinned locators keep aiming through it).

## 4. Tests

- `crates/guiwright-core/src/wait.rs` (inline) — the inversion table:
  satisfied→pending, pending→satisfied (absence and after a scripted
  world change), the hidden-inner case (`not(hidden)` on an absent node
  keeps waiting), strict mode and the non-checkable role failing under
  negation, the negated count flipping both ways, the description and
  the `Debug` rendering.
- `crates/guiwright-core/src/expect.rs` (inline) — the builder surface:
  `not().to_be_visible()` passing for hidden and absent, the fail-fast
  checked rule, the negated count, the strict-mode failure, and the
  timeout headers naming `not (visible …)` / `not (count 4 …)`.
- `crates/guiwright-driver/src/session.rs` (inline) — the session-loop
  surface: immediate negatives, the negated count, the timeout shapes,
  strict mode, the fail-fast rule, and the negated assertion completing
  on a scripted removal through the shared script channel.
- `crates/guiwright-kit/tests/negated_assertions.rs` — end-to-end on
  the mock adapter: absent/hidden/disabled negatives, the scripted
  removal completing event-driven, the negated count waiting for the
  set to move, strict mode, the fail-fast rule, the timeout shape, and
  window-scoped negation through `app.window(handle)`.
- `crates/guiwright-kit/examples/negated_assertions.rs` — the consumer
  example: immediate negatives, a scripted removal and late button
  completed event-driven, then the two failure shapes printed.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `not_assert` row (sample: one `not().to_be_visible()` on an
  absent locator, riding `resolve_cached_index`), enforced by the CI
  guard.

## 5. Remaining gaps (honest list)

- The negation vocabulary stops at the embedded surface: no wire
  domain, no TypeScript client row, no REPL command — the same
  deferred-half shape M20–M22 shipped with (assertions are client-side
  by design; the wire's `Resolve` auto-waits `Attached` only).
- The decorator's `stuck_on` is the default `None` — naming *what
  still holds* in the negated timeout ("count still 4") wants a
  satisfied-state accessor on the `Waiter` trait; the header names the
  inversion and the locator today.
- `.not()` composes with every current `to_*`; a future
  soft-assertions or polled-expectation surface would need the same
  decorator reviewed against its retry semantics.
