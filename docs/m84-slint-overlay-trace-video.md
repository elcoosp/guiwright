# M84/M85 — The slint inspector overlay ships (5/7) and the trace + video rows validate end-to-end on real adapters

**Status:** implemented, both milestones green. M84 closes the §4.4
"top-level overlay window" design for slint — the overlay count moves from
4/7 to 5/7 (floem/gpui ride their recorded upstream blockers). M85 pins the
§12 viewer and the §10 recorder against **real** adapter sessions: trace
archives generated from egui/iced/slint render through `guiwright
show-trace`, and video recordings from egui/masonry/slint finalize valid
artifacts. **No wire change**: both milestones move adapter reality and
test evidence, not protocol rows.

---

## 1. Why now

After the 1.0.0 release exactly three §13/§12/§10 parity gaps remained.
The §4.4 overlay bullet had documented the top-level-window design since
the m78 recon ("the registry hosts multiple windows, and
`MinimalSoftwareWindow` is a no-window-manager `WindowAdapter`") without
an implementation; the `guiwright show-trace` pipeline had only ever been
exercised on synthetic recorder fixtures (`guiwright-viewer` e2e,
`guiwright-cli` show-trace tests); and the video recorder's real-pixel
coverage stopped at the mock adapter's scripted frames.

## 2. M84 — the slint overlay (`glue/slint/src/overlay.rs`)

* **The design as documented** — the overlay renders in a borderless
  second `.slint` component window: inline `slint!` compilation (pinned
  `#[edition = "1.19"]`), `no-frame` + `always-on-top` hints (no compositor
  stands behind them on the testing backend), `background: transparent`
  so every unpainted pixel stays at alpha 0 in the snapshot (the software
  renderer renders into a premultiplied buffer whose clear is
  `(0,0,0,0)` — verified against `i-slint-renderer-software` 1.19.0's
  `take_snapshot`, which un-premultiplies to the straight-alpha public
  format).
* **The plan stays data** — `OverlayHost::show(plan, size)` converts the
  §13 [`OverlayPlan`] into model rows (one outline row per hover rect,
  one chip row per badge with the freya chip metrics), re-mirrors the app
  window's geometry on resize, and appends the same `outline`/`badge`
  paint-log lines the other overlays keep. `show_overlay(None)` drops the
  window; nothing draws after it.
* **Compositing at capture time** — `render_frame` (and, since m85,
  `last_frame`) blends the overlay window's `take_snapshot` bytes onto the
  app frame with straight-alpha source-over. The blend is a no-op outside
  the drawn overlay, so an empty plan leaves the frame byte-identical.
* **The red→green evidence** — `glue/slint/tests/slint_overlay.rs` failed
  first on `Unsupported("inspect overlay (§13)")` (the trait default),
  then greened on: the pixel-diff proof (composited frame differs from
  the bare app, frame digest moves), `DisableOverlay` restoring the bare
  frame byte-for-byte, and the paint log naming the draws (test ids beat
  names — the badges carry `#CounterForm::increment-btn`).

## 3. M85 — trace + video validated on real adapters

* **Trace (egui, iced, slint)** — each adapter's embedded test boots the
  real harness inside the kit [`App`], enables tracing with an explicit
  archive path, drives a real action (click, and fill on slint), and
  finishes the archive. `guiwright_cli::show_trace` renders the scrubber
  HTML next to it; the tests assert the `gwtrace` marker, the adapter
  name from the attach report, and the recorded event vocabulary.
  Files: `egui_embedded.rs` / `iced_embedded.rs` (feature-gated, the CI
  adapter-features job) / `slint_trace_video.rs` (the slint-glue job).
* **Video (egui, masonry, slint)** — the recorder rides the session's
  pump-presented frames through each adapter's `last_frame`:
  egui (wgpu cache, lavapipe/Metal/DX) and slint (the new §10 frame
  cache — one `take_snapshot` per pass sequence) record ≥2 distinct
  frames around a real repaint; masonry (vello_cpu) records the honest
  static-UI contract (its 0.4 test apps carry no callbacks, so the
  hash-equality dedup collapses the settle span — at least the first
  presented frame encodes). All three assert the §10 metadata: non-zero
  artifact, the wall-clock duration spanning the recorded window, and a
  valid container (ffmpeg EBML magic when the binary is present, the
  `video.json` PNG-sequence manifest otherwise).
* **The red→green evidence** — the slint video row failed first with
  `frames == 0` (`last_frame` returned `None`), then greened on the
  frame-cache implementation keyed on the pass sequence; the slint trace
  row was already green (the kit's §12 flow is adapter-agnostic) and
  pins the real-adapter archive content.
* **CI placement** — no new jobs: the egui/masonry rows run in the
  existing `test` job (ubuntu installs lavapipe; macOS/Windows carry
  native wgpu adapters), the iced row in the existing `iced`
  adapter-features job, and the slint rows in the existing `slint-glue`
  runtime job.

## 4. Deliberate scope lines

* The iced embedded test file needed a compile repair this milestone
  carries: the m76 ticker test referenced
  `iced_test::futures::subscription::time`, a path that does not exist in
  iced_futures 0.14.0 (the subscription re-export has no `time`; `every()`
  lives on the backend modules), and the unified backend was
  `thread-pool`, whose `time` module is a stub — the file could not
  compile, blocking the m85 iced trace row. The test now rides
  `backend::default::time::every` with the `smol` feature on the iced
  dev-dependency (both Program impls already bind
  `backend::default::Executor`).
* The slint `last_frame` pixels materialize on demand through the
  canonical `take_snapshot` raster path — the testing backend has no
  presentation loop, so §10's "no re-render" reads as "one snapshot per
  pass sequence, cached across the pump's presented stamps".
* iced video stays unclaimed: its §6.2 row is honest (`last_frame: false`,
  `render_frame: true` on tiny-skia) and the trace row is the §12
  coverage the trio needed; widening it is adapter work beyond this
  milestone's validation scope.
* floem/gpui overlays remain blocked upstream (no overlay surface in the
  headless harness paths; §4.5/§4.7 records unchanged).
