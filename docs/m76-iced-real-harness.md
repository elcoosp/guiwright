# M76 — The real iced harness: `SimulatorSeam` bound to `iced_test::Emulator` (the §17 MVP trio's last leg)

**Status:** implemented. The §17 MVP trio (egui + masonry + iced) is
whole: the iced adapter's `SimulatorSeam` is no longer a trait waiting
for a CI pass — [`EmulatorHarness`] binds it to a real
`iced_test::Emulator` running a real iced `Program` headlessly. Interactions
inject real `core::Event`s through `UserInterface::update` (§4.6
"injection = `UserInterface::update(&[core::Event])`"); the semantic tree
comes from the §4.6 read-back (`Task::widget` operation shipping the
laid-out widget walk over an mpsc channel — "operations run on the loop
thread, the channel is the safe boundary"); the CI `adapter-iced` job
runs the harness (`cargo test`, not `cargo check`). **No wire change**:
this milestone moves adapter reality, not protocol rows.

---

## 1. Why now

The m41–m75 stream moved the wire, the clients and the kit while the
adapter matrix stayed where M0 left it: egui + masonry real, iced/freya
shipping seam *traits* with zero framework call sites (`SimulatorSeam`
unbound; `iced_test` named by the feature docs but never imported;
the CI iced job a `cargo check`). Architecture §17's MVP trio — "egui +
Masonry + Iced … exercises all three levels (L1 native, L1
custom-harness, L2 logical-tree)" — has been unfinished at its third
leg since M0: a Level-2 adapter that compiles but drives nothing. This
record pays that debt: the framework type lands in non-test impl code
(`iced_test::Emulator`, `iced_test::instruction::*`), a runtime test
exercises it through [`Session`], and the §4.6 claim "Guiwright wraps
`Emulator`/`Simulator` + selectors instead of re-implementing" becomes
true rather than aspirational.

## 2. Design

* **The Emulator, not the Simulator** — §4.6's embed paragraph is the
  contract: "`iced_test::Emulator` drives a real `Program` (update +
  subscriptions + tasks) headlessly with determinism modes". The
  `Simulator` drives a bare `Element` and rebuilds its renderer per
  construction (`Renderer::new` inside every `with_size`); the
  `Emulator` owns one renderer for its lifetime, rebuilds the UI from
  the program's view per instruction, and restores widget state through
  the layout cache. One `Mode::Zen` settle per instruction proves it
  applied (`Event::Ready`, `pending_tasks` counting) — the §6.1
  rendezvous rides iced's own determinism, not a digest heuristic.
* **The read-back is an operation, not a scrape** — `iced_selector`'s
  candidates are flat (no parent links, no widget-type names), so a
  scrape could not rebuild the tree. The §4.6 live-mode pattern is the
  fit: the app routes one hook message (`HarnessApp::hook`), the default
  [`HarnessApp::run_hook`] returns `Task::widget(TreeDumpOp)`, and the
  operation walks the laid-out UI through the `Operation` hooks
  (`container` / `text_input` / `focusable` / `text` / `scrollable`),
  nesting via `traverse` and closing finished siblings by
  bounds-containment (leaf widgets never traverse). The walk ships
  `WidgetReport`s over the channel; the harness folds them into the
  engine.
* **The renderer backend is explicit** — `iced_test` names no renderer
  backend of its own: with only its default features
  `iced_renderer::Renderer` is the `()` null renderer (zero-bounds text,
  no-op editors, and a release-mode `compile_error!`). The `iced`
  feature therefore enables `iced_renderer/tiny-skia` (+ the pure-Rust
  `wayland`/`x11` presentation backends — without one, softbuffer
  compiles no backend at all and fails to build). CPU rendering keeps
  the harness deterministic with no GPU dependency (§11).
* **Kinds are the app's contribution** (§4.6 "adds role semantics via a
  contributed widget-kind registry") — iced 0.14 passes a button's `Id`
  to *no* operation hook (`Button::operate` fires `container(None)`),
  so hooks alone can't see interactive widgets. Apps wrap their
  interactive widgets in `Id`-carrying containers and answer
  [`HarnessApp::kind_of`]; the walk prefers the app's kind over the
  hook's capability class, and the registry maps it to the AccessKit
  role. The same contract masonry meets with `WidgetTag`.
* **Names follow AccessKit naming** — a semantic node without its own
  text adopts the content of its single-text chain (a button's label
  lives inside the button); plain containers keep the raw capability
  shape. `role_named(Button, "Increment")` resolves the way the §8
  locators expect.
* **Handles are stable where iced allows it** — `Id`-carrying widgets
  intern stably (id equality across dumps); id-less widgets ride the
  walk counter (stable while the structure is unchanged). Every widget
  reserves a fresh handle slot so the two sources never collide; the
  engine's generation counters guard recycling (§8.2).
* **The quirks are encoded, not hidden** (§9): `instruction::Key` covers
  Enter/Escape/Tab/Backspace only; fills click the field's right edge
  (iced places the text cursor at the click point) and clear with
  backspaces before retyping — backspaces on an empty editor are
  no-ops, so the count overshoots safely; wheel input has no
  instruction vocabulary and stays an explicit
  `AdapterError::Unsupported` row, as do blur and the pixel rows
  (render_frame/last_frame — available later through
  `Emulator::screenshot`, not this milestone's scope).

## 3. Surface

- `crates/guiwright-adapter-iced/src/dump.rs` — the §4.6 read-back:
  `TreeDumpOp` (the `Operation` walk), `TreeDumpTx` / `DumpResult`
  (the channel boundary), `HarnessApp` (the one-hook app contract with
  the default `run_hook`), `adopt_names` (accessible names).
- `crates/guiwright-adapter-iced/src/emulator.rs` — `EmulatorHarness`
  (the real `SimulatorSeam`: one-pass drain+dump, `apply_op`
  press/fill/focus, `interact` click/typewrite/tap_key, `pointer`
  move/down/up, the instruction settle loop), `SharedHarness` +
  `embedded_adapter` (the adapter construction, the egui shared-state
  shape).
- `crates/guiwright-adapter-iced/src/harness.rs` — the `SimulatorSeam`
  trait gains `pointer`; `IcedAdapter::input` routes pointer ops
  through the real pipeline (the coordinate hit-test row's
  "Unsupported" is paid: the Emulator resolves instruction targets).
- `crates/guiwright-adapter-iced/Cargo.toml` — the `iced` feature pulls
  `iced_test` + `iced_renderer` (tiny-skia); the `iced` facade rides
  dev-deps for the test/example apps.
- `crates/guiwright-adapter-iced/tests/iced_embedded.rs` — the runtime
  tests (§4 below).
- `crates/guiwright-adapter-iced/examples/counter_form.rs` — the
  consumer walkthrough (`cargo run -p guiwright-adapter-iced --features
  iced --example counter_form`).
- `crates/guiwright-bench/benches/iced_harness.rs` — the read-back
  bench (gated on the bench crate's `iced` feature).
- `bench/budgets.toml` — the `iced_tree_readback` design-target row
  (`m0_measurable = false`; byte-identical crate copy re-stamped).
- `.github/workflows/ci.yml` — the `adapter-iced` job runs
  `cargo test --features iced` (freya stays `cargo check` until its
  glue binds).

## 4. Tests

- Red→green: the red step was the runtime test against the real
  `iced_test` stack — `tests/iced_embedded.rs` failed to compile against
  the unbound seam (no `EmulatorHarness`), then the green step bound it.
  No mock rides the assertions: the `Program` is real, the events are
  real, the layout is real (tiny-skia measures "Count: 0" at its shaped
  width).
- `cargo test -p guiwright-adapter-iced --features iced` — 29 lib +
  6 integration tests passing. The integration legs: the logical tree
  resolves buttons by role+name with laid-out bounds; a semantic click
  drives the real counter (`Count: 1`, then `Count: -1` through two
  decrements); `fill` types through the real text editor and the
  greeting label follows per keystroke; the synthetic pointer pipeline
  (move → down → up) increments the counter; re-snapshots stay stable.
- `cargo run --example counter_form --features iced` — the runnable
  walkthrough prints every step's observed tree change.
- `cargo bench -p guiwright-bench --bench iced_harness --features iced`
  — one full read-back pass measures ~1.1 ms (release, 320×240); the
  `iced_tree_readback` budget row records it as a design target.
- `cargo test --workspace` (default features) — the feature-gated
  surface stays out of default builds (§5.3); the default-feature build
  carries no iced stack.

## 5. Remaining gaps (honest list, §4/§17/§18 plan items)

- **Freya glue** (§17 wave: "Freya/Slint in the first expansion wave"):
  `TestingSeam` is still unbound; the CI freya job stays
  compile-verify-only. The m76 design (Emulator + read-back operation)
  is the template — freya's TestingRunner will need its own equivalent.
- **Iced pixels** (§4.6 embed row's screenshot capability): the
  `Emulator::screenshot` path exists in iced_test 0.14 and maps cleanly
  onto `render_frame`/`last_frame`; the caps stay false until the §10
  baseline pass lands (its own milestone, §17 plan — not new surface
  rows).
- **Runtime CI matrix** (§11): the m76 leg runs the iced harness on
  ubuntu only, like the egui/masonry matrix's first stage; macOS/Windows
  iced legs ride the same job's matrix expansion, and the perf-guard
  enforcement (`m0_measurable` for `iced_tree_readback`) waits for
  cross-host baseline data.
- **crates.io publication** (§18 packaging risk): the release workflow
  packages into GitHub releases; publication stays unautomated.
