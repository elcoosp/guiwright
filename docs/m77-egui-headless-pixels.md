# M77 — The egui headless pixel path: wgpu offscreen render + buffer readback (§4.1)

**Status:** implemented. The egui adapter's `render_frame` / `last_frame`
rows are real: the pass's `ClippedPrimitive`s render through
`egui_wgpu::Renderer` into an offscreen `RGBA8UnormSrgb` texture, copy
into a map-able buffer, and read back as premultiplied RGBA8 — the
`egui_kittest` shape, no surface required (§11 "windowless"). Frames
render **once per egui pass**, keyed by
`Context::cumulative_pass_nr`; `app.screenshot`, `locator.screenshot`
and video ride the same cache through the existing §15.5 substrates.
GPU-less hosts answer `AdapterError::Unsupported` at the first pixel
demand (the cached probe failure — the masonry precedent). **No wire
change**: the pixels ride the existing `Shot` / `FramePresented`
domains byte-for-byte.

---

## 1. Why now

The m0-era egui adapter shipped the scene digest as its whole visual
story: "xxh3 over tessellated geometry — no GPU readback at M0", with
`render_frame` / `last_frame` explicit `false` rows in the §6.2 matrix.
§4.1 specifies the real shape ("egui: wgpu 30 offscreen readback"; the
arch §4.1 §11 plan: lavapipe in CI), the session's substrates already
consume pixels (`render_once` rejects a frame without them; the video
recorder offers `last_frame` per presented stamp), and every other
pixel-consuming domain — screenshots, traces' frame bins, video — has
been answering `Unsupported` on egui since M0. This record pays the
last adapter gap in the MVP trio: egui now renders, like masonry
always did.

## 2. Design

* **Render-once, keyed by the pass** — the pump renders its own pass's
  pixels into a cache keyed by `cumulative_pass_nr` (§15.4 render-once);
  `last_frame` is a pure cache read (never re-renders — video capture
  hands out the frame's own pixels), and `render_frame` answers cache
  hits for the current pass, running exactly one fresh pass on a miss. A
  caller-requested size swaps the window rect for that pass (the app
  relays out) and restores the adapter's rect afterwards.
* **The backend probe is cached, both ways** — `Instance::new` +
  `request_adapter` + `request_device` happen once; a GPU-less host
  keeps the failure and every pixel row answers `Unsupported` (logged,
  not silent); a GPU host keeps the device/queue/renderer for the
  adapter's lifetime. The stamp semantics do not change: the frame hash
  stays the §10 scene digest (egui has the scene; the pixel buffer rides
  alongside), so traces, video dedup and wait-for-visual-change keep
  their recorded meanings.
* **wgpu 30 beside wgpu 26** — egui-wgpu 0.37 pins wgpu 30; masonry's
  vello pins 26. The two major versions coexist in the graph (each crate
  binds its own); the adapter binds 30 so the device/queue types match
  egui-wgpu's renderer.
* **The readback strips the row padding** — `copy_texture_to_buffer`
  pads rows to 256 bytes; the map strips them into a tight RGBA buffer
  before it enters `FrameRef.pixels` (`Arc<[u8]>`, the §15.5 shape the
  recorder clones).
* **Textures ride the deltas** — font atlases apply through
  `renderer.update_texture` per `ImageDelta` (the pump's old
  `drop_without_applying_deltas` comment said pixel capture "lands in
  M1"; it landed here — deltas apply on GPU hosts, drop on hash-only
  hosts).

## 3. Surface

- `crates/guiwright-adapter-egui/src/pixels.rs` — `PixelBackend`: the
  probe, the offscreen target (recreated on size change), the
  render-encode-map-readback cycle, the pass-keyed cache.
- `crates/guiwright-adapter-egui/src/lib.rs` — caps flip
  (`last_frame`/`render_frame` → true), the `PixelProbe` tri-state,
  pump-time render (video's source), `render_frame` / `last_frame`
  impls, the guarded physical-size conversion.
- `crates/guiwright-adapter-egui/tests/egui_embedded.rs` — the pixel
  tests (§4 below).
- `crates/guiwright-adapter-egui/Cargo.toml` — `egui-wgpu` + `wgpu` 30 +
  `pollster` (the wgpu-ecosystem `block_on` for the sync adapter).

## 4. Tests

- Dual-behaved by host, both branches assert the §6.2 contract:
  `window_screenshot_is_real_pixels_or_gracefully_unsupported` (real:
  800×600, PNG signature, non-trivial payload; GPU-less: the honest
  `Unsupported` row), `pixels_change_after_a_click_on_gpu_hosts` (the
  counter's pixels differ before/after a click — visual-change evidence
  straight from the buffers), `locator_screenshot_crops_the_button`
  (the crop path over real pixels), `caps_advertise_the_pixel_rows`
  (the matrix pin, unconditional).
- This sandbox has no Vulkan/GL (no ICDs, no EGL): the local run covers
  the graceful branch + the caps pin + full compile; the **real branch
  is the CI lavapipe leg** — the existing `test` job already installs
  mesa-vulkan-drivers + `WGPU_BACKEND=vulkan` on ubuntu, macOS/Windows
  run their real GPUs, so the m77 tests exercise real pixels on all
  three legs in CI.
- No budget row: the screenshot readback budget (`screenshot_readback`,
  `m0_measurable = false`) already records the path class; the egui leg
  rides the same §15.1 row.

## 5. Remaining gaps (honest list, §4/§17/§18 plan items)

- **Freya glue** (§17 first expansion wave): `TestingSeam` still
  unbound; the m76 design (runtime-owned harness + read-back) is the
  template.
- **Pixel determinism baselines** (§10): egui's captured frames are
  asserted structurally (size, non-uniformity, click-change) — the
  per-OS PNG baseline + bless flow (§10 "baselines stored per
  platform+scale+renderer") stays its own §17-plan milestone.
- **Slint / Floem / GPUI adapters** (§17): framework-free engines ship;
  platform glue lands downstream (§18 recorded constraints).
- **crates.io publication** (§18): the release workflow packages into
  GitHub releases; publication stays unautomated.
