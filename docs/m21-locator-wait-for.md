# M21 — The locator wait-for surface

**Status:** implemented (the §16 auto-waiting row's explicit half: the
actionability machinery has driven every locator action since M0, and the
assertion family covers the `expect(locator).to_*` shape — but a test
that wants to *wait without asserting* had to drop to
`session.wait_for(...)` with a hand-built check list and deadline).

---

## 1. Why now

Playwright ships `locator.waitFor(state)` as the explicit escape hatch
from auto-waiting: wait for the node to be attached / visible / hidden /
stable before continuing, without asserting anything. Guiwright had all
of the machinery — the session's event-driven `wait_for` loop, the §8.3
check table, the M17-published `StabilityTracker` — and none of the DSL
surface. The gap is the same shape M19 closed for actions: the vocabulary
existed everywhere except where tests drive it.

## 2. Design: a state enum over the check table, nothing new underneath

The load-bearing decision: **the session's `wait_for` loop is the whole
implementation**. `WaitState` maps each Playwright state onto its §8.3
check list — `attached` → `[Attached]`, `visible` → `[Visible]`,
`hidden` → `[Hidden]` (absence passes, Playwright parity), `stable` →
`[Visible, Stable]` (visible first, then the two-identical-frames
tracker). The kit method feeds that list to the same zero-polling loop
every assertion and actionability gate uses: free initial evaluation, one
re-evaluation per tree diff or presented frame, deadline-bounded.

- **No new session or wire surface** — the method is client-side work on
  the embedded path, exactly like M19's parity actions and M20's
  assertions.
- **Codegen rides the same tap.** `record_action("wait_for", …)` falls
  through the recorder's generic arm, so the emitted line reads
  `locator.wait_for();` — Playwright's `await locator.waitFor()` shape.
- **The timeout error names the state and the locator**
  (`visible — getByRole(Button, "Ghost" (fuzzy))`), consistent with the
  M20 failure-header rule: errors carry what was waited for and where.

## 3. Surface

- `crates/guiwright-kit/src/wait_for.rs` — the `WaitState` vocabulary
  (`checks()` / `describe()`), `Locator::wait_for(state)` and
  `Locator::wait_for_with_timeout(state, timeout)`. A new surface module
  (the locator struct lives in `app.rs`; its fields are crate-visible so
  the `expect` helper and this module share one access path).
- `crates/guiwright-kit/src/lib.rs` — `pub mod wait_for` and the
  `WaitState` re-export at the crate root (the `App`/`expect` row of the
  public API).

## 4. Tests

- `crates/guiwright-kit/tests/wait_for_state.rs` — the four states
  end-to-end on the mock adapter: the immediate states completing on the
  initial evaluation, `Stable` fed two scripted presented frames, the
  late-appearing node completing on the scripted tree diff (the
  shared-script channel, 100 ms in), the timeout naming state + locator,
  and the recorded codegen line.
- `crates/guiwright-kit/examples/wait_for_states.rs` — the consumer
  example: a button scripted to appear 300 ms in, waited for through the
  event-driven loop, plus the immediate states and the failure shape.

## 5. Remaining gaps (honest list)

- `waitFor` on the remote/TypeScript path would ride the existing
  `wait_for`-shaped request over GWP; the client vocabulary has no
  `waitFor` row yet — a clients-side follow-on, no protocol change.
- `wait_for(state)` with element-scoped filters composing into the
  stable-state tracker across *selector narrowing* changes (`.first()`
  mid-wait) stays out of scope — the waiter resets on identity change
  (§8.2) and that behavior is what the current tests pin.
