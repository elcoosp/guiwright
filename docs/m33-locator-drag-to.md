# M33 — The locator drag (`Locator::drag_to`)

**Status:** implemented. The m19 parity stream's named drag item — the
Playwright `locator.dragTo(target)` shape on the kit's borrow-and-act
doctrine, riding the M7 gesture synthesis unchanged: two gated
resolves, one gesture, one budget row.

---

## 1. Why now

The M19 locator-action pass carried `press`/`check`/`uncheck`/
`set_checked`/`focus`/`blur` and deliberately left `drag` out: the
gesture vocabulary already had `Gesture::Drag` over the wire (M7) and
the kit's raw `mouse().drag(from, to)` surface, but the locator-shaped
item — resolve two *locators*, drag between their centers — had no
home. Every framework's dnd/reorder suite asks for it by name
(Playwright's `dragTo`); the raw coordinate form forces every suite to
hard-code pixel positions, which the locator doctrine exists to
prevent. This milestone closes the action row: `drag_to` is the last
member of the §16 actions vocabulary the kit surface lacked.

## 2. Design: one gesture, one coordinate space, the borrow rule made honest

Two constraints shaped the contract:

* **The borrow rule.** Every locator holds the `&mut App` it acts
  through (the M26 `all()` doctrine — two locator values would be two
  mutable borrows of one app). So the target cannot be a locator
  *value*; the kit's honest answer is a **`Selector`** — the aim
  resolves in the source locator's own window context, which is also
  the semantically correct constraint: **a drag is one gesture in one
  window's coordinate space**, so a cross-window drag has no
  coordinate answer here by construction. The aim resolves inside the
  source's subtree (its pin, §16 v0.4) or it never arrives — the
  auto-wait times out naming the aim, exactly like every locator
  action, and never hits across windows.
* **The gate.** Each end waits `Attached` on the zero-polling loop —
  the wire `Resolve` discipline — then geometry is the next line's
  contract: the drag steers by the resolved bounds' centers, so a node
  without bounds is the honest §10 error ("no bounds — nowhere to
  drag"), not an empty drag. The deeper visibility/stability gates
  stay with the raw pointer surface (`mouse.drag`), which gates
  nothing and takes coordinates; `drag_to` adds the locator layer's
  arrival wait and identity, nothing more.

The drag itself rides the M7 synthesis unchanged: one
`Gesture::Drag` through `Session::perform_gesture` — the adapter's
pacing table ratchets the pointer path (§9, ~5 px/tick), one
rendezvous pump applies it, and the outcome's detail names both
centers. No wire change, no session change, no core change.

* **Records (§13 over §9).** The applied gesture records exactly like
  the raw-input surface: the raw `app.mouse().drag(...)` kit line
  while a codegen recording runs (rendered before the batch moves,
  landed after the pump confirms it — the M7 rule), plus one
  `drag_to` trace event naming both ends' selectors. A failed aim
  synthesizes no input and records nothing.

## 3. Surface

- `crates/guiwright-kit/src/app.rs` — `Locator::drag_to(target:
  Selector)` + the `drag_center` helper (the bounds-midpoint read
  with the §10 error shape).

## 4. Tests

- `crates/guiwright-kit/tests/drag_to.rs` — the pointer path end to
  end on the mock (press at the source's center, release at the
  target's, the §9 ratchet's step count pinned), the late-target
  auto-wait (the scripted 100 ms arrival completes the wait — no
  polling), the window-pinned drag resolving both ends inside the
  subtree (and the Preview-only aim timing out, never hitting across),
  strict mode intact on both ends (the unscoped two-"Save" drag fails
  fast, no gesture), the no-bounds §10 error, the §13-over-§9
  codegen line, and the no-events-on-failed-aim rule.
- `crates/guiwright-kit/examples/drag_to.rs` — the consumer example:
  the embedded drag with its synthesized path, the recorded kit code,
  and the window-pinned aim on the two-window fixture.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `drag_to` row (sample: one locator drag on the button fixture —
  two gated resolves + synthesis + one pump), enforced by the CI
  guard.

## 5. Remaining gaps (honest list)

- No `drag_to` on the wire or the REPL: the gesture domain already
  carries `Gesture::Drag` (M7) and the REPL speaks `mouse drag`; a
  locator-pair wire domain would repeat the set-read debate (M30) for
  a two-endpoint case no suite has asked for.
- No `target_position` / `source_position` offsets (Playwright's
  optional drag-point options): the centers-plus-pacing synthesis
  covers the recorded suites so far; offsets are a one-line widening
  of `drag_center` when a suite asks.
- The gate is `Attached`, not the full actionability set: the raw
  `mouse.drag` surface remains the stability/hit-test path; if a
  suite needs a gated locator drag, the check list widens through the
  same `wait_for` call.
- Cross-window drags stay impossible by construction (one coordinate
  space); a suite that truly needs one drives two windows' raw
  surfaces — no fixture has asked.
