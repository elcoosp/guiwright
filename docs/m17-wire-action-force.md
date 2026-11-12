# M17 — The wire `Action` honors `force` and `timeout_ms`: the pre-resolved actionability gate

**Status:** implemented (this milestone cashes the sharpest item of the
M16-era micro-debt audit: `Session::apply_action` carried a bare
`let _ = (force, timeout_ms);` — the GWP `Action` request advertised two
knobs the wire path silently discarded. The pre-resolved path now
re-checks actionability at act time, bounded by `timeout_ms`, with
`force` skipping exactly the checks the embedded `ActionOpts` contract
says it skips — plus a budget row, wire tests and docs).

---

## 1. Why the gate, and why now

The wire's `Action` request is the §7 shape a client sends after a
`Resolve`: node handle + generation + action + `force` + `timeout_ms`.
The generation guard ran at the boundary (§8.2 ABA), but between the
Resolve and the Action the tree may have moved on — the button hid, the
widget disabled itself, an overlay landed on top. `perform_action` (the
embedded §8.3–§8.4 path) handles exactly this with its checks + deadline
loop; `apply_action` (the wire path) answered with
`let _ = (force, timeout_ms);` and clicked whatever it was handed. A
client could Resolve a visible button, receive an `Action` ack for a
button that had since been hidden, and never learn the click landed on
nothing. The knobs existed on the wire, the schema, and the docs (§8.2
"auto-wait over the wire") — only the implementation disagreed. That is
the worst kind of API debt: a misleading parameter pair on a public
protocol.

The M16 audit flagged the line as the repo's sharpest micro-debt item;
M17 makes it behavior instead of deleting it, because the wire semantics
were already documented — this milestone makes the code honest with its
own contract.

## 2. Design: `wait_for`'s discipline for a pinned identity

The gate lives in `crates/guiwright-driver/src/session.rs`
(`wait_key_actionable` + `eval_key_checks`). The load-bearing decision:
the pre-resolved path checks a **fixed `NodeKey`**, not a selector —
the node identity is already pinned and generation-guarded, so there is
nothing to re-resolve, only a world to re-read. Consequences:

- **`wait_for`'s cadence, minus the re-query.** The initial evaluation
  is free (many gates pass without pumping at all — at `timeout_ms: 0`
  an actionable node still acts); after that, one evaluation per pump,
  and a pump is a frame boundary (§15.4). No sleep-poll loop: the same
  zero-polling contract as the selector waiter, documented at the gate.
- **The `Stable` check rides the real tracker.** `StabilityTracker` —
  bounds identical across 2 consecutive frame boundaries — was private
  to `guiwright-core::wait`; M17 publishes it (the waiter keeps using it
  unchanged) so the gate speaks the exact §8.3 semantics instead of an
  approximation.
- **`Timeout` names what the node was stuck on.** The wire error is a
  `CoreError::Timeout` whose operation reads
  `actionable action on <key> — stuck on visible (node has no bounds
  yet)` — the Playwright-style failure report the error taxonomy
  promises (§8.3), instead of an opaque timeout.
- **`force` means what `ActionOpts` says.** `action_checks(force)` is
  the single vocabulary: force skips `Stable` + `ReceivesEvents`, and
  keeps `Attached` + `Visible` + `Enabled`. Force is not a license
  through hidden or disabled controls — pinned on the wire by
  `force_still_refuses_disabled_and_hidden_nodes`.
- **The embedded path is untouched.** `perform_action` keeps its
  selector-driven loop; the gate is the wire path's twin, not a
  refactor of both.

## 3. Surface

- `guiwright-driver`: `Session::apply_action` honors `force` +
  `timeout_ms` (the `let _ = (…)` debt is gone); new private
  `wait_key_actionable` / `eval_key_checks` + the `GateEval` summary.
- `guiwright-core`: `wait::StabilityTracker` published (struct + 
  `on_frame`), re-exported at the crate root.
- `guiwright-bench`: the `wire_action_click` budget row (canonical
  `bench/budgets.toml` + the in-crate copy) — the force-less action
  path (gate + act + pump) measured over the M16 embedded door, resolve
  paid once outside the window.

## 4. Tests and gates

- `guiwright-driver`: `tests/wire_action_opts.rs` — four wire tests
  over the embedded door:
  - `action_without_force_refuses_a_hidden_node` (the fixture's
    `Invisible` button): `Timeout` inside `timeout_ms`, message names
    `stuck on visible`, and the wait actually waited;
  - `action_without_force_waits_out_the_gap`: bounds scripted away
    before the Resolve and restored mid-wait — the gate pumps the world
    back and the click lands without force;
  - `action_with_force_skips_the_hit_test_gate`: an overlay group
    occludes the Save button — without force the request refuses stuck
    on `receives events`, with force it clicks through;
  - `force_still_refuses_disabled_and_hidden_nodes`: the fixture's
    disabled `Confirm` button refuses both with and without force
    (`Enabled` is not a force-skipped check).
- The first test was written red and failed exactly as the debt
  predicted (`expected a Timeout error, got ActionDone`) before the
  gate landed.
- Budgets: `wire_action_click` bound in `check_budgets` (release) —
  19.3 µs p50 against an 800 µs design row.

## 5. Remaining gaps (honest list)

- **`Apply` on a detached node** — a node that vanishes mid-wait times
  out (`Attached` stays pending) rather than failing fast with
  `StaleNode`; a fast-path `UnknownNode`/`StaleNode` refusal is a
  plausible refinement if wire clients report the wait as wasteful.
- **`Input` gestures do not ride the gate** — the raw §9 batch is
  coordinate-driven and stays outside the actionability vocabulary by
  design; a client that wants the gate resolves + acts semantically.
- **The schema is untouched** — `force`/`timeout_ms` existed on the
  wire since the M0 shape; no discriminant moves, no goldens re-blessed.
