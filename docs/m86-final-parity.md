# M86 — Final parity: the iced/freya video + visual-regression rows ship, the overlay count closes at 7/7, and the publish dry-run gate passes

**Status:** implemented, all three legs green. The 1.0.0 capability
matrix carried three recorded residual gaps: the iced and freya video
rows were "documented unclaimed" (both adapters have pixel paths, so
§6.2 parity requires video + visual-regression baselines), the floem/gpui
overlay hooks left the §13 count at 5/7, and the ultimate release gate —
`cargo publish --dry-run --locked` over all 18 crates — had never been
executed locally. This record pays all three. **No wire change**: every
milestone moves adapter reality, binding proof, and release evidence,
not protocol rows.

---

## 1. Why now

After m84/m85 the adapter matrix was real on all seven frameworks but
the *capability* rows were not: iced advertised `render_frame` only (its
§6.2 row promises `last_frame` too), freya advertised `last_frame` while
its readback returned `None` (the caps row lied), and neither adapter had
a video or baseline test. The floem/gpui adapters answered `show_overlay`
with the trait default (`Unsupported`), leaving the §13 overlay count at
5/7 against the plan's explicit fallback ("if system dependencies block
sandbox compilation, implement them using mocked framework types that
exactly mirror the real APIs, with compile_fail doc-tests or integration
tests proving the trait signatures bind correctly"). And the release
orchestration had been written (the justfile's `release-prep`) but never
run — the disk ceiling stopped it.

## 2. M1 — iced & freya video + visual-regression parity

* **iced (`guiwright-adapter-iced/src/harness.rs`)** — the caps row now
  matches the §6.2 matrix (`last_frame: true`): the adapter caches the
  most recent screenshot pass (keyed by `pass_seq`) and serves it from
  the new `last_frame` readback; `render_frame` and `last_frame` share
  one side-effect-free pass helper (`screenshot_pass_cached`), so the
  video recorder's per-stamp readback hits the cache within a pump and
  re-renders only across passes. The §10 pixels flow: pump stamps →
  `last_frame` → recorder (hash-equality dedup collapses the settled
  pass — the Zen pump emits it twice).
* **freya (`guiwright-adapter-freya/src/runner.rs`)** — `render_frame`
  caches the frame it builds; `last_frame` returns the cache within a
  pass and takes one fresh deterministic snapshot across passes (the
  §6.2 "post_render_callback" row, now honest). Freya's pump stamps ride
  the pass sequence as their digest (documented on the stamp), so the
  static-span dedup is asserted on iced and NOT on freya — the test
  documents the divergence.
* **The evidence rows** (`iced_embedded.rs`, `freya_embedded.rs`):
  `video_records_presented_pixels` (start → pump → 80 ms wall-clock span
  → a real repaint → stop; asserts ≥2 distinct frames, a deduped static
  span on iced, non-zero artifact, duration ≥40 ms, and container
  validity — EBML webm magic or the `video.json` manifest at
  `gwvideo/v1`), and `screenshot_baseline_bless_and_diff_roundtrip`
  (`BlessMode::Failing` blesses an element baseline from the real
  raster, `BlessMode::Off` diffs the unchanged re-render clean — the §10
  pipeline end to end through `guiwright-shot`'s YIQ + pixel-budget
  store). Unit-level readback proofs (`last_frame_reads_back_the_
  presented_pixels`) pin the cache discipline in both adapters.
* **Red → green** (TDD, one cycle): with the readbacks unimplemented the
  video tests fail (`0 passed; 1 failed` on both adapters — the recorder
  finalizes an empty artifact); with the caches in, the iced leg runs
  10/10 and the freya leg 11/11 under `--features`.

## 3. M2 — floem & gpui overlay mock-binding proof (5/7 → 7/7)

* **The mock hosts** (`guiwright-adapter-floem/src/overlay.rs`,
  `guiwright-adapter-gpui/src/overlay.rs`): framework-free types that
  exactly mirror the real APIs the §13 route touches — floem's
  `OverlayHost::present(&plan)` mirrors the headless present-path hook
  (post-paint → `PaintOp`s the tiny-skia `PaintState` rasterizes), and
  gpui's `OverlayHost::paint_quads(&plan)` / `painted_quads()` /
  `scene_digest()` mirror the test-support paint route
  (`Window::paint_quad` → `painted_quads`, the same scene digest the
  glue's frame hashes ride). Both record the §13 draw-call log and both
  ship `composite` paths (ops onto the tiny-skia scene; straight-alpha
  source-over quads onto a scripted RGBA raster).
* **The binding** — the mirror examples implement `show_overlay` against
  the hosts (floem: overlay ops appended to the present-time scene; gpui:
  the quads composited onto the scripted raster so the pixels AND the
  §15.5 digest move), and the real glue workspaces wire the identical
  hooks (`glue/floem/src/harness.rs` composites onto the PaintState
  pixels; `glue/gpui/src/harness.rs` folds the host's scene digest into
  the hash-only path — its `HeadlessRenderer` pixels blocker stays the
  documented upstream gap). The mirror proof is the plan's sanctioned
  fallback for the upstream blockers; the glue edits ride the same
  engine calls the mirrors prove.
* **The signature pins** — `compile_fail` doc tests in both mirrors'
  `lib.rs`: a placeholder without the host's present/paint signature
  cannot bind the §13 route (E0599), the negative half of "when the real
  framework APIs match the mock signatures, the overlay binding compiles
  and routes correctly"; the positive half is the engine unit tests
  (plan → ops/quads → log → pixel/digest movement) plus the mirror
  integration tests.
* **The evidence row** (`examples/downstream-glue/{floem,gpui}/tests/
  overlay.rs`): through a real `Session` — `inspect_enable_overlay` +
  `inspect_highlight` composite the plan, the frame pixels differ from
  the bare app's baseline and the digest moves, the draw log names the
  outline + badge draws, and `inspect_disable_overlay` restores the bare
  frame byte-for-byte.
* **Red → green**: the red is the bind failure the plan describes — the
  tests reference `show_overlay` routing and a `take_overlay_paint_log`
  surface that did not exist (`E0599`, compile red); with the hosts and
  hooks in, both mirrors run green (bind + caps + overlay + doc tests).

## 4. M3 — the publish dry-run gate

`scripts/m3_release_dry_run.sh` (the justfile `release-prep` recipes,
minus the version bump): `cargo clean` → the metadata strictness gate
(all 18 crates carry description/license/repository/keywords/categories)
→ `cargo publish --workspace --dry-run --allow-dirty --locked`. The
workspace dry-run packages every crate in dependency order against the
local family and runs the release verification build — the gate the CI
release workflow re-runs on the pushed tag. Green with exit 0 (the run
log is the evidence; `CARGO_PROFILE_RELEASE_DEBUG=0` keeps the 9.9 GB
disk ceiling comfortable, `cargo clean` before and after).

## 5. What moved, at a glance

| Area | Files |
|---|---|
| iced video/readback | `crates/guiwright-adapter-iced/src/harness.rs` (+ tests, dev-dep `serde_json`) |
| freya video/readback | `crates/guiwright-adapter-freya/src/runner.rs` (+ tests, dev-deps kit/tempfile/serde_json) |
| floem overlay host | `crates/guiwright-adapter-floem/src/overlay.rs`, `lib.rs`, `Cargo.toml`, `contract.rs` |
| gpui overlay host | `crates/guiwright-adapter-gpui/src/overlay.rs`, `lib.rs`, `Cargo.toml`, `contract.rs` |
| mirror binding proof | `examples/downstream-glue/{floem,gpui}/` (adapter, lib doc tests, `tests/overlay.rs`, Cargo.toml) |
| real glue hooks | `glue/floem/src/harness.rs`, `glue/gpui/src/harness.rs` (+ Cargo.toml inspect deps) |
| release gate | `scripts/m3_release_dry_run.sh` (run log in the handoff evidence) |
| docs | `docs/architecture.md` (§4.4, §6.2, §10, §16), this record |

No protocol rows moved, no new test vocabulary, no new CLI flags, no
adapter engines — the allowed surface for this milestone was exactly the
iced/freya video integration, the floem/gpui overlay mock-bindings, and
the release dry-run orchestration.
