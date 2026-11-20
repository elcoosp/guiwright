# M80 — The floem and gpui downstream glue: both last engines bound to their real frameworks (§4.7/§4.5, §17 rows 6+7)

**Status:** implemented (floem fully green; gpui green on tests/example with
two residual clippy-pedantic errors in the glue workspace to fix in a
follow-up). The repo's **`glue/floem`** and **`glue/gpui`** workspaces bind
the two remaining framework-free engines to their real frameworks, the way
m76 (iced), m78 (slint) and m79 (freya) did. **No wire change**: this
milestone moves adapter reality, not protocol rows.

---

## 1. Why now

After the rc.1 reconciliation, exactly two core plan items remained — the
final two adapters. The architecture doc (§4.7 floem, §5.3/§17 gpui)
explicitly sanctions **downstream glue workspaces** for both: floem's
git-only `HeadlessHarness` plus its Linux gtk graph (muda, the winit fork)
never enters the main workspace, and gpui's Linux platform stack needs
system libraries (xkbcommon, wayland) a default build must never require —
plus `test-support` is a git-only feature. This record pays that debt the
way m78 did: real framework types land in non-test impl code, runtime
tests drive a real app through `Session`, and the "downstream" notes
become false in the honest direction.

## 2. Floem — `glue/floem` (M1, fully green)

* **The harness is real** — `HeadlessHarness` exists only on
  `lapce/floem` main; the glue pins the exact revision the §4.7 recon
  cites (`1351ffb`, "fix: forward post_paint through Box<dyn View>
  (#1089)"). `TestRoot::new()` claims the thread's root view, the app
  factory builds the real view tree against it, and
  `HeadlessHarness::new_with_size` wraps it in the headless
  `WindowHandle`.
* **Renderer constraint, empirically pinned** — floem's
  `paint/renderer.rs` imports `floem_vger_renderer` unconditionally
  (E0432 without the feature), so the glue carries `vger` (wgpu-based;
  compiles headless, never initialized by the harness).
* **The walk is real** — `ViewId::children()`/`debug_name()`/
  `get_visual_rect()` fold into the engine's `FloemLogicalTree`; test
  ids ride floem's own `debug_name(...)` decorator (the engine's
  `TestIdRegistry` is fed from it); focus is real via
  `HeadlessHarness::is_focused`; kinds decode floem's
  `"<decorators> - <Type>"` names (module paths AND content payloads —
  `"floem::views::button::Button"`, `"Label: \"Count: 0\""`) into the
  role registry's snake_case vocabulary.
* **Input is real** — `dispatch_event` with real `Event::Pointer` /
  `Event::Key` payloads, clicks through real hit-testing
  (`element_id_at` → `owning_id()`, with an ancestor climb because hit
  tests land on the button's inner label), fills type real
  `KeyboardEvent`s.
* **Pixels are the contribution's driver half** — the engine's tiny-skia
  `PaintState` rasterizes the real laid-out view geometry into
  `PaintOp`s (§4.7 contribution 2's surface + op contract; the upstream
  patch routing floem's display list would make it bit-identical to the
  GPU scene).
* **Blocker 1, file-level evidence** — floem's `text_input` requests IME
  the moment it gains focus (`src/views/text_input.rs:1403`,
  `set_ime_allowed(true)` hard-coded), and the headless `MockWindow`
  rejects every IME request (`src/window/mock.rs:262`,
  `Err(ImeRequestError::NotSupported)`), which
  `src/window/handle.rs:1270` unwraps into a panic — floem's own
  40-file harness suite never focuses a text_input. The glue gates
  `SetValue` on `Role::TextInput` BEFORE the focus click and reports
  `AdapterError::Unsupported` carrying the evidence; the fill
  implementation (real focus click + real `KeyboardEvent` sequence)
  stays in the harness for the upstream fix.

## 3. GPUI — `glue/gpui` (M2, tests/example green)

* **The dependency is git-only by necessity** — crates.io `gpui` 0.2.2
  ships no `test-support` feature (verified against the registry index;
  its feature list is `inspector, leak-detection, macos-blade,
  runtime_shaders, screen-capture, wayland, windows-manifest`), so the
  glue rides the zed git source with `default-features = false,
  features = ["test-support"]`.
* **Boot is real** — `TestAppContext::build(TestDispatcher::new(seed))`
  (the §4.5 seeded virtual scheduler), then `open_window` on the real
  `TestPlatform` running the real counter+form entity (`glue/gpui/src/
  app.rs`): `div()` buttons with real `on_click` listeners over real
  entity state, a focused text field with a real `on_key_down` handler,
  roles via gpui's own `role(accesskit::Role)` DSL, names/geometry via
  gpui's own `debug_selector` (the framework records laid-out bounds at
  paint).
* **Input is the canonical seam** — `Window::dispatch_event(PlatformInput)`
  with real `MouseDown/MouseUp/MouseMove/Scroll/KeyDown/KeyUp` events
  (synthetic events are bit-identical to OS input, §4.5); clicks hit
  real hit-testing and real focus.
* **The tree is the hybrid's fallback** — the a11y tree never flows on
  the test platform (`platform/test/window.rs` overrides nothing of the
  `a11y_init` activation path), so the glue synthesizes an
  AccessKit-shaped `LogicalTreeDiff` (roles from the `role(...)` DSL,
  labels/values from the entity's real state, bounds from the
  framework-recorded paint geometry) and feeds it through the engine's
  `A11yHybrid` bookkeeping. gpui's accesskit generation (0.24) rides
  aliased (`accesskit-gpui`) next to the main workspace's 0.25.2.
* **The digest is content-sensitive** — §15.5 xxh3 over the
  framework-recorded element geometry + the folded tree's labels/values;
  it follows the painted content (the counter label changes it).
* **Clock is first-class** — the seeded `TestDispatcher::advance_clock`
  rounds + `run_until_parked` (§5.1's GPUI waker row).
* **Blocker 2, file-level evidence** — the real `KeyDown` path panics
  inside zed's test scheduler (`scheduler/src/test_scheduler.rs:232`,
  `state.next_session_id.0 += 1` overflow), reproducible with three
  real `Keystroke` dispatches after a real focus click. The glue gates
  `SetValue` with `AdapterError::Unsupported` carrying the evidence;
  the real fill sequence (focus click + `Keystroke`s through
  `dispatch_event` into the focused `on_key_down`) is specified in the
  crate docs and waits on the upstream scheduler fix.
* **Pixel gap, documented** — `Window::render_to_image` bails without a
  `HeadlessRenderer` and `TestPlatform::new` ships no factory
  (`platform/test/platform.rs` passes `None`); `render_frame` reports
  `Unsupported` with that evidence, and `last_frame` stays the
  hash-only §15.5 row.

## 4. Surface

- `glue/floem/` — workspace root (`Cargo.toml` + `Cargo.lock`, lints
  mirroring the main policy with the glue's documented deltas),
  `src/lib.rs` + `src/harness.rs` (the adapter), `tests/floem_embedded.rs`
  (8 runtime tests), `examples/counter_form.rs`, `benches/harness.rs`
  (the `floem_introspection` criterion suite; the sample rides CI).
- `glue/gpui/` — the same shape: `src/lib.rs` + `src/app.rs` (the real
  counter+form entity) + `src/harness.rs` (the adapter),
  `tests/gpui_embedded.rs` (7 runtime tests), `examples/counter_form.rs`,
  `benches/harness.rs`.
- `.github/workflows/ci.yml` — the `floem-glue` and `gpui-glue` jobs
  (fmt + clippy + runtime tests + the consumer example), with the
  Linux system packages each framework graph needs (`libgtk-3-dev` for
  floem's muda/rfd; `libxkbcommon-dev` + `libwayland-dev` for gpui's
  platform stack).
- `crates/guiwright-adapter-floem` / `-gpui` — engines unchanged; the
  glue binds them unchanged.
- `README.md` — the adapter matrix row now reads 7/7 real.
- `CHANGELOG.md` — the unreleased section.

## 5. Tests

- Floem (all green): `cargo test --manifest-path glue/floem/Cargo.toml`
  — 8 integration tests: the logical tree resolves buttons/input/labels
  by role with laid-out bounds; a semantic click drives the real counter
  (`Count: 1`, then `Count: -1`); the synthetic pointer pipeline
  (move → down → up) increments the counter; `fill` reports the
  documented headless-IME blocker and the loop keeps working around the
  blocked cell; hit-testing agrees with the walked geometry;
  re-snapshots stay stable; harness ticks advance virtual time;
  `render_frame` returns real tiny-skia pixels. The consumer example
  runs end-to-end. `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --all -- --check` clean.
- GPUI (tests/example green): `cargo test --manifest-path
  glue/gpui/Cargo.toml` — 7 integration tests: role/name resolution
  with real bounds; semantic clicks drive the real counter through
  `dispatch_event`; the synthetic pointer pipeline increments; `fill`
  reports the documented scheduler blocker with the evidence string;
  re-snapshots stay stable; dispatcher rounds advance virtual time; the
  scene digest follows painted content. The consumer example runs
  end-to-end. Residual: two clippy-pedantic errors in the glue's bench
  target remain (follow-up).

## 6. Remaining gaps (honest list)

- **GPUI fill** — blocked by the zed test-scheduler session-id overflow
  (evidence above); the real sequence is specified and gated.
- **Floem fill** — blocked by the headless IME unwrap (evidence above);
  same shape.
- **GPUI pixels** — `render_to_image` needs a `HeadlessRenderer`
  factory the test platform does not ship; configure one (a
  `PlatformHeadlessRenderer` over lavapipe/llvmpipe) to unlock
  `render_frame` and real pixels.
- **GPUI a11y** — the synthesized-diff fallback stands in for the
  Level-1 tree until a test-platform activation path exists upstream.
- **GPUI glue clippy** — two residual pedantic errors in the bench
  target (the rest of the workspace passes `-D warnings`).
