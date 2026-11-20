# M79 — The real freya harness: `TestingSeam` bound to `freya_testing::TestingRunner` (§17 row 3)

**Status:** implemented (runtime-verified on CI). The freya adapter's
`TestingSeam` is no longer a trait waiting for a CI pass —
[`FreyaTestingHarness`] owns a real `freya_testing::TestingRunner` and
answers the seam from its verified public surface:
`sync_and_update()` commits the real AccessKit `TreeUpdate` (the Level-1
diff surfaced through `tree_update`, previously a hardcoded `None`),
input rides `send_event(PlatformEvent)` in the frame-quantized
press→sync→release order with modifiers and explicit `KeyUp` (§4.3 gap),
`Focus`/`Blur` ride the native `send_accessibility_action` path, the §14
clock steps the `RenderingTicker` through `poll_n`, and `render_frame`
decodes the runner's PNG snapshot into §10 RGBA pixels. The CI
`adapter-freya` job upgrades from `cargo check` to `cargo test --features
freya`. **No wire change**: this milestone moves adapter reality, not
protocol rows.

---

## 1. Why now

m76 bound iced; m78 bound slint; freya was the last seam trait with zero
framework call sites — `freya-testing` named by the feature docs but
never imported (the m2-era `unused_dependencies = "allow"` carve-out
existed exactly for it). This record pays that debt: the framework type
lands in non-test impl code (`freya_testing::TestingRunner`, real
`PlatformEvent`s, `ActionRequest`s), runtime tests drive a real freya
app through [`Session`], and the carve-out retires.

## 2. Design

* **The runner, whole** — `TestingRunner::new(app, size, hook, 1.0)`
  boots a complete sync, windowless harness (ragnarok events, the
  accessibility tree, the Skia rasterizer). Scale factor 1.0 degenerates
  the §9 "adapters own the scale conversion" rule: logical px ==
  physical px. The hook provides the app's root contexts; the test
  observes the same `State` handles.
* **`AccessibilityId = accesskit::NodeId`** — freya 0.5 aliases the two,
  so the app-assigned `.a11y_id(AccessibilityId(n))` numbers ARE the
  committed update's node ids. The [`AccessibilityMap`] bridge binds the
  ids each update carries and unbinds the ones the removal diff reports
  (the §8.2 ABA guard against recycled ids), and `act` maps a session
  node straight back to its freya id.
* **Frame-quantized actions** (§4.3 "input is frame-quantized by
  ragnarok's `EventBuffer` — respect press→sync→move→release ordering"):
  coordinate actions queue `Press` → flush (`send_event`) → one harness
  step (`sync_and_update`) → `Release` → flush → step. Semantic
  `Click`/`Check`/`Uncheck`/`DoubleClick`/`Hover`/`Scroll` ride that
  pipeline at the node's laid-out bounds (ragnarok hit-testing, §4.3
  "pixel clicks go through `send_event` + hit-testing"); `Focus`/`Blur`
  ride the native accessibility path
  (`send_accessibility_action(ActionRequest)`); fills click the field
  into focus, clear with backspaces through the real editor, and commit
  the text as one `Key::Character` KeyDown (the upstream `write_text`
  shape); key presses run the full §9 sequence — modifier downs, key
  down with modifiers, key up, modifier ups in reverse (the §4.3
  `press_key`-sends-KeyDown-only gap closed).
* **Pixels are PNG round trips** — `TestingRunner::render()` rasterizes
  its N32 surface and PNG-encodes it; the adapter decodes back to RGBA
  through `guiwright-shot` (the framework-free §10 codec stack) so
  `FrameRef` carries real pixels and the xxh3 frame digest stays the
  visual-change signal.
* **Quirks encoded, not hidden** (§9): the runner renders at its boot
  viewport, so `render_frame` ignores a requested size (documented);
  `last_frame` stays hash-of-pass (no continuous presentation); the
  removal diff is computed by the seam (tracked ids minus the committed
  update's), not a framework callback.

## 3. Surface

- `crates/guiwright-adapter-freya/src/runner.rs` — the `TestingSeam`
  trait (now returning the committed `TreeUpdate`, plus `poll` and
  `render`), [`FreyaAdapter`] (absorbs updates into the id bridge and
  the bounds/value maps; `tree_update` one-shot; coordinate/native
  action split; ticker clock; PNG `render_frame`), and
  [`FreyaTestingHarness<T>`] — the real seam.
- `crates/guiwright-adapter-freya/src/platform.rs` — the mirror
  vocabulary gains `TextInput` (the fill commit path).
- `crates/guiwright-adapter-freya/Cargo.toml` — the `freya` feature
  pulls `guiwright-shot` (the §10 decode); `freya-components` rides
  dev-deps for the test/example apps.
- `crates/guiwright-adapter-freya/tests/freya_embedded.rs` — the runtime
  tests (§4 below).
- `crates/guiwright-adapter-freya/examples/counter_form.rs` — the
  consumer walkthrough (`cargo run -p guiwright-adapter-freya --features
  freya --example counter_form`).
- `.github/workflows/ci.yml` — the `adapter-freya` job runs
  `cargo test --features freya` (upgraded from `check`).
- `Cargo.toml` (workspace) — the `unused_dependencies = "allow"`
  carve-out retires: every feature-gated harness dependency now has a
  real use-site (iced_test since m76, freya_testing since m79).

## 4. Tests

- Red→green: the red step was the runtime test against the real stack —
  `tests/freya_embedded.rs` failed to compile against the unbound seam
  (no `FreyaTestingHarness`), then the green step bound it. The freya
  stack (software Skia) needs a C++ toolchain, so this leg is
  **runtime-tested on CI** (`cargo test -p guiwright-adapter-freya
  --features freya`); locally the framework-free surface compile-verifies
  (default `cargo check`/`clippy`/`test` carry no freya crate).
- The integration legs (CI): the committed diff resolves the
  `.a11y_id`-assigned nodes with laid-out bounds; a semantic click drives
  the real counter (`Count: 1`); `fill` types through the real `Input`
  editor and the greeting label follows; the raw pointer pipeline
  (move → down → up) increments the counter; `render_frame` returns real
  RGBA pixels at the boot viewport; key sequences carry modifiers and
  KeyUp.
- Unit legs (feature-gated, CI): the mock-seam suite pins the adapter
  contract — canonical buffer order, the full key sequence, native
  action routing, bounds-demanding coordinate actions, unknown-node
  loudness, ticker-backed clock, KeyUp delivery.

## 5. Remaining gaps (honest list, §4/§17/§18 plan items)

- **Floem/GPUI glue** (§17 rows 6–7): framework-free engines with
  recorded downstream contracts; the m76/m78/m79 designs are the
  templates.
- **Runtime CI matrix** (§11): the freya leg runs on ubuntu only (the
  Skia stack compiles there); macOS/Windows ride the job's matrix
  expansion when baselines need them.
- **crates.io publication + tag/version reconciliation** (§18 packaging
  risk): workspace 0.4.0 vs the v0.2.0 tag, CHANGELOG still
  `[Unreleased]`, publication unautomated — the NEXT session's release
  work.
