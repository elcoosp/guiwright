# M78 — The real slint harness: `TestingBackend` bound behind the glue workspace (§17 row 5)

**Status:** implemented. The slint adapter's framework half is no longer a
doc contract waiting for a downstream pass — the repo's **`glue/slint`
workspace** binds the framework-free engine to the real
`i-slint-backend-testing` `TestingBackend`: `slint::platform::set_platform`
installs it (mock time + the software rasterizer), interactions inject real
`WindowEvent`s through `Window::dispatch_event` ("the same code path real
winit events take", §4.4 door A), the tree folds real `ElementHandle`
introspection through the engine, pixels come from
`Window::take_snapshot`, and the clock executes the §14 plan on
`mock_elapsed_time`. The CI `slint-glue` job runtime-tests it
(`cargo test --manifest-path glue/slint/Cargo.toml`). **No wire change**:
this milestone moves adapter reality, not protocol rows.

---

## 1. Why now

m76 paid the iced seam debt; freya stayed the only bound-pending harness
and slint stayed where M2 left it: an engine crate (introspection model,
id bridge, role registry, clock plan, build helper) whose framework half
was a doc paragraph pointing at workspaces that do not exist. §17 row 5
("testing backend is complete but semver-exempt … a11y door exists for
live apps") kept slint in the first expansion wave. This record pays that
debt the way m76 did: a real framework type (`i_slint_backend_testing::`
`TestingBackend`, `ElementHandle`, `WindowEvent`) lands in non-test impl
code, a runtime test drives a real `.slint` app through [`Session`]
(resolve by role/name with real bounds → click → fill → clock → pixels),
and the "slint glue lands downstream" notes become false in the honest
direction: the glue now lives in the repo, in its own workspace.

## 2. Design

* **The glue workspace, not a feature** — the m2-era constraint held:
  the slint *facade* is the skia carrier. Verified against slint 1.19.0
  empirically (the first m78 attempt tried the in-workspace feature):
  the facade depends on `i-slint-backend-selector` unconditionally; the
  selector's desktop graph carries `i-slint-backend-winit` (optional),
  whose own optional `i-slint-renderer-skia` → `skia-safe` →
  `skia-bindings` chain is *lockfile-resolved even when no feature
  activates it* — `cargo tree -i i-slint-renderer-skia` prints "nothing
  to print" while `Cargo.lock` still carries `skia-bindings 0.157.0`,
  and the resolver rejects the graph the moment freya's
  `freya-skia-bindings` joins it (`links = "skia"` twice). The
  `i-slint-backend-testing` crate alone *is* skia-free (its
  `renderer-software` feature pulls the pure-Rust rasterizer), but real
  `.slint` apps cannot avoid the facade — `slint!` and `slint-build`
  both generate facade-bound code. So the task's fallback branch fires:
  **`glue/slint/` is its own workspace root** (`resolver = "2"`),
  path-depends on the engine + `guiwright-driver` + `guiwright-core`,
  and carries slint's skia lock entry while the main workspace keeps
  freya's. The "can't carry both" note stays true for one workspace and
  false for one repo — the constraint's crate-doc rewrite spells out the
  evidence.
* **`set_platform`, not the init helpers** — `init_no_event_loop()`
  hardcodes `renderer_name: None` (the mock renderer: deterministic
  fixed-font metrics, no pixels). The harness calls the public
  [`slint::platform::set_platform`] with a
  `TestingBackendOptions { mock_time: true, threading: false,
  renderer_name: Some("software") }` — mock time for §14 and a real
  rasterizer so `Window::take_snapshot` produces pixels. The platform is
  per-thread: one harness per test thread (§18's process-per-test
  discipline maps onto threads).
* **The walk is flat; nesting is reconstructed** — the public
  `ElementHandle` API exposes `visit_descendants` (DFS, no parent links)
  and per-element geometry. The harness rebuilds the tree by bounds
  containment — the m76 sibling-closing technique — with a half-px
  tolerance. Elements the walk yields invisibly are skipped upstream (the
  backend already filters `is_visible`).
* **Widget-internal editors collapse** (the one synthesis rule beyond
  containment): slint's `LineEdit` nests its real `TextInput` editor
  under generic layout boxes *and* the placeholder `Text`, so a
  role-matching descendant is not a direct child of its widget. The walk
  marks any non-generic element whose **role matches an open ancestor**
  as collapsed; at unwind it folds its facts into that ancestor (label,
  value, test id, editor length, and the editor's geometry — the widget
  rect includes padding the editor does not fill, so fills click the
  editor's right edge) and re-parents its children. The session's
  strict-mode resolver then sees one semantic node per widget
  (`getByRole(TextInput)` resolves uniquely). The m76 precedent is the
  same idea: synthesis closes implementation detail so §8 selectors work.
* **Text semantics are the app's contribution** — stock `Button` /
  `LineEdit` carry roles and values; plain `Text` elements carry no
  accessible role until the app declares one. The m78 consumer app
  declares `accessible-role` + `accessible-label` on its counter/greeting
  texts (the §4.4 shape of the iced adapter's role-registry
  contribution), so `Selector::text("Count: 1")` resolves like it does on
  the other adapters. Element *names* from the debug-info build
  (`SLINT_EMIT_DEBUG_INFO=1`, here via
  `CompilerConfiguration::with_debug_info(true)` in the glue's build.rs)
  ride the diff as test ids.
* **Keyboard rides the real pipeline** — `WindowEvent::KeyPressed` /
  `KeyReleased` with explicit modifier presses and releases (uppercase
  chars ride a Shift pair, mirroring the backend's own string-sequence
  helper), key names mapped onto the §9 slint key vocabulary
  (`Enter`/`Escape`/`Tab`/`Backspace`/`Delete`/`Home`/`End`/arrows/
  modifiers, single characters pass through; anything else is an explicit
  `Unsupported` row). Wheel maps to `PointerScrolled` at the last pointer
  position. Clicks are move → press → `mock_elapsed_time(50)` → release
  (the backend's own `mock_single_click` timing, inside the 500 ms
  default click interval, so double clicks stay recognizable).
* **Quirks encoded, not hidden** (§9): the testing backend exposes no
  public focus query, so synthesized nodes carry no focus fact; `Blur`
  has no dispatch target (focus moves by clicking elsewhere); equal-rect
  siblings nest in containment order; `last_frame` stays false (no
  continuous presentation — `render_frame` re-renders on demand).

## 3. Surface

- `glue/slint/` — the new workspace root (own `Cargo.toml` +
  `Cargo.lock`, lints table mirroring the main workspace policy with two
  documented deltas: `unsafe_code = "deny"` because the slint
  `include_modules!` expansion carries its own
  `#[allow(unsafe_code)]` vtable macros — `forbid` is a hard E0453
  against generated code — and `missing_debug_implementations = "allow"`
  for the same generated module; the hand-written glue keeps manual
  `Debug` impls and zero unsafe).
- `glue/slint/src/harness.rs` — `install_testing_platform`,
  `SlintAdapter::boot` (platform → factory → size → first pass), the
  containment walk with same-role collapse, and the
  [`FrameworkAdapter`] impl (pump/mock-time settle, `logical_snapshot`,
  `act` click/double-click/fill/focus/hover/press/scroll, `input`
  pointer+wheel+keys, `clock` plan execution, `render_frame` via
  `take_snapshot`).
- `glue/slint/src/lib.rs` — the crate root naming the binding and the
  constraint.
- `glue/slint/build.rs` — the real `.slint` app compiled with
  `debug_info: true` (the programmatic §11 flag).
- `glue/slint/examples/counter_form.slint` + `counter_form.rs` — the
  consumer walkthrough (`cargo run --manifest-path glue/slint/Cargo.toml
  --example counter_form`).
- `glue/slint/tests/slint_embedded.rs` — the runtime tests (§4 below).
- `glue/slint/benches/harness.rs` — the `slint_introspection` criterion
  suite (always-on: the whole crate is the glue workspace).
- `crates/guiwright-adapter-slint/src/lib.rs` + `Cargo.toml` — the
  false "downstream" contract rewritten with the verified evidence; the
  description names the glue workspace. The engine itself is unchanged —
  the glue binds it unchanged (the walk emits its
  [`IntrospectionItem`]s and executes its clock plan).
- `bench/budgets.toml` (+ crate copy) — the `slint_introspection`
  design-target row (`m0_measurable = false`; measured by the glue's
  criterion suite, not the main workspace's samples).
- `.github/workflows/ci.yml` — the `slint-glue` job (fmt + clippy + the
  runtime tests) with its own rust-cache workspace entry; the stale
  "slint has no compile-verify job" comment replaced.

## 4. Tests

- Red→green: the red step was the runtime test against the real stack —
  `tests/slint_embedded.rs` failed to compile against the missing
  `harness` module (the `.slint` app itself compiled first — build.rs,
  debug info, `include_modules!`), then the green step bound it.
- `cargo test --manifest-path glue/slint/Cargo.toml` — 7 integration
  tests passing: the logical tree resolves buttons/line-edit by role with
  laid-out bounds; a semantic click drives the real counter (`Count: 1`,
  then `Count: -1` through two decrements); `fill` types through the real
  editor pipeline and the greeting label follows; the synthetic pointer
  pipeline (move → down → up) increments the counter; re-snapshots stay
  stable; `mock_elapsed_time` advances virtual time;
  `render_frame(240, 180)` returns real RGBA pixels (non-transparent).
- `cargo run --manifest-path glue/slint/Cargo.toml --example
  counter_form` — the walkthrough prints every step's observed change,
  ending in a 480×320 RGBA snapshot.
- `cargo bench --manifest-path glue/slint/Cargo.toml --bench harness` —
  one full introspection pass measures ~41 µs (release, 480×320); the
  `slint_introspection` budget row records it as a design target.
- `cargo clippy --manifest-path glue/slint/Cargo.toml --all-targets --
  -D warnings` and `cargo fmt --manifest-path glue/slint/Cargo.toml
  --all -- --check` — clean (the glue's own gate, mirrored by the CI
  job).
- Main workspace default builds stay framework-free for slint (§5.3):
  `cargo tree` carries no slint crate; the skia entry belongs to freya's
  feature graph alone.

## 5. Remaining gaps (honest list, §4/§17/§18 plan items)

- **Freya glue** (§17 wave: "Freya/Slint in the first expansion wave"):
  `TestingSeam` is still unbound; the CI freya job stays
  compile-verify-only. The m78 design (real harness + collapse rule +
  facts map) is the template.
- **Slint live door** (§4.4 door B): the winit backend's AccessKit tree
  is the remote/live path; the glue binds the testing backend only, per
  §17 row 5's embedded scope.
- **Focus facts** (§4.4 caveat): the testing backend's focus item is
  internal-only; the synthesized tree carries no focus until upstream
  exposes it (the ElementHandle surface is the per-release boundary).
- **Runtime CI matrix** (§11): the slint-glue leg runs on ubuntu only,
  like the m76 iced leg; macOS/Windows ride the same job's matrix
  expansion when the baselines need them.
- **crates.io publication** (§18 packaging risk): the release workflow
  packages into GitHub releases; publication stays unautomated (the
  glue workspace's own lock/version train rides the same
  reconciliation).
