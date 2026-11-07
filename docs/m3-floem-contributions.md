# M3 — the two Floem contributions (§4.7, §17)

**Status:** implemented on the Guiwright side; the upstream patches are
small and fully specified below.
**Scope:** §17 M3 "floem contributions (test-id, tiny-skia headless) +
adapter". The adapter engine ships in `crates/guiwright-adapter-floem`
(framework-free, the `guiwright-adapter-slint` precedent — floem 0.2.0 on
crates.io predates the `HeadlessHarness`; the harness binding compiles
downstream, `contract.rs` records it).
**Evidence base:** `docs/architecture.md` §4.7, §11 (floem row), §17 row
6; recon commit `lapce/floem@1351ffb` (`src/headless.rs:101`,
`src/window/handle.rs:313-322`, `app/mod.rs:39`).

---

## 1. Why floem needs two contributions

Floem ranks 6th in the §17 maturity table: a great headless event harness
and solid public geometry, but three gaps close the door on a
Playwright-grade adapter:

| Gap | Consequence today | Contribution |
|---|---|---|
| No test ids | `getByTestId` impossible; locators fall back to text/geometry → flaky | **1a — `test_id(...)` decorator** |
| No role metadata | the logical tree cannot carry AccessKit-compatible semantics | **1b — role metadata on core views** |
| No headless pixels | `PaintState` never initializes without a GPU surface (`window/handle.rs:313-322`) | **2 — tiny-skia headless `PaintState`** |

Guiwright's side of each contribution is implemented and tested in
`crates/guiwright-adapter-floem` (see the module docs there); this
document specifies the small upstream halves.

---

## 2. Contribution 1a — the `test_id(...)` decorator

### Upstream shape (floem)

```rust
// src/views/mod.rs (new export)
pub use test_id::test_id;

// src/views/test_id.rs (new, ~40 lines)
/// First-class test id, mirroring the `debug_name` slot: registers the id
/// in a global registry keyed by this view's `ViewId`.
pub fn test_id(id: impl Into<String>, child: impl IntoView) -> TestId { ... }
```

Registration semantics — exactly what
`guiwright-adapter-floem::ids::TestIdRegistry` requires of the map:

1. **insert-once**: `register(id, view_id)` fails on a duplicate id from a
   *different* view (strict-mode duplicate, §8.1); the same view
   re-registering is idempotent;
2. **generation bump**: re-registering a handle (a slotmap key that
   recycled) bumps its generation — the ABA guard the driver's §8.2
   resolution pipeline consumes;
3. **removal hook**: when a view drops, `forget(view_id)` releases the id
   so it becomes reusable.

The registry itself can live upstream as a
`thread_local!(static TEST_IDS: RefCell<TestIdRegistry>)` (floem's widget
tree is main-thread-confined, §5.1) or reuse
`guiwright-adapter-floem::ids` directly — the module is
framework-independent and dependency-light.

### Guiwright side (already shipped)

- [`ids`](../crates/guiwright-adapter-floem/src/ids.rs) — the registry
  semantics + tests (duplicates, ABA generations, removal);
- [`logical`](../crates/guiwright-adapter-floem/src/logical.rs) — the
  harness pulls `registry.view_of(view_id)` per pass into
  `ViewReport.test_id`, and the diff lands it in the session's O(1)
  `getByTestId` hash.

---

## 3. Contribution 1b — role metadata on core views

A `view_kind → AccessKit Role` table for floem's core views
(button/label/text_input/checkbox/slider/…). Upstream shape:

```rust
// src/view.rs (extension)
impl ViewId {
    /// The §4.7 role metadata (contribution 1b): core views report their
    /// kind so accessibility layers and test drivers can map semantics.
    pub fn view_kind(&self) -> &'static str { ... }
}
```

The mapping table lives in
[`roles`](../crates/guiwright-adapter-floem/src/roles.rs)
(`RoleRegistry::defaults()` — 25 core kinds) and is data-driven: apps
extend/override it at startup without touching either codebase.

---

## 4. Contribution 2 — tiny-skia headless `PaintState`

### The gap

Headless floem never paints: `PaintState` initializes only with a GPU
surface (`window/handle.rs:313-322`), so `render_frame` cannot produce
pixels in CI. tiny-skia is already compiled in (the renderer fallback
path; `FLOEM_FORCE_TINY_SKIA=1` proves the route), and softbuffer too.

### Upstream shape (floem)

```rust
// src/window/handle.rs — PaintState::new
let paint_state = if headless {
    // §4.7 contribution 2: CPU offscreen surface (tiny-skia), no GPU.
    PaintState::TinySkia(guiwright_paint::PaintState::new(w, h)?)
} else {
    /* existing GPU-backed path */
};

// present time (same dispatch the inspector Capture path uses,
// `Renderer::finish() -> Option<ImageBrush>` when `begin(true)`):
if let PaintState::TinySkia(surface) = &mut self.paint_state {
    surface.paint(&scene_to_paint_ops(&self.scene));  // §15.5 digest + §10 capture
}
```

### Guiwright side (already shipped)

[`paint`](../crates/guiwright-adapter-floem/src/paint.rs) — the offscreen
surface + op contract, fully tested:

- `PaintState::new(w, h)` — CPU `tiny_skia::Pixmap`, size-guarded;
- `paint(&[PaintOp])` — `SolidRect`/`RoundedRect` (the M3 subset of the
  scene vocabulary; new variants extend without a semver break);
- `strided_hash()` — the §15.5 strided-pixel xxh3 frame digest (the
  `FramePresented` hash + the video dedup signal, §10);
- `png()` — lossless export via tiny-skia's encoder (§10).

The glue flips the adapter's `Caps::render_frame` row the moment the
patch is present (`FloemAdapterContract::caps_with_paint_state_patch`).
Element screenshots then crop from `last_frame` exactly like every other
adapter (§10), and the video recorder consumes frames unchanged.

---

## 5. What the adapter does *without* the patches

`FloemAdapterContract::caps_pre_patches()` is the honest §6.2 row of a
floem build without them: `logical_snapshot` ✅, `input` ✅, `clock` ✅,
`render_frame` ❌. The engine is complete and green (23 tests) — text
locators, geometry actionability, gestures and the virtual clock all work
today; `getByTestId` and pixels activate with the two patches.

## 6. CI posture

No compile-verify job for a `floem` feature: the adapter ships
framework-free (0.2.0 predates the `HeadlessHarness`), so the default
workspace build is the verification. When floem releases the harness, the
glue lands behind a `floem` feature exactly like iced's (`iced_test`) and
freya's (`freya-testing`) and joins the `adapter-features` matrix (§18
"adapters are thin & feature-gated").
