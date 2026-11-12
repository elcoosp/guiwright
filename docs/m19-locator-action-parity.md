# M19 — The kit locator surface carries the full semantic action vocabulary

**Status:** implemented (this milestone cashes the "schema is ahead of
the embedded DSL" note from the M18-era audit: the wire's
`SemAction::Press/Check/Uncheck/Focus/Blur` were served by the driver
and spoken by the REPL and the TypeScript client, but the kit's
`Locator` — the surface most tests actually drive — had only
`click`/`dblclick`/`fill`/`hover`. The parity methods are now kit-side
work only, exactly as the audit predicted: no wire, no session, no
schema change).

---

## 1. Why now

The §16 actions row has said "MVP" since M0 — true for the semantic
layer (the wire and the session gate carry the whole vocabulary), but
the kit's locator surface stopped at the click-family. A test author
who wanted `check()` on the embedded path had to drop to
`session.perform_action(...)` — hand-building a `SemAction` and
rewriting the codegen + trace recording the locator actions do for
free. The vocabulary existed everywhere except where it is used most.
M19 lands the six missing methods and the DSL stops being a dialect of
the wire vocabulary and becomes a full rendering of it.

## 2. Design: one path, no new machinery

The load-bearing decision: **no new session or wire surface**. Each
method is the existing `perform_action` path with the corresponding
`SemAction` variant — the same actionability gate (§8.3, the M17
refusal semantics included), the same auto-wait, the same codegen
recording (`record_action`) and trace event vocabulary as
`click`/`fill`/`hover`. Consequences:

- **`press(key)` is a semantic action, not a raw chord.** It carries
  `SemAction::Press { key, modifiers: Vec::new() }` — the session
  gate runs first, the adapter's `act` applies the press at a frame
  boundary. The raw per-window chords stay on `app.keyboard()`
  (§9); `locator.press` is the "press this node" form Playwright
  ships. The codegen recorder's existing `press` arm emits
  `locator.press("Enter");` — the M4 remote-codegen shape, now
  reachable from the kit.
- **`set_checked(bool)` routes through `check`/`uncheck`.** No third
  wire variant exists and none is needed — the bool picks the
  semantic action, the detail strings and codegen lines stay in the
  `check`/`uncheck` vocabulary.
- **The detail strings stay readable.** `press` with no modifiers now
  describes as `press(Enter)` (the `+` separator only joins actual
  modifiers) — a driver-side `SemAction::describe` cleanup that came
  out of this milestone's tests; nothing pinned the old wart.

## 3. Surface

- `crates/guiwright-kit/src/app.rs` — `Locator::{press, check,
  uncheck, set_checked, focus, blur}` (consuming, actionability-gated,
  codegen-recorded).
- `crates/guiwright-driver/src/adapter.rs` — `SemAction::describe`
  renders a no-modifier press as `press(Enter)`.
- `crates/guiwright-kit/tests/locator_actions.rs` — the vocabulary
  end-to-end on the mock adapter (every method → the exact
  `SemAction` at the adapter), the gate refusal for a hidden node,
  and the codegen lines.
- `crates/guiwright-kit/examples/locator_actions.rs` — the consumer
  example: all six actions on the fixture app, the codegen output and
  the session trace.

## 4. Remaining gaps (honest list)

- `select_option`, `scroll_into_view_if_needed`, `drag_to`,
  `wait_for(state)` and the content getters (`inner_text`,
  `get_attribute`, `input_value`) stay §16 follow-ons — several need
  vocabulary the wire does not carry yet (option lists, scrollable
  ancestors), which is why they were not part of this milestone's
  "schema already speaks it" scope.
- Checked-state *assertions* (`to_be_checked`) need the checked state
  surfaced through the §8 check vocabulary first (`Check::Checked`
  reading AccessKit's tri-state) — a core-side seam, deliberately not
  squeezed in here.
