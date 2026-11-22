# M87 — Floem/GPUI pixel parity, the live-mode HTML emitter, and the codegen end-to-end proof

**Status:** implemented, the locally-buildable legs green. The 1.0.0
state map carried four recorded residual gaps: the floem glue's §10
rows (video + visual-regression baselines) were unclaimed while the
§6.2 matrix still printed "❌ → contribute tiny-skia" for a pixel path
m80 had already shipped, the gpui glue's `render_frame` answered
`Unsupported` against §4.5's own "render_frame turns true under
test-support" promise, the §12 live mode could accumulate a scrubber
but nothing attached the emitter to a *running session through the
CLI*, and the §13 codegen recorder had per-surface fragments but no
end-to-end proof that a recorded session's Rust code matches exactly.
This record pays all four. **No wire change**: the live capture rides
the M12 subscribe vocabulary, the codegen surfaces record through the
existing taps, and the pixel rows move adapter reality only.

---

## 1. Why now

After m86 every adapter had a pixel story *somewhere* — but the two
glues whose frameworks live in their own workspaces still carried
documented "❌" rows: floem's §6.2 cell pointed at a contribution the
engine had already implemented driver-side (the tiny-skia `PaintState`
over real laid-out view geometry), and gpui's `render_frame` gate
quoted the `HeadlessRenderer`-factory blocker while the test-support
surface this very glue compiles against exposes the painted-quad scene
readback the engine's m86 mock had been mirroring all along. The plan's
own binding contract ("when the real framework API matches the mock
signatures, the glue swaps the mock for the real call sites unchanged")
was one uncollected inheritance away. On the tooling side, M18/M23
shipped the live feed, the sync model and the `(live)`-stamped emitter
— but only as library pieces; the CLI's deep-link surface rendered
*finished* archives, so a running session had no mid-session
checkpoint. And §13's recorder had unit tests per tier but no test that
records a whole session and asserts the program it emits.

## 2. M1 — floem & gpui full pixel/video parity

* **gpui (`glue/gpui/src/harness.rs`, `glue/gpui/src/app.rs`)** — the
  capture is the real test-support scene: one `Window::draw` per fold
  (the framework's own render pipeline, the same call the framework's
  visual tests make), then `Window::painted_quads()` — the quads of the
  most recently rendered frame, in paint order. Each quad is clipped to
  its content mask (the API doc's own caveat: bounds are *not*
  pre-clipped; a mask-less quad carries the full viewport), divided out
  of the window scale (the test platform's 2× default), quantized
  through the §10 pixel ramp, and rasterized by the engine's new
  `composite_quads` source-over raster (`guiwright-adapter-gpui/src/
  overlay.rs` — the m86 mock raster, now the driver-side half the
  binding contract promised). `last_frame` serves the most recent
  presented raster from a per-pass cache (the m86 iced/freya shape,
  keyed by `pass_seq`); `render_frame` re-rasters on demand and honors
  a size override by scaling the scene's logical coordinates (the floem
  `PaintState::new` resize shape). The caps row flips to
  `render_frame: true`. The glyph row (text primitives) stays the
  documented upstream gap — `TestPlatform::new` ships no renderer
  factory (`platform/test/platform.rs` passes `None`), so
  `Window::render_to_image` still bails; the capture covers the quad
  scene exactly as the m86 mock's doc pinned. The m80 app gains real
  painted surfaces (base, buttons, field) and a count bar whose width
  tracks the state — a click moves real pixels, which is what the §10
  rows record.
* **floem (`glue/floem/src/harness.rs`)** — the engine's tiny-skia
  `PaintState` was already the §4.7 contribution's driver-side half;
  what was missing was the §10 rows. `render_frame` refactors onto a
  shared `raster` helper, `last_frame` serves the per-pass readback
  cache, and the pump primes the cache at its final pass — the video
  recorder's per-stamp reads hit the cache inside a pump and never
  re-render mid-settle.
* **Evidence** — `glue/gpui/tests/gpui_video.rs` and
  `glue/floem/tests/floem_video.rs` (both mirroring the m85/m86
  shape): `video_records_presented_pixels` (≥2 distinct frames around a
  real repaint, hash-equality dedup on the settled span, wall-clock
  duration metadata, EBML `.webm` magic or the PNG-folder
  `gwvideo/v1` manifest) and
  `screenshot_baseline_bless_and_diff_roundtrip` (the §10 bless → diff
  flow on real rasters). `glue/gpui/tests/gpui_embedded.rs` adds
  `render_frame_rasterizes_the_painted_scene` (requested size + pixel
  content) and keeps the m80 digest test green — the digest now rides
  the §15.5 strided *pixel* hash the engine's `frames.rs` documented
  all along.

## 3. M2 — the live-mode emitter and the codegen end-to-end

* **CLI (`crates/guiwright-cli/src/live_html.rs`, `args.rs`, `main.rs`,
  `lib.rs`)** — `guiwright live-html <target> [--out] [--duration-ms]
  [--test]`: connect over any door (the same `ConnectTarget` forms the
  REPL speaks), attach the M18 `LiveFeed` (handshake + subscribe +
  initial dump), accumulate while the session runs, and write M23's
  `render_live_html` output — stamped `gwtrace/v1 (live)`, adapter name
  in the header, `--test` naming the capture. The window ends early
  when the door goes quiet or closes; a subscribed session's idle-tick
  frames keep the full window busy, so the early end is the door-closed
  path and the checkpoint still emits what the feed saw. The viewer
  gains `LiveFeed::set_test` (the label override the flag carries —
  the `attach` agent string remains the default).
* **Evidence** — `crates/guiwright-cli/tests/live_html.rs`:
  `live_html_emits_mid_session_over_a_real_uds_door` (a real UDS
  session server, a second client performing a real click inside the
  capture window, the written HTML carrying the live stamp, the
  adapter header, the test label and the captured fixture tree) and
  `live_html_ends_early_when_the_door_closes`. Parser unit tests ride
  `args.rs`'s own fully-tested rule.
* **Codegen** — `crates/guiwright-kit/tests/codegen_e2e.rs`: one
  recording session in the §13 interactive shape (overlay on, recorder
  on), a click/fill/press/hover sequence driven through the locator
  taps, and the emitted program asserted **exactly** — role+name
  selectors from the cost model, the fill and press arguments carried,
  the tier order proven (role+name outranks text on the same node),
  reads recording nothing. `crates/guiwright-driver/tests/
  codegen_wire.rs` is the remote half: `codegen_start` → the
  pre-resolved GWP action path (`apply_action`, M17) → `codegen_stop`
  handing back the same exact code, plus the double-start/double-stop
  idempotence errors.

## 4. What did not move

No protocol rows (no new discriminants, no schema edits), no new GWP
domains, no adapter engines, no CI workflow edits — the floem/gpui
glue legs compile in their dedicated CI jobs exactly as they have since
m80 (the local host carries neither gtk3 nor xkbcommon/wayland/clang;
the recorded system-dep constraint keeps those graphs out of the
default build). The gpui glyph row and the `HeadlessRenderer` factory
stay upstream asks, recorded where they were.

## 5. Files

| Row | Files |
|---|---|
| gpui engine raster | `crates/guiwright-adapter-gpui/src/overlay.rs` (`composite_quads` + tests) |
| gpui glue capture | `glue/gpui/src/harness.rs`, `glue/gpui/src/app.rs`, `glue/gpui/Cargo.toml`, `glue/gpui/tests/gpui_embedded.rs`, `glue/gpui/tests/gpui_video.rs` |
| floem glue readback | `glue/floem/src/harness.rs`, `glue/floem/Cargo.toml`, `glue/floem/tests/floem_video.rs` |
| live emitter | `crates/guiwright-cli/src/{live_html,args,main,lib}.rs`, `crates/guiwright-cli/tests/live_html.rs` |
| viewer label | `crates/guiwright-viewer/src/live.rs` |
| codegen proof | `crates/guiwright-kit/tests/codegen_e2e.rs`, `crates/guiwright-driver/tests/codegen_wire.rs` |
| docs | `docs/architecture.md` (§4.5, §4.7, §6.2, §10, §12, §13, §16, §17), this record, `CHANGELOG.md` |
