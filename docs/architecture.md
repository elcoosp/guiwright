# Guiwright — Technical Architecture

**Status:** living design doc — the original v0.1 draft, amended through milestone 19 (records: `docs/m2-*.md` … `docs/m19-*.md`); the §16 parity map and §7 transport list track what shipped
**Date:** 2026-11-06 (drafted), 2026-11-12 (last amended)
**Scope:** a Playwright-grade test & automation driver for Rust GUI frameworks — GPUI, Xilem/Masonry, Floem, Iced, egui, Slint, Freya
**Evidence base:** shallow clones of `zed`, `linebender/xilem`, `lapce/floem`, `iced-rs/iced`, `emilk/egui`, `slint-ui/slint`, `marc2332/freya`, `AccessKit/accesskit`, `rerun-io/kittest` (HEADs of 2026-11-03..06; versions and file paths cited inline). Paths are relative to each repo root.

---

## 0. How to read this document

- §2–§4 are the **reconnaissance deliverable**: what each framework actually offers today, with file-level evidence, and where reality differs from the original plan (`guiwright.md`).
- §5–§14 are the **architecture**: process model, Rust traits, the Guiwright Protocol (GWP), and every Playwright feature mapped to a concrete mechanism.
- §15 is the **performance engineering** chapter (budgets, zero-polling design, capture pipeline, benchmarks).
- §16–§18 cover the Playwright parity map, maturity ranking / MVP order, and risks.

---

## 1. Mission & scope

Guiwright reproduces the entirety of Playwright's developer experience for Rust GUI apps:

- **Locators with auto-waiting** (`app.get_by_role(Role::Button, "Valider").click().await`) with actionability checks (attached, visible, stable, enabled, receives-events) so tests never `sleep()`.
- **Assertions** (`expect(locator).to_have_text(...)`, `to_have_screenshot(...)`) with retry-until-timeout semantics.
- **Test runner integration** (cargo-nextest friendly), fixtures, per-test app isolation, parallelism, sharding, retries, JUnit/HTML reports.
- **Time-travel trace viewer** (zip archive: UI trees, input events, logs, frame captures).
- **Codegen & interactive inspector** (`--inspect` overlay that records interactions as Rust code).
- **Screenshots, video, visual regression** with per-OS baselines and AA tolerance.
- **Virtual clock & mocking** (fast-forward timers/animations, resettable state seams).
- **Headless CI** (no display server) on GitHub Actions, for every supported framework.

Non-goals (v1): driving non-Rust toolkits; recording Android/iOS device video; fuzzing (out of scope, though the harness is fuzz-friendly).

Two deployment modes, one core:

1. **Embedded mode** — Guiwright links into the test binary and drives the app in-process through each framework's existing headless harness (fastest path, zero IPC). This is the `cargo test` experience.
2. **Remote mode** — the app is built with a `guiwright-driver` Cargo feature that embeds a protocol server (UDS by default, TCP/WebSocket optional); external clients (CLI, codegen, viewer, other languages) attach to a *live* app, Playwright-style.

---

## 2. Reconnaissance summary — what the codebases actually offer

### 2.1 Corrections to the original plan

The original plan (`guiwright.md`) assumed: GPUI = winit-ish dispatcher + blade + taffy; Floem/Iced = winit + taffy; Freya = taffy + AccessKit; kittest = the harness to build on. Recon corrects four load-bearing assumptions:

| # | Original assumption | What the code actually shows (evidence) | Consequence |
|---|---|---|---|
| 1 | kittest is the testing harness to reuse | kittest **0.5.0** is now a *pure query library* over `accesskit_consumer` (~680 lines: `State`, `Queryable`, `By` filters). The `Harness` (loop, rendering, snapshots) moved into **`egui_kittest`** inside the egui repo; masonry has its own harness. There is **no auto-wait anywhere** (only a commented-out `Findable` sketch, `kittest/src/query.rs:266-302`). | Guiwright must own the *harness layer* (frame pump, actionability, waits) and can reuse kittest only for tree querying. This is actually good: the missing piece is exactly our differentiator. |
| 2 | GPUI uses blade + needs a Level-2 adapter | GPUI **0.2.2** split into `gpui` core + `gpui_platform/gpui_macos/gpui_linux/gpui_windows/gpui_wgpu` (root `Cargo.toml:99-109`). Blade is **gone** (no `blade` in `Cargo.lock`); rendering is wgpu 30 (`gpui_wgpu`). AccessKit is an **unconditional dependency** (`crates/gpui/Cargo.toml:63`) and headless is a first-class Linux mode (`WindowingRequest::Headless`, `src/platform.rs:293-300`). | GPUI is closer to Level-1 than planned: queries ride the AccessKit tree; input injection uses `Window::dispatch_event` (`src/window.rs:5864`). The hard part is main-thread confinement, not a11y. |
| 3 | Freya uses taffy + Dioxus | Freya **0.5.0-rc.13** uses **torin** (custom pure-Rust layout, no taffy dep — `crates/torin/README.md:11`) and its **own reactive runtime** (`freya-core/src/runner.rs:161`); Dioxus survives only in devtools/hot-reload. AccessKit is non-optional in `freya-core` and `AccessibilityId` *is* `accesskit::NodeId` (`crates/freya-core/src/accessibility/id.rs:9-12`). | Freya is a strong Level-1 citizen with a ready-made headless harness (`freya-testing`, software Skia, PNG output). Locators key on explicit `.a11y_id(...)`. |
| 4 | Iced lacks testing support (Level-2 by hand) | Iced **0.15.0-dev** ships `iced_test` (`Simulator` headless view-driving + `Emulator` full-runtime with deterministic `Mode::{Zen, Patient, Immediate}` and `Preset` boot state), `iced_selector` (`Selector` trait + `Find`/`FindAll` operations producing `Target { bounds, visible_bounds }`), a `Headless` renderer trait with `screenshot()` for both wgpu and tiny-skia, and an **event-injection primitive**: `iced_runtime::Action::Event { window, event }` (`runtime/src/lib.rs:59-65`). But: **no AccessKit, no taffy** (custom layout). | Iced's Level-2 adapter is mostly *pre-built*; Guiwright wraps `Emulator`/`Simulator` + selectors instead of re-implementing. Locators key on `widget::Id::Custom` + text. |

Two more refinements:

- **Masonry** has its own layout engine (not taffy) and its AccessKit tree is **lazy**: it only produces `TreeUpdate`s after `WindowEvent::EnableAccessTree` (`masonry_core/src/render_root.rs:530-538`). The harness (`masonry_testing`) is excellent but does **not** use kittest (TODO at `harness.rs:64-68`) and hard-wires vello_cpu screenshots.
- **Floem** has **no accessibility at all** (0 grep hits) but already ships a `HeadlessHarness` (`src/headless.rs:101`) with a DOM-like event router, taffy 0.9 geometry with public `get_visual_rect`, and hit-testing (`element_id_at`) — yet **no headless pixels** (its `PaintState` never initializes without a GPU surface, `src/window/handle.rs:313-322`).

### 2.2 Versions snapshot (as reconed)

| Repo | Version / HEAD | Windowing | Input vocabulary | Semantic tree | Layout | Renderer | Existing test stack |
|---|---|---|---|---|---|---|---|
| egui | 0.37.0 | winit 0.30.13 (via eframe) | `egui::Event` (synthesizable via `RawInput`) | AccessKit 0.25.2, **always compiled, runtime-toggled** | immediate-mode `WidgetRects` | wgpu 30 / glow; epaint tessellation | `egui_kittest` (harness) + `egui_inspection` (TCP protocol) |
| Xilem/Masonry | 0.4.0 (`dc49687`) | winit 0.30.13 | `ui-events` `PointerEvent`/`KeyboardEvent`, `WindowEvent`/`TextEvent` | AccessKit 0.21 + consumer 0.31 (via masonry 0.4); `WidgetId == NodeId` | custom (LenReq/measure) | vello (wgpu 28) / vello_cpu / skia via `masonry_imaging` | `masonry_testing::TestHarness` (headless, snapshot diffs) |
| Freya | 0.5.0-rc.13 (`c4c27f1`) | winit 0.30.13 | `PlatformEvent` → ragnarok pipeline | AccessKit 0.25.2 (`AccessibilityId = NodeId`) | torin (custom) | Skia (GL/Vulkan/Metal/softbuffer software) | `freya-testing::TestingRunner` (sync, headless, PNG) |
| Iced | 0.15.0-dev (`8ff48c4`) | winit 0.30.13 (iced fork) | `core::Event` + `Action::Event` injection | **none** | custom (`widget::Tree`, `Layout`) | wgpu 30 / tiny-skia; `Headless::screenshot` | `iced_test` (`Simulator`/`Emulator`), `iced_selector`, `iced_tester`, `.ice` |
| Slint | 1.19.0 (`34008e5`) | winit 0.30.2 | core `WindowEvent` via `Window::dispatch_event` | AccessKit 0.25 (winit backend, default-on) + `ElementHandle` (debug-info) | compiler-generated items | femtovg (default) / skia / software (`render(&mut [TargetPixel])`, `take_snapshot`) | `i-slint-backend-testing` + driver test suite + MCP/system-testing servers |
| GPUI | 0.2.2 (zed HEAD) | **custom** per-OS loops (calloop/Wayland/x11rb; not winit) | `PlatformInput` → `Window::dispatch_event` | AccessKit 0.24, unconditional | taffy 0.15 | wgpu 30 (`gpui_wgpu`) / Metal | `test-support`: `TestPlatform`, `TestWindow`, `VisualTestContext`, `render_to_image` |
| Floem | 0.2.0 (`1351ffb`) | **winit fork** `floem-winit 25.10.0` | `ui-events` 0.3 via `WindowEventReducer`; `Event` enum | **none** | taffy 0.9.2 + `BoxTree` spatial index | skia > vello > vger; tiny-skia always available | `HeadlessHarness` + `floem-test` (42 test files) |
| AccessKit | 0.25.2 / consumer 0.39.2 | — | 22 `Action`s, `ActionRequest` | `TreeUpdate` (differential), `NodeId` = u64, `TreeId` = Uuid | — | — | `accesskit_consumer` = filter/hit-test/diff engine |
| kittest | 0.5.0 (`11c1a3f`) | — | — | query layer over `accesskit_consumer` | — | — | `Queryable`/`By` only; no waiting, no rendering |

---

## 3. Integration matrix — the exact hook points

This is the operational answer to *"how do we interact with each framework"*. Details and file evidence in §4.

| Concern | egui | Masonry/Xilem | Freya | Slint | GPUI | Iced | Floem |
|---|---|---|---|---|---|---|---|
| **Driver residency** | in-process (`ctx.run_ui` / egui Plugin) | in-process on `RenderRoot` (headless) or `MasonryState` (live) | in-process on `TestingRunner` (headless) or `EventLoopProxy` (live) | in-process via `set_platform(TestingBackend)` | in-process, main-thread via `dispatch_on_main_thread` | in-process on `Emulator`/`Simulator` | in-process on `HeadlessHarness` / `route_event` |
| **Semantic-tree source** | `FullOutput.platform_output.accesskit_update` (every pass) | `RenderRoot::redraw()` → `TreeUpdate` after `EnableAccessTree` | `AccessibilityTree::process_updates` / `TestingRunner::commit_accessibility()` | `accesskit.rs` adapter (winit) or `ElementHandle` (testing backend) | `PlatformWindow::a11y_tree_update` (every frame when active) | — (build tree from `iced_selector` ops) | — (build tree from view traversal) |
| **Node identity** | `egui::Id` → `Id::accesskit_id()` = `NodeId` | `WidgetId` ↔ `NodeId` (1:1, `widget.rs:668-672`) | `AccessibilityId` = `NodeId` (`.a11y_id` for stability) | `NodeId = component_id << 16 \| item_index` | `NodeId` from `GlobalElementId` hash | `widget::Id` (Unique/Custom) | `ViewId` (slotmap key, !Send) |
| **Input injection** | queue `egui::Event`s into `RawInput` | `process_pointer_event` / `process_text_event` (headless); winit `AsyncAction` user event (live) | `send_event(PlatformEvent)` (headless); `NativeWindowEventAction::PlatformEvent` (live) | `Window::dispatch_event(WindowEvent::…)` | `Window::dispatch_event(PlatformInput)` | `Action::Event { window, event }` via proxy; `UserInterface::update(&[Event])` headless | `HeadlessHarness::dispatch_event` (headless); `ViewId::route_event` (live) |
| **Geometry / hit-test** | `WidgetRects` + `hit_test` (public) | `ctx.window_transform()`, `find_widget_under_pointer` | torin `LayoutNode.area` (absolute) | `item_absolute_position`, `ItemRc::map_to_native_window` | a11y `node_bounds` map; hitboxes | `iced_selector` `Target::bounds/visible_bounds` | taffy + `get_visual_rect`, `element_id_at` |
| **Test IDs** | widget `Id`s (hash chain) | `WidgetTag::named("…")` | `.a11y_id(NodeId)` | element ids (`"Component::elem"`, needs debug info) | `GlobalElementId` / `debug_selector` (test builds) | `widget::Id::Custom("…")` | `debug_name` (informal) |
| **Screenshots** | kittest wgpu readback (CPU-adapter friendly) | harness `render()` via vello_cpu; wgpu surface live | `TestingRunner::render_to_file`; live `post_render_callback` | `Window::take_snapshot` (all renderers) | `Window::render_to_image` (test-support) | `Headless::screenshot` (wgpu + tiny-skia), `window::screenshot` | **gap** — needs tiny-skia offscreen `PaintState` (contribution) |
| **Virtual clock** | `RawInput { time, predicted_dt }` (trivial) | `AnimFrame` events + `animate_ms` | `TestingRunner::poll(step)` ticker; winit `ControlFlow` live | `mock_elapsed_time(ms)` (first-class) | `TestDispatcher::advance_clock` (test-support) | `Emulator` `Mode` determinism; `time` not injectable live | harness-driven; `action.rs` timers |
| **Auto-wait substrate** | `repaint_delay == 0` settle loop | rewrite-pass saturation + signal queue | `sync_and_update()` per step | mock-time tick drains timers | `run_until_parked` / `simulate_scheduled_frame` | frame loop in `Emulator` | `process_update_no_paint` cycle |
| **Perf-relevant quirks** | full a11y tree rebuild per pass | lazy a11y; 4-pass rewrite cap | frame-quantized input buffer | needs `SLINT_EMIT_DEBUG_INFO=1`; semver-exempt internals | main-thread only; `test-support` gating | ids optional; widget `Tree` not `Send` | thread-local global state; winit fork |

---

## 4. Per-framework integration dossiers

Each dossier states: (a) how Guiwright embeds, (b) how the semantic tree is obtained, (c) how input is injected, (d) how frames/pixels are captured, (e) gaps we fill.

### 4.1 egui — the reference adapter (Level 1)

- **Embed:** Guiwright drives one `egui::Context` per session. Events are queued and applied at the next `ctx.run_ui(RawInput { events, time, screen_rect, .. })`. The sanctioned extension surface is an **`egui::Plugin`** (`crates/egui/src/plugin.rs:13-58`: `input_hook(&mut RawInput)` before the pass, `output_hook(&mut FullOutput)` after) — exactly how `egui_inspection`'s `InspectionPlugin` and kittest's `RecordingPlugin` work. egui already ships a TCP driver protocol (`egui_inspection`: `GetTree`, `GetScreenshot`, `ApplyEvents`, `Resize`, `Settle` over MessagePack, default port 5719) — we treat it as prior art and interop target.
- **Tree:** `ctx.enable_accesskit()` (runtime toggle; accesskit 0.25.1 is unconditional). Every `end_pass` emits a **full** `TreeUpdate` on `platform_output.accesskit_update` (`context.rs:3182-3207`); widget `Id`s map losslessly via `Id::accesskit_id()` (`id.rs:163-165`). Because the tree is rebuilt per pass, Guiwright diffs at the consumer (`accesskit_consumer::Tree::update_and_process_changes`) — zero work when nothing changed.
- **Actions:** inbound `ActionRequest`s arrive as `egui::Event::AccessKitActionRequest` and are honored by stock widgets (click sets `FAKE_PRIMARY_CLICKED`, scroll/SetValue handled in `Context::interact`/sliders/text edits). `Node::click_accesskit` works even on non-visible widgets — Guiwright exposes both "pixel click" (synthetic pointer path) and "semantic click".
- **Hit-test:** `egui::hit_test(&WidgetRects, …) -> WidgetHits` is public and answers click/drag targets; `interact_rect` already carries clipping.
- **Pixels:** wgpu headless device without display handle, render to `RENDER_ATTACHMENT|COPY_SRC`, `poll(Wait)`, map buffer → premultiplied `RgbaImage` (`egui_kittest/src/wgpu.rs:191-248`), preferring CPU adapters (lavapipe) for CI determinism with `RendererOptions::PREDICTABLE`. Render exactly once per pass (cache keyed by `ctx.cumulative_pass_nr()`) — double-render corrupts paint callbacks.
- **Clock:** egui has essentially no internal `Instant` use — animations interpolate from `RawInput.time`. Virtual clock = synthesize `time`. kittest additionally nulls animations (`animation_time = 0`, scroll animation off) for determinism; Guiwright exposes both "kill animations" and "advance virtual time" modes.
- **Gaps we fill:** auto-wait (kittest has none — settle loop only), trace, codegen overlay, out-of-process protocol beyond inspection's five ops.

### 4.2 Masonry / Xilem — the closest spiritual sibling (Level 1)

- **Embed (headless):** replicate `masonry_testing::TestHarness`: own a `RenderRoot`, send `WindowEvent::EnableAccessTree` at creation (a11y is lazy!), drive `process_pointer_event` / `process_text_event` / `AnimFrame`, render via vello_cpu (`harness.rs:532-572`). This path is 100 % windowless and byte-deterministic.
- **Embed (live):** two sanctioned seams: (1) `MasonryState` ("if you run Masonry from an external winit event loop … forward events", `event_loop_runner.rs:201-205`, example `xilem/examples/external_event_loop.rs`) — Guiwright can own the `ApplicationHandler`; (2) `EventLoopProxy<MasonryUserEvent>` where `AsyncAction(WindowId, ErasedAction)` is the documented cross-thread channel (`event_loop_runner.rs:41-55`). We add a Guiwright `ErasedAction` payload type the app-driver routes.
- **Tree:** `run_accessibility_pass` emits bounds (border box) + transforms scaled by the factor (`passes/accessibility.rs:86-142`). **`WidgetId` ↔ accesskit `NodeId` is identity** (`From<WidgetId> for accesskit::NodeId`, `widget.rs:668-672`) — locators resolved on the AccessKit tree convert back to widgets with zero bookkeeping (the harness today needs an `unsafe` transmute for lack of a public `NodeId` ctor — we file upstream and mirror the workaround).
- **Input:** real event pipeline = `PointerEvent`/`KeyboardEvent` via `ui_events_winit::WindowEventReducer`; synthetic equivalents already exist in the harness (`mouse_move_to` performs `find_widget_under_pointer` visibility verification, `harness.rs:697-716`). Keyboard fidelity caveat: harness `keyboard_type_chars` commits via IME, not key events — Guiwright sends true `TextEvent::Keyboard` sequences.
- **Pixels:** headless vello_cpu `render() -> RgbaImage`; live path renders through wgpu (vello) with `TextureBlitter` — capture hook is `RenderRoot::redraw()` → `VisualLayerPlan` scenes which we rasterize offscreen.
- **Gaps:** kittest integration is an upstream TODO — Guiwright contributes the query/actability layer instead; rewrite-pass saturation (4 iterations) means batched mutations per frame.

### 4.3 Freya (Level 1)

- **Embed:** `freya-testing::TestingRunner` is a complete sync, windowless harness: software Skia (`raster_n32_premul`), `sync_and_update()` per step, `send_event(PlatformEvent)` injection, `find`/`find_many` node queries, `render_to_file` PNG. Guiwright wraps it 1:1 for embedded mode.
- **Tree:** `AccessibilityTree` maintains `FxHashMap<AccessibilityId, NodeId>` with dirty-node diffing (`accessibility_diff`, `tree.rs:111`); `commit_accessibility()` returns the `TreeUpdate`. **Stable selectors require apps to set `.a11y_id(AccessibilityId(n))`** (auto ids are a process counter). Guiwright docs/codegen steer users to explicit ids, role+label otherwise.
- **Actions:** `AccessibilityTree::handle_action` handles Focus/Blur/ScrollIntoView natively (scroll → synthetic wheel); the rest becomes `EventName::AccessibilityAction` routed to app handlers (`runner.rs:370-381`). Pixel clicks go through `send_event(PlatformEvent::Mouse …)` + ragnarok hit-testing (`is_point_inside` + clip-aware `visible_area().contains`).
- **Live mode:** `LaunchConfig::with_event_loop` (`crates/freya-winit/src/config.rs:520`) lets Guiwright own the `EventLoop<EventLoopProxy<NativeEvent>>` and post `NativeWindowEventAction::PlatformEvent` directly; frame capture via `window.post_render_callback` (drained after render, before present; proven by `crates/freya-winit/tests/smoke.rs:61-91`).
- **Gaps:** `press_key` helper sends KeyDown only (we drive raw `PlatformEvent::Keyboard` with modifiers+KeyUp); headless = single window; input is frame-quantized by ragnarok's `EventBuffer` (respect press→sync→move→release ordering).

### 4.4 Slint (Level 1, two doors)

- **Door A — testing backend (embedded/headless):** `i-slint-backend-testing` replaces the whole `Platform`: `init_no_event_loop()` (mock time, no threads), `TestingBackend` windows registered in a thread-local registry, injection via core `Window::dispatch_event(WindowEvent::PointerMoved/Pressed/Released/Keyboard…)` — the *same* code path real winit events take. Element queries via `ElementHandle` (`find_by_accessible_label`, `find_by_element_id`, `absolute_position`, `invoke_accessible_default_action`, `single_click`/`drag`/`scroll`) — requires `SLINT_EMIT_DEBUG_INFO=1` at app compile time, so Guiwright ships a `guiwright-build` helper that sets `CompilerConfiguration::with_debug_info(true)`.
- **Door B — real winit app (remote/live):** the winit backend's AccessKit adapter (feature `accessibility`, on by default; accesskit 0.25) maps the item tree to NodeIds (`component_id << 16 | item_index`), advertises bounds (physical px, `accesskit.rs:784-793`), and executes `ActionRequest`s (Click → the widget's `clicked`, plus Focus/Increment/SetValue/ReplaceSelectedText) deferred on the UI loop. External input cannot be injected at winit level; the protocol is either AccessKit actions (semantic) or Guiwright's in-process plugin.
- **Pixels:** every renderer implements `take_snapshot() -> SharedPixelBuffer<Rgba8Pixel>` (`internal/core/api.rs:844-852`); the software renderer can render into any CPU buffer (`render(&mut [TargetPixel], stride)`, partial dirty regions) and `MinimalSoftwareWindow` is a no-window-manager `WindowAdapter` — the canonical headless raster path. Headless = `SLINT_BACKEND=headless[-software|-skia]`.
- **Clock:** `mock_elapsed_time(ms)` advances the animation driver, fires timers, runs change handlers, instantiates repeaters — the best virtual-clock primitive in the ecosystem; Guiwright maps `Clock::FastForward` onto it directly.
- **Caveats:** internal crates are semver-exempt → Guiwright pins `=1.19.x` per release train (CI matrix per Slint version); license is GPLv3/royalty-free/commercial — Guiwright is a *tool* linking into the test/driver binary only, keeping app-side glue feature-gated; MCP + system-testing protobuf servers are prior art for protocol shape.

### 4.5 GPUI (Level 1 queries + Level 2 input — hybrid)

- **Embed:** everything is main-thread-confined (`App`/`Window`/`Entity` are `Rc`-based; `ForegroundExecutor: !Send`). The protocol server runs on a side thread; commands are marshalled via `PlatformDispatcher::dispatch_on_main_thread` / `SendForegroundExecutor`; results return through oneshot channels. For full control Guiwright injects a custom `Platform` via `Application::with_platform` — precisely what `TestPlatform` does (`src/platform/test/platform.rs:31`).
- **Input:** `Window::dispatch_event(PlatformInput)` (`window.rs:5864`) is the canonical seam; real backends feed it through `platform_window.on_input` (`window.rs:2241-2252`), so synthetic events are bit-identical to OS input. `VisualTestContext` demonstrates the full vocabulary: `simulate_mouse_move/down/up/click`, `simulate_keystrokes("cmd-shift-p enter")`, generic `simulate_event<E: InputEvent>` (`app/test_context.rs:935-1068`).
- **Tree:** AccessKit is unconditional; per-frame `TreeUpdate`s flow through `PlatformWindow::a11y_tree_update` with node bounds recorded per `NodeId` (`A11y.node_bounds`, `window/a11y.rs:150`; built in `element.rs:428-462`). Node ids derive from `GlobalElementId` (stable iff elements are `.id()`-ed). `Action::Click` on the a11y path synthesizes real mouse events (`window.rs:7348-7366`) — semantic actions degrade gracefully to pixel actions.
- **Geometry:** release builds expose rects only via the a11y tree; test builds add `debug_bounds` (`Frame.debug_bounds`, filled by `.debug_selector(...)`, `elements/div.rs:908-924`). Guiwright relies on a11y rects, and contributes `debug_selector` adoption for element-screenshot baselines.
- **Pixels/headless:** `WindowingRequest::Headless` (Linux) keeps layout/input without drawing; `PlatformHeadlessRenderer::render_scene_to_image` + `Window::render_to_image` (feature `test-support`) provide RGBA readback; `HeadlessAppContext::capture_screenshot` is the packaged path (`app/headless_app_context.rs:164-171`). GPUI's `test-support`/`bench-support` features gate all of this — Guiwright's adapter is `#[cfg(feature = "guiwright-driver")]` in the *app*, enabling `gpui/test-support` transitively.
- **Risks:** platform crates churned recently (split at 0.2.2); macOS visual tests park windows off-screen and capture via ScreenCaptureKit (`app/visual_test_context.rs`) — we adopt that for macOS baselines.

### 4.6 Iced (Level 2, mostly pre-built)

- **Embed:** `iced_test::Emulator` drives a real `Program` (update + subscriptions + tasks) headlessly with determinism modes (`Zen` = settle-to-quiescence, `Patient`, `Immediate`) and `Preset` reproducible boot state; `Simulator` drives a bare `Element` for widget-level tests. Injection = `UserInterface::update(&[core::Event])`. Screenshots via the `Headless` renderer trait (`screenshot(size, scale, background)`), backend chosen by `ICED_TEST_BACKEND` (wgpu-30 offscreen readback or tiny-skia CPU).
- **Locators:** `iced_selector` already provides the `Selector` trait (matches by `&str` text, `widget::Id`, `Point`, closures, focus) and `Find`/`FindAll` `Operation`s returning `Target { id, bounds, visible_bounds }` with scrollable-aware clipping. Guiwright maps `get_by_text`/`get_by_id` onto it and adds role semantics via a contributed widget-kind registry (button/text-input/checkbox map to `Tag`/widget type introspection).
- **Live mode:** injection via `iced_runtime::Action::Event { window, event }` pushed through the event-loop `Proxy` (pattern proven at `winit/src/lib.rs:1770`; the `Proxy` wraps winit's proxy with 100-slot backpressure). Read-back via `Task`/`task::effect(Action::Widget(op))` with an `Operation` that ships `{id, bounds, visible_bounds, text}` over an mpsc channel — operations run on the loop thread, the channel is the safe boundary. Window capture via `window::screenshot(id)`.
- **Gaps:** no a11y tree (nothing to piggyback — Guiwright builds its logical tree from selector ops); widget `Id`s are optional → text/geometry fallbacks matter; snapshot hashes are renderer- and font-dependent (baselines must record `renderer.name()` and load fonts via `Settings.fonts`); `iced_tester`'s `.ice` format stays experimental — Guiwright does not depend on it.

### 4.7 Floem (Level 2, needs two contributions)

- **Embed:** `HeadlessHarness` (`src/headless.rs:101`) already simulates the whole style → taffy layout → box-tree commit cycle and dispatches DOM-like `Event`s (capture/target/bubble; `RouteKind::Spatial` hit-tests through the `BoxTree` spatial index). Helpers exist for click/dblclick/touch/scroll/keyboard; 42 in-repo tests define the patterns.
- **Locators:** no a11y and no test ids. Public geometry is solid (`ViewId::get_visual_rect/origin`, `element_id_at(x,y) -> ElementId path`). Guiwright contributes: (1) a first-class **`test_id(...)`** decorator (mirroring `debug_name`) registered in a global id→`ViewId` map, and (2) role metadata on core views (button/text-input/checkbox) so the logical tree can carry AccessKit-compatible roles even without AccessKit.
- **Input:** headless = `dispatch_event`; live = `ViewId::route_event(event, RouteKind)` (public, queued via update messages) or `ext_event` triggers + the `EVENT_LOOP_PROXY` static (`app/mod.rs:39`). Note winit is a **fork** (`floem-winit`: `Window` is a trait, `proxy_wake_up` semantics) — Guiwright builds against the fork, never crates.io winit.
- **Pixels — the gap:** headless `PaintState` never initializes (`window/handle.rs:313-322`), so no headless screenshots today. Contribution: a tiny-skia offscreen `PaintState` (tiny-skia + softbuffer are already compiled in; `FLOEM_FORCE_TINY_SKIA=1` proves the renderer path). Live capture exists via the inspector's `Capture` path (`window/handle.rs:852` → `Renderer::finish() -> Option<ImageBrush>` when `begin(true)`).
- **Clock:** harness-driven timers (`action.rs`) — Guiwright pumps `Event::Window(WindowEvent::UpdatePhase)` ticks with virtual time.

### 4.8 Foundation: AccessKit + kittest (what we build on)

- **Data model:** `TreeUpdate { nodes: Vec<(NodeId, Node)>, tree: Option<TreeInfo>, tree_id, focus }` — differential, must be applied in order; `NodeId` is an app-assigned `u64` stable within a tree; `TreeId` is a Uuid (`TreeId::ROOT` = nil). Bounds are window-relative **physical pixels** after transform resolution (`accesskit/src/lib.rs:2512-2537`) — Guiwright never multiplies by scale factor itself.
- **Consumer superpowers we reuse (do not re-implement):** incremental updates with **change notifications** (`ChangeHandler::node_added/updated/focus_moved/removed`, `accesskit_consumer/src/tree.rs:581-586`) — this is the substrate for event-driven auto-waiting; `Filter` API + `common_filter` (excludes hidden/graft/clipped subtrees); label resolution incl. `labelled_by` and descendant-`Label` fallback; `hit_test`/`node_at_point`; `supports_action`/`is_disabled`/`toggled`.
- **kittest 0.5:** reuse `Queryable`/`By` semantics (get panics on 0 or ≥2 matches; label queries return the labelled widget — `getByLabelText` parity) as our query vocabulary, but not as the harness.

---

## 5. System architecture

### 5.1 Process & threading model

The decisive recon fact: **every framework's widget tree is main-thread-confined and non-`Send`** (egui `Context` aside, masonry `RenderRoot`, freya `Runner`, floem thread-local storage, iced `widget::Tree`, GPUI `Rc` entities). Therefore:

```
┌────────────────────────── App process ───────────────────────────┐
│                                                                   │
│  UI thread                          Driver thread(s)              │
│  ┌──────────────────────┐  frame   ┌────────────────────────┐    │
│  │  Framework app       │  boundary│  guiwright-driver      │    │
│  │  + FrameworkAdapter  │◀────────▶│  - session/host        │    │
│  │    (runs ON this     │  channels│  - tree service        │    │
│  │     thread, always)  │          │  - action scheduler    │    │
│  └──────────────────────┘          │  - trace recorder      │    │
│            ▲                       └───────────┬────────────┘    │
│            │ hook (a11y TreeUpdate,            │ UDS / TCP / WS  │
│            │  present callback, Plugin)        ▼                 │
│  ┌─────────┴──────────┐            ┌────────────────────────┐    │
│  │ Frame/Tree taps    │            │ Clients: kit, CLI,     │    │
│  └────────────────────┘            │ viewer, codegen, REPL  │    │
│                                    └────────────────────────┘    │
└───────────────────────────────────────────────────────────────────┘
```

- The **adapter** executes only on the UI thread, at frame boundaries. The **host** (server) owns state that may cross threads: logical tree snapshots, session bookkeeping, trace writer, client sockets.
- Rendezvous discipline: commands are queued to the UI thread with a waker specific to the framework (`egui::Context::request_repaint`, winit `EventLoopProxy::wake_up`/`send_event`, slint `invoke_from_event_loop`, GPUI `dispatch_after(0)`, iced `Proxy::send_action`, floem `send_proxy_event`, freya proxy + `PollRunner`). The adapter applies the command **inside the next frame**, then replies. This guarantees we never touch widgets off-thread and never observe torn trees.
- **Embedded mode** short-circuits the transport: `guiwright-kit` calls the adapter directly in the test's process/thread; the same trait boundary is exercised, so embedded tests and remote sessions behave identically.

### 5.2 Crate workspace

```
guiwright/
├── guiwright-core        # framework-agnostic brain: tree model, Selector, actionability
│                         #   state machines, Expect/assertions, virtual clock semantics
│                         #   (no framework deps; hot paths allocation-light)
├── guiwright-proto       # GWP codec: postcard schema, framing, versioning, transports
│                         #   (UDS, TCP, WS, in-process duplex), backpressure, binary frame side-channel
├── guiwright-driver      # in-app host: session manager, tree service, action scheduler,
│                         #   frame service, trace recorder, clock service
├── guiwright-adapter-*   # standalone per-framework adapter crates (egui, masonry,
│                         #   iced, freya, slint, floem, gpui) — not driver features;
│                         #   iced/freya carry feature-gated harness glue, the rest ship
│                         #   framework-free engines + recorded downstream contracts
├── guiwright-kit         # user-facing: App, Locators, #[guiwright::test], fixtures,
│                         #   expect!, embedded Harness (mirrors Playwright's fixture system)
├── guiwright-shot        # framework-free screenshot stack: PNG codec, YIQ diff,
│                         #   per-OS baselines + the GUIWRIGHT_BLESS flow
├── guiwright-macros      # proc macros: #[guiwright_kit::test], #[seam], register_seams!
├── guiwright-trace       # recorder (lock-free SPSC ring → zip writer thread), schema v1
├── guiwright-viewer      # self-contained HTML trace viewer (loads .zip, replays)
├── guiwright-inspect     # inspector overlay injection + interaction→code recorder
├── guiwright-cli         # `guiwright repl|test|show-trace|bless|install-deps`,
│                         #   nextest/report glue, sharding
└── guiwright-bench       # criterion suites + CI perf guards (§15.7)
```

Dependency rule: `core` depends only on `accesskit` + `accesskit_consumer` (the one universal substrate). Adapters depend on core + their framework. The app depends on its adapter crate (the framework stack stays behind the adapter crate's own feature — `guiwright-driver` ships framework-free) — nothing else leaks into release builds.

### 5.3 Cargo integration (the `--features playwright-driver` idea, made precise)

```toml
# app Cargo.toml
[dependencies]
guiwright-driver = { version = "0.1", optional = true }

[features]
gw-egui   = ["dep:guiwright-driver", "dep:guiwright-adapter-egui"]
gw-gpui   = ["dep:guiwright-driver", "dep:guiwright-adapter-gpui", "gpui/test-support"]
```

Launch modes: `GUIWRIGHT_DRIVER=uds:///tmp/app.gwp ./app` (remote), or `cargo test` with `guiwright-kit` (embedded). Release builds without the feature compile the driver to nothing (`#[cfg]`-gated taps).

---

## 6. Core traits (Rust)

### 6.1 Adapter — runs on the UI thread

```rust
/// Implemented once per framework. All methods execute on the UI thread,
/// marshalled by the host at frame boundaries (§5.1).
pub trait FrameworkAdapter: 'static {
    fn meta(&self) -> &AdapterMeta;              // name, caps, quirks
    fn events(&mut self) -> &mut EventSink;      // drained by host each frame

    /// Pump frames until the app is quiescent or `deadline` hits.
    /// Returns what happened (frames presented, timers fired, exit requested).
    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome;

    /// Latest semantic-tree diff (Level-1 adapters). `None` = unchanged.
    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate>;

    /// Level-2 fallback: geometry + role/label polling for logical-tree building.
    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff>;

    /// Apply a semantic action on a resolved node (click / set_value / focus / …).
    /// Adapters translate to AccessKit ActionRequests or framework-native paths.
    fn act(&mut self, node: NodeKey, action: SemAction) -> ActionOutcome;

    /// Inject synthetic input through the framework's REAL event pipeline.
    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError>;

    /// Last presented frame (no re-render). Cheap: backends keep the scene/buffer.
    fn last_frame(&mut self) -> Option<FrameRef>;

    /// Render on demand (headless/offscreen-capable adapters only).
    fn render_frame(&mut self, size: Option<Size>) -> Result<FrameRef, AdapterError>;

    /// Virtual clock (adapter-specific precision; see §14).
    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError>;
}
```

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum NodeKey {
    /// Level-1 canonical identity.
    AccessKit { tree: accesskit::TreeId, node: accesskit::NodeId },
    /// Level-2 framework identities (normalized into core's handles).
    Iced   { window: iced::window::Id, widget: iced::widget::Id },
    Floem  (floem::ViewId),
    Masonry(masonry_core::WidgetId),
    Gpui   (gpui::GlobalElementId),
    Slint  { component: u64, item: u32 },
}
```

### 6.2 Capability matrix per adapter

| Capability | egui | masonry | freya | slint | gpui | iced | floem |
|---|---|---|---|---|---|---|---|
| `tree_update` (a11y diff) | ✅ | ✅ (after `EnableAccessTree`) | ✅ | ✅ (winit a11y) / ❌ (testing backend → logical) | ✅ | ❌ → `logical_snapshot` | ❌ → `logical_snapshot` |
| `input` (real pipeline) | ✅ RawInput | ✅ | ✅ PlatformEvent | ✅ dispatch_event | ✅ dispatch_event | ✅ Action::Event / UI::update | ✅ dispatch_event/route_event |
| `last_frame` w/o re-render | ✅ (cached by pass nr) | ✅ (VisualLayerPlan scenes) | ✅ post_render_callback | ✅ take_snapshot | ✅ rendered_frame (test-support) | ✅ window::screenshot | ⚠️ capture path only |
| `render_frame` headless | ✅ wgpu offscreen | ✅ vello_cpu | ✅ software Skia | ✅ software/skia | ✅ PlatformHeadlessRenderer | ✅ Headless::screenshot | ❌ → contribute tiny-skia |
| `clock` | ✅ (RawInput.time) | ✅ (AnimFrame) | ✅ (ticker) | ✅✅ (mock_elapsed_time) | ⚠️ test-support dispatcher | ⚠️ Emulator modes | ⚠️ harness timers |

### 6.3 Client-side API (guiwright-kit, embedded & remote share it)

```rust
use accesskit::Role;
use guiwright_kit::App;

// Embedded mode (§5.1): the kit calls the adapter in-process, fully
// synchronous — every call pumps the session loop and auto-waits.
let mut app = App::launch("my-test", my_adapter);
app.wait_for_settle();

app.get_by_role(Role::Button, "Save").click()?;           // actionability-gated
expect(app.get_by_text("Welcome, Ada")).to_be_visible()?; // retry-until-timeout

// Multi-window (§16 v0.4): aim one window target; the locators born
// from the handle resolve only inside its subtree.
let preview = app.window(&w2_handle)?;
preview.get_by_role(Role::Button, "Save").click()?;
```

---

## 7. Guiwright Protocol (GWP)

CDP-inspired: sessions → targets (windows) → domains; request/response + server-push events; versioned, binary-first.

- **Transport:** UDS (default), TCP (CI containers + other-language clients), WS (the viewer live-mode door — served in Rust since M15), in-process duplex pipes (the embedded door — served since M16, with the public `Transport` seam turning the transport-blind bring-up into a downstream extension point: any listener that yields byte streams serves the byte-identical conversation). Framing: 4-byte length + postcard-encoded body (schema-versioned enum; `#[serde(skip)]`-free, explicit `V1` variants so old clients get clean errors). Large payloads (frames, trees, traces) go on a **side channel as length-prefixed postcard+LZ4 blobs** to keep command latency independent of payload size.
- **Multiplexing:** monotonic `request_id`; events tagged with `seq` per topic; every mutation response carries the `frame_seq` it was applied at (clients can await that exact frame — no polling).
- **Events (M12, shipped shape; logs M14):** the sketch's `TreeSubscribe` landed as the append-only `Subscribe { On { kinds } | Off }` domain (discriminant 15); pushed events ride the response envelope as `ResponseV1::Event(EventV1)` (17) — every client demultiplexes one enum, a connection that never subscribes keeps the byte-exact pre-M12 conversation, and the tree-dump domain (M11/M13) is the read-side counterpart (`Snapshot::Full` / `Snapshot::Filtered`). The `Log` kind rides the per-session `tracing` tap (`ServerOptions::log_tail`): a thread-local dispatcher on the session thread feeds a bounded live tail whose records stream as `EventV1::Log` — one `logs.ndjson`-shape JSON line each; a session without the tail still answers the kind with a typed `Unsupported`.

```
// Handshake
Hello { proto_version, agent, adapter: AdapterMeta, caps: CapSet }
  → Welcome { session_id, tree_seq, frame_seq, clock: ClockState, windows: [WindowInfo] }

// Domain: tree
TreeSnapshot { window, filter: Option<By> }        → TreeSnapshotV1 { seq, nodes, focus }
TreeSubscribe { topics: [TreeChanged, FramePresented, Logs] }
  ⇒ Evt::TreeChanged    { seq, diff: DiffRef }      // incremental; DiffRef = side-channel blob
  ⇒ Evt::FramePresented { seq, wallclock_us, hash } // hash: cheap visual-change signal
  ⇒ Evt::Log            { record: LogRecord }

// Domain: locate & act
Resolve  { window, selector: SelectorV1, timeout_ms }        → Resolved { handle, rect, gen }
Action   { handle, action: SemActionV1, opts: ActionOpts }   → ActionDone { frame_seq, detail }
Input    { window, events: Vec<InputEventV1> }               → InputQueued { frame_seq }

// Domain: render
Screenshot { target: Window|Element(handle), opts }          → Shot { blob_ref, meta }
Video      { Start(opts) | Stop }                            → VideoFile { path }

// Domain: clock & mock
Clock { InstallVirtual | FastForward(ms) | RunFor(ms) | Pause | Resume | SetTime(t) }
Mock  { InstallSeam { id, response } | Clear(id) | ResetState { scope } }

// Domain: trace & inspector
Trace { Start(opts) | Stop | Attach(stream) }
Inspect { EnableOverlay | Highlight(handle) | StartRecording | StopRecording }
```

Interop notes: `egui_inspection`'s `Request` set is a strict subset of GWP's Render/Input/Tree domains — the egui adapter can *serve* both protocols from one plugin. Slint's MCP tools (`click_element`, `take_screenshot`, …) map onto the same domains, informing naming.

---

## 8. Locator engine & auto-waiting

### 8.1 Selector model

```rust
pub enum Selector {
    Role   { role: Role, name: Option<NameMatch> },        // getByRole
    Label  (TextMatch),                                    // getByLabel (labelled_by aware)
    Text   (TextMatch),                                    // getByText
    TestId (String),                                       // getByTestId  (a11y_id / widget::Id /
                                                           //   WidgetTag / test_id / debug_selector)
    Value  (TextMatch),                                    // getByValue
    All    (Vec<Selector>),                                // chained descendant
    Filter { source: Box<Selector>, has_text: Option<TextMatch>,
             has_not: Option<Box<Selector>>, nth: Option<Nth> },
}
pub struct TextMatch { pub text: TextPattern, pub exact: bool, pub ignore_case: bool }
```

### 8.2 Resolution pipeline (performance-shaped)

1. **Per-frame indexes**, maintained incrementally from `ChangeHandler` diffs (not rebuilt per query): role+name → node list; `TestId` → node (O(1) hash); normalized text → nodes (incremental trie refreshed only on text-run changes). L2 adapters rebuild their `LogicalTreeDiff` from selector `Operation`s/`ViewId` walks, same index structures.
2. **Resolve** = index lookup + filter application (`accesskit_consumer::Filter`), returning `(NodeKey, generation)`. Generations guard against ABA problems when ids are recycled (floem slotmap keys, iced `Unique` ids).
3. **Uniqueness policy** mirrors Playwright/kittest: strict mode — resolve fails on ≥2 matches unless `first()/nth()/last()` or `.filter()` narrows.

### 8.3 Actionability checks (the anti-flakiness core)

Before every action, per target node, in order — each check is **event-driven where possible**:

| Check | Mechanism (L1) | Mechanism (L2) |
|---|---|---|
| Attached | node present in consumer tree | id in logical tree |
| Visible | `common_filter` (hidden/clipped excluded) + `bounding_box().area > 0` | `visible_bounds.area > 0` (`iced_selector` / floem clip-aware rects) |
| Stable | rect identical across **2 consecutive `FramePresented` events** (animation finished) | same, on adapter frame events |
| Enabled | `!node.is_disabled()` | widget state op (`is_disabled`) |
| Receives events | `accesskit_consumer::hit_test` at the click point resolves to the node or a descendant (occlusion check) | framework hit-test: egui `hit_test`, floem `element_id_at`, masonry `find_widget_under_pointer`, GPUI `Frame::hit_test`, iced via point selector |

- **Event-driven waiting:** the waiter registers with the tree service and is woken by `TreeChanged`/`FramePresented` diffs; there is **no sleep-poll loop** (§15.3). A frame-cadence fallback (one check per presented frame, ≤60 Hz) covers L2 adapters that lack diffs.
- **Timeout & retries:** default `timeout = 5 s` (configurable per action/locator/global); every retry re-reads the *changed* regions only; on timeout the error embeds the last tree snapshot + trace link (Playwright-style error report).
- **Force mode:** `force(true)` skips stability/hit-test checks (parity with Playwright).

### 8.4 Action translation

`SemAction` maps per adapter, preferring the framework's *own* semantics (most robust) with a synthetic-input fallback (most realistic):

- `Click` → AccessKit `ActionRequest{Click}` (egui stock widgets honor it; freya routes to `AccessibilityAction`; masonry → `AccessEvent` → widget action; slint → `AccessibilityAction::Default` → `clicked()`; GPUI → built-in synthesis to real mouse events) **or** pointer down/move/up through the real input pipeline (for coordinates, hover states, drag chains).
- `SetValue/Check/Select` → AccessKit `SetValue`/`Increment/Decrement` where supported (egui sliders/text edits, slint) else keyboard-typed input into the focused editable.
- `Hover/Move/Drag/Scroll` → always the **real input pipeline** (they are not semantic actions; pointer fidelity matters).

---

## 9. Input pipeline

- **Event vocabulary:** Guiwright's `InputEventV1` normalizes pointer (down/up/move/enter/leave/cancel, buttons, modifiers, click-count), keyboard (physical key, modified key, text, repeat, modifiers), wheel (lines/pixels), IME (preedit/commit), window (resize/scale/focus). Adapters translate to: `egui::Event`, masonry `PointerEvent`/`TextEvent`, freya `PlatformEvent`, slint `WindowEvent`, GPUI `PlatformInput`, iced `core::Event`, floem `Event`.
- **Gesture synthesis:** click = move→down→(settle)→up; drag = down→interpolated moves (ratchet ~5 px/tick, mirroring slint's flickable-aware drag pacing)→up; keyboard = modifiers-changed→key down→(text)→key up. Each adapter's known quirks are encoded here (freya's frame-quantized `EventBuffer`, masonry's IME-vs-keys distinction, iced's `text` riding `KeyPressed`).
- **Coordinates:** all client-side coordinates are logical px relative to the window; adapters convert once (they own scale factor). AccessKit bounds are already physical — the tree service stores both and documents which space a rect is in.
- **Fidelity rule:** synthetic events enter at the same choke point as OS events on each framework (§3 row "Input injection") so hover/focus/capture state machines behave identically in tests and production.

---

## 10. Frames, screenshots, video, visual regression

- **Frame taps:** adapters register a present-time hook (egui pass end; winit `RedrawRequested` paths in masonry/iced/freya/slint; GPUI `on_request_frame`; floem `render_frame`). The tap emits `FramePresented { seq, hash }`; the **hash** (xxh3 of a strided sample of pixels, or scene-digest where available) powers "wait for visual change" without shipping pixels.
- **Screenshot paths** (see §3): prefer `last_frame` (no re-render, no state disturbance); `render_frame` for headless on-demand. Readbacks are async buffer maps with a small ring of 2–3 map buffers so GPU→CPU copy overlaps the next frame (§15.5). Output: PNG (lossless) or WebP-lossless for trace frames.
- **Element screenshots:** crop from the window frame by the node's clip-aware rect (`visible_bounds`); never re-render. Popups/multi-layer: frameworks expose layer info (masonry `VisualLayerPlan` layers, egui layers/popups, freya overlay layers) — capture composites the layers intersecting the rect.
- **`expect(...).to_have_screenshot(name)`:**
  - Baselines stored per platform+scale+renderer in `tests/screenshots/<name>/<os>-<arch>-<renderer>.png` (iced's renderer-suffix lesson).
  - Diff: pixelwise with per-pixel threshold (max weighted squared YIQ distance — adopt egui_kittest/dify semantics) **plus** `max_failed_pixels` budget; config in `Guiwright.toml` with per-OS `[mac]`/`[windows]`/`[linux]` overrides (we inherit kittest.toml's shape and its *parser*, avoiding the documented `[macos]` README bug).
  - `GUIWRIGHT_BLESS=1` to update; `force` updates all. Failing runs emit `<name>.new.png` + `<name>.diff.png`.
  - Determinism kit per adapter: disable cursor blink/scroll animations (egui), fixed test font (slint), seeded dispatchers (GPUI/masonry), `PREDICTABLE` render options, CPU adapter preference in CI.
- **Video:** the trace recorder doubles as video (frame ring → ffmpeg pipe, precedent: kittest's `RecordingPlugin`/`KITTEST_RECORD`). Frame pacing is bounded by backpressure: if the encoder lags, frames are dropped by hash-equality (static UI compresses to keyframe + no-ops).

---

## 11. Headless & CI

| Framework | Headless mechanism | CI notes |
|---|---|---|
| egui | kittest-style: no window at all; wgpu offscreen, CPU adapter preferred | no xvfb needed; `WGPU_BACKEND=gl/vulkan` + lavapipe |
| Masonry/Xilem | `RenderRoot` + vello_cpu (windowless by construction) | none |
| Freya | `TestingRunner` (software Skia, no winit) | none |
| Slint | `SLINT_BACKEND=headless[-software|-skia]` via `TestingBackend`; `take_snapshot` | `SLINT_EMIT_DEBUG_INFO=1` at build |
| GPUI | `WindowingRequest::Headless` (Linux) + `PlatformHeadlessRenderer`; macOS: off-screen window + ScreenCaptureKit (visual tests) | needs `test-support`; Linux headless is native |
| Iced | `Emulator`/`Simulator` + `Headless::screenshot` (`ICED_TEST_BACKEND`) | none |
| Floem | `HeadlessHarness`; pixels via contributed tiny-skia `PaintState` | tiny-skia = CPU only, no GPU deps |

Guiwright CLI `install-deps` provisions CI (apt: libwayland, xkbcommon, lavapipe/mesa; docs per matrix). GitHub Actions matrix: {ubuntu-latest (software GL), macos-latest (Metal baselines), windows-latest} × {egui, masonry, iced, slint, freya, floem, gpui} — baselines generated per-OS per §10. Parallelism: **process-per-test** (nextest model) matches every framework's process-global state (§18) and gives crash isolation; Guiwright adds session-level sharding over the protocol (multiple apps per host, N windows per session).

---

## 12. Trace viewer (time travel)

**Archive:** `guiwright-<test>-<ts>.zip` (schema `gwtrace/v1`, versioned & forward-compatible):

```
meta.json          app, adapter, versions, os, seed, resolution, scale, settings
events.ndjson      input events, semantic actions, protocol calls (amortized, seq'd)
trees/<seq>.bin    logical-tree snapshots (postcard+LZ4) every N frames
trees/diff.bin     incremental diffs between snapshots (ChangeHandler output)
frames/<seq>.png   framebuffer captures (hash-deduped: identical frames skipped)
logs.ndjson        `tracing` subscriber output (frameworks already use tracing)
clock.bin          virtual-time ledger (install/fast-forward history)
index.bin          frame_seq → offsets (O(1) random access)
```

- **Recorder:** lock-free SPSC ring on the UI thread → writer thread (LZ4, ring buffer with sliding-window eviction; cap default 256 MB, oldest-first eviction). Frames captured at a configurable rate (default: every frame up to 60 fps, hash-deduped).
- **Viewer:** self-contained `trace.html` (plain HTML + vanilla JS; the WASM viewer is §12's full vision): timeline scrubber (frames), tree inspector at any point (resolved from snapshot+diffs), input replay overlay (ghost cursor/keys), log view, protocol calls; "open on failure" deep-links from assertion errors (`guiwright show-trace …`). Live mode (the v0.6 row of §16) shipped with M18: the transport-side client seam (`ClientConn`, the client half of `Handler`) plus the viewer's `LiveFeed`/`LiveTimeline` — a running session over any door (UDS/TCP/WS/embedded) feeds the same scrubber model the archive path builds, dump-per-change, event-fed frame axis. The `trace.html` emitter for live captures stays a follow-on (the §12 archive is still the file format); the kit's trace flow still archives first.

---

## 13. Codegen & inspector (`--inspect`)

- **Overlay:** adapter-level composition on top of the app's scene at present time (egui: painter layer after the pass; masonry/freya: overlay layer — both have native overlay/layer stacks; GPUI: `debug_bounds`-style overlay element; slint: top-level overlay window; iced: overlay via `Operation`-returned rects drawn by a wrapper program). Overlay draws hover outlines + role/TestId badges from the current tree indexes.
- **Recording:** interactions (semantic + input) are captured at the adapter tap, *not* by hooking app callbacks — so codegen works on unmodified apps. Each action is emitted as kit code against the **minimal stable selector** chosen by a cost model: `TestId` > role+accessible-name > label > text (with `.filter()` narrowing on ambiguity). The recorder runs the selector-uniqueness check live (strict-mode warnings as you click).
- **Interactive session:** REPL `codegen start | stop` records while you drive the attached app (no standalone subcommand) with the driver + overlay, and streams generated code to stdout/editor; a REPL (`guiwright repl`) attaches to a live session for ad-hoc locators/eval — the debugging loop that makes Playwright pleasant.

---

## 14. Virtual clock & state mocking

- **Clock service:** `InstallVirtual` swaps the adapter's time source where the framework allows *clean* injection:
  - egui: synthesize `RawInput.time/predicted_dt` (egui has no internal wall-clock in animation paths) — full control.
  - Slint: `mock_elapsed_time` (advances animations, timers, change handlers, repeaters) — first-class.
  - Masonry: synthetic `WindowEvent::AnimFrame(delta)` ticks + harness scheduler.
  - Freya: `TestingRunner::poll(step, duration)` ticks the `RenderingTicker`; live mode: proxy-time + ticker notify.
  - GPUI/Iced: embedded mode uses the test dispatchers/Emulator determinism (virtual scheduler `advance_clock` / `Mode`); live-mode clock is best-effort (documented per-adapter cap).
- Semantics: `fast_forward(dt)` = advance virtual time in frame-sized steps until drained (timers + animations + subscriptions settle), `run_for(dt)` = real-time pace, `pause/resume`. All clocks are per-session; `clock.bin` in traces replays timing.
- **State seams (mocking):** Guiwright does not fake the network; it exposes a `Mock` domain where *apps* register reset/init hooks behind the `guiwright-driver` feature:

```rust
#[guiwright_kit::seam]                   // app code, compiled out without the feature
fn reset_db() -> Result<(), AppError> { app_state::reset_to_fixture("seed-42") }

// test side
app.mock().install("reset_db").await?;
app.mock().reset_state("seed-42").await?;   // between tests — fixture parity with Playwright contexts
```

  This mirrors iced's `Preset` (reproducible boot state) and masonry's `edit_root_widget` fixture pattern; each kit test gets a fresh app instance (process-per-test in CI, in-process re-launch in embedded mode).

---

## 15. Performance engineering

Guiwright must never become the bottleneck of a test suite nor disturb the app under test. Targets first, then mechanisms.

### 15.1 Budgets (design targets, enforced by `guiwright-bench` in CI)

| Operation (1280×800, ~1 000-node tree) | p50 target | p95 target | Notes |
|---|---|---|---|
| `Resolve` locator, cached index, embedded | 0.05 ms | 0.3 ms | hash/trie hit |
| `Resolve` locator, remote (UDS round-trip) | 0.6 ms | 3 ms | incl. marshalling |
| Apply semantic action → frame applied | 1 frame (≤16.7 ms) | 2 frames | rendezvous-bound, not CPU-bound |
| Tree diff apply (1 k nodes) | 0.10 ms | 0.4 ms | consumer `ChangeHandler` |
| Full tree snapshot serialize (postcard) | 0.7 ms | 2 ms | ~1 k nodes, ~200 B/node |
| `TreeChanged` diff blob (LZ4) | 0.05 ms | 0.3 ms | typ. < 2 % nodes changed |
| Screenshot readback (warm, wgpu) | 8 ms | 16 ms | async map, ring of 3 buffers |
| Element screenshot (crop) | 0.5 ms | 2 ms | crop, no re-render |
| Auto-wait wait per settled frame | ~0 CPU | — | event-driven, zero polling |
| Driver memory (steady state) | < 32 MB | — | + trace ring ≤ 256 MB cap |
| Trace write amplification | < 1 ms/frame amortized | — | writer-thread off critical path |

### 15.2 Tree service: one diff, many consumers

The single hottest path is "what changed since last frame". Design:

- The adapter's `TreeUpdate` (already incremental in masonry/freya/slint; full-but-cheap in egui) feeds **one** `accesskit_consumer::Tree`; `ChangeHandler` diffs fan out to: (a) locator indexes, (b) waiters, (c) trace recorder, (d) remote subscribers. One traversal, zero duplication.
- egui's full per-pass rebuild is absorbed here: consumer diffing makes "full rebuild" cost O(changes), so egui costs the same as incremental frameworks at the driver boundary.
- Snapshots are pooled (`Vec<NodeRecord>` reuse), versioned by `seq`; stale snapshots are dropped under backpressure (subscribers get `seq` gaps and re-sync via `TreeSnapshot`), never queued unboundedly.

### 15.3 Zero-polling auto-wait

Waiters are state machines woken by `TreeChanged`/`FramePresented`. There is no periodic scan: between frames the driver does **no CPU work** (verified by bench: wait phase allocs == 0). The only polling in the system is the L2 fallback at frame cadence (≤60 checks/s) and the client timeout watchdog (1 Hz timer). This is the biggest departure from naive harness loops (`sleep + retry`) and the main anti-flakiness win: checks run exactly when the world changes.

### 15.4 Frame-boundary discipline (correctness *is* performance)

- One rendezvous per action: command queued → applied at next `begin_pass`/update → response carries `frame_seq`. No "spin the loop N times" heuristics (kittest's `max_steps` idea is kept only as a hang detector with diagnostics).
- Pointer-move coalescing: adapters accept `InputBatch { events }`; frameworks that coalesce anyway (freya's `EventBuffer`, egui's per-pass events) receive batch-aligned input, avoiding frame amplification for drag gestures.
- Render-once invariant (egui): frame cache keyed by `ctx.cumulative_pass_nr()`; overlays/screenshots never trigger extra passes.

### 15.5 Capture pipeline

- Readback: `RENDER_ATTACHMENT|COPY_SRC` texture → `MAP_READ` buffer with 2–3 buffer ring; `device.poll(PollType::Wait)` only when a frame is actually requested; bytes are premultiplied RGBA8, converted once.
- Trace frames: hash-dedup (xxh3 over strided samples; full hash on mismatch), PNG-lossless on the writer thread, keyframe interval 1 s. Sliding-window ring keeps memory O(cap) for hour-long runs.
- Element screenshots: crop-by-rect from the window buffer (composite intersecting layers), single conversion pass.

### 15.6 IPC design

- UDS + postcard (2×-5× smaller than JSON, zero-copy-friendly); LZ4 frame compression only for blobs > 4 KiB; `MSG_MORE`-style batching of event flushes per frame.
- Backpressure everywhere: bounded channels (default 64 messages, 8 blob slots); slow clients degrade to "tree snapshots only" (frames dropped by hash), never block the UI thread — the UI thread's only costs are the tap hooks (a queue push each).
- Future (v0.3+): shared-memory frame ring for same-host clients (zero-copy pixels), mirroring CDP's binary socket philosophy.

### 15.7 Benchmark suite (`guiwright-bench`)

- Micro (criterion): codec round-trips, diff apply, index build on synthetic 10 k-node trees, LZ4 blobs, hash throughput.
- Macro: scripted 200-action session on the egui demo app and a synthetic masonry app (1 k nodes): end-to-end action latency histogram, suite wall-clock for 1 000 asserts, memory profile.
- Perf guards in CI: p95 budget regressions fail the build (budgets stored in `bench/budgets.toml`). §15.1's eleven rows are the design baseline; the canonical table has grown a row per milestone since (gesture, kit input, wire screenshots, dumps, events, log tail, ws/in-process ping, the wire action path, the live-feed sync, the checked-assert floor — 30 rows today), each bound to a sample in `check_budgets`.

---

## 16. Playwright parity map

| Playwright feature | Guiwright mechanism | Status |
|---|---|---|
| Locators (`getByRole/Label/Text/TestId/Value`, filters, strict mode) | §8 selector model over accesskit consumer + L2 logical trees | MVP |
| Auto-waiting / actionability | §8.3 event-driven checks; explicit `locator.wait_for(state)` on the kit surface (`attached`/`visible`/`hidden`/`stable`, M21) | MVP |
| Actions (click/dblclick/hover/fill/press/check/select/drag/scroll/focus) | §8.4–§9 (semantic + real-input paths); the wire `Action` honors `force`/`timeout_ms` (M17's pre-resolved gate); the kit locator surface carries `press`/`check`/`uncheck`/`set_checked`/`focus`/`blur` too (M19) | MVP |
| Assertions with retry (`expect(locator).to_*`) | `guiwright-core::expect` + waiter state machines; the checked tri-state rides the §8 check row (`to_be_checked` / `to_be_unchecked` / `to_be_checked_mixed`, M20) | MVP |
| Screenshots (page/element) + `toHaveScreenshot` | §10 (crop, per-OS baselines, dify-style diff); locator-level element shots (M9), the §7 render domain over GWP (M10) | MVP (embedded), M9 (locator), M10 (wire) |
| Test runner, fixtures, parallel/shard, retries, reports | `guiwright-kit` + nextest integration + JUnit/HTML; the §16 v0.5 own runner is `guiwright test` over `gwtest/v1` specs (M5) | MVP (nextest-native), M5 (own runner) |
| Trace viewer (time travel, live) | §12 | v0.3 (archive), M18 (live feed over the `ClientConn` seam) |
| Codegen + inspector overlay | §13 | M2/M4 (codegen), M8 (live overlay) |
| Video recording | §10 (ffmpeg pipe) | M3 (recorder + wire domain) |
| Virtual clock (`clock.install/fastForward`) | §14 | v0.3 (egui/slint/masonry/freya), v0.5 (gpui/iced live) |
| Network mocking analog (state seams) | §14 `Mock` domain | M2 (`#[seam]` + registry + wire) |
| Multi-window / tabs | targets per window (all frameworks support multi-window; freya-testing single-window initially) | M6 |
| Headless CI | §11 | MVP |
| Error context (call log, DOM snapshot on failure) | error embeds tree snapshot + trace link | MVP |
| REPL / debugging | `guiwright repl` over GWP | M4 (M6 window commands, M7 input, M8 inspect, M10 screenshot, M11 dump, M12 events on/off, M13 snapshot under, M14 log kind) |
| Live event stream (server-push) | §7 event channel: `Subscribe { On | Off }`, events ride `ResponseV1::Event`; the `Log` kind rides the per-session `tracing` tap | M12 (logs M14) |
| Component testing | embedded mode = widget-level (Simulator/RenderRoot/harness) | MVP |
| Other-language clients | GWP over TCP/UDS/WS + the in-process embedded door; schema-published | M3 (TypeScript client + e2e); M15 (WS served in Rust); M16 (embedded door + public `Transport` seam) |

---

## 17. Maturity ranking & MVP roadmap

Ranked by (a) harness completeness today, (b) a11y substrate, (c) integration risk:

1. **egui** — full a11y, existing harness + existing driver protocol; adapter ≈ glue.
2. **Masonry/Xilem** — `WidgetId↔NodeId` identity, headless deterministic harness, wgpu-free screenshots; gaps: kittest TODO (we fill), EnableAccessTree laziness (known).
3. **Iced** — `Emulator`+`iced_selector`+headless screenshots are 80 % of a Level-2 adapter; gap: no a11y (logical tree), role registry contribution.
4. **Freya** — `TestingRunner` + software Skia + a11y ids; gap: key-event fidelity, multi-window headless.
5. **Slint** — testing backend is complete but semver-exempt (per-release maintenance) and needs debug-info builds; a11y door exists for live apps.
6. **Floem** — great headless event harness; needs test-id + role metadata + headless pixels (two scoped contributions).
7. **GPUI** — powerful but: platform-crate churn, main-thread-only, test-support-gated capture; highest ceiling (Zed-scale apps), scheduled after the core stabilizes on the six above.

> Note: the original plan proposed GPUI + Floem + egui as the prototype trio. Recon supports **egui + Masonry + Iced** as the fastest MVP that still exercises all three levels (L1 native, L1 custom-harness, L2 logical-tree), with Freya/Slint in the first expansion wave and Floem/GPUI following their upstream contributions.

- **M0 (weeks 0–6):** `guiwright-core` (tree service, selectors, actionability, expect), egui adapter (embedded + GWP over UDS), masonry adapter (embedded), CI matrix, budgets bench.
- **M1 (weeks 6–12):** iced + freya adapters; trace recorder + zip; screenshot baselines + bless flow; `#[guiwright::test]` kit; nextest integration.
- **M2 (weeks 12–20):** trace viewer; codegen + overlay (egui, masonry); slint adapter (testing backend + build helper); clock service; remote-mode GPUI spike.
- **M3:** floem contributions (test-id, tiny-skia headless) + adapter; GPUI adapter (test-support + a11y hybrid); video; other-language client.

---

## 18. Risks & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Framework churn (freya rc, gpui platform split, iced 0.15-dev, slint semver-exempt internals, floem winit fork) | adapters break per release | adapters are thin & feature-gated; CI runs against each framework's release **and** main; `AdapterMeta` reports versions in traces |
| No stable ids in some stacks (floem, iced optional ids, egui hash chains) | flaky locators | `TestId` first-class + codegen guidance; generation counters against id recycling; strict-mode errors show the tree |
| AccessKit laziness/activation semantics differ (masonry `EnableAccessTree`, freya screen-reader gate, GPUI active flag) | missing trees | adapters force-enable at driver attach (`update_if_active`-compatible; ActivationHandler with immediate activation for tests) |
| Main-thread confinement (all but egui-ish paths) | driver latency if UI thread busy | frame-boundary rendezvous + wakers; never block UI thread; bounded queues; document per-adapter `pump` guarantees |
| Pixel determinism (fonts, GPU, OS) | visual-regression noise | per-OS thresholds + `max_failed_pixels`; CPU adapters in CI; deterministic font loading; hash-dedup |
| Rewrite-pass saturation (masonry), frame-quantized input (freya), coalescing differences | gesture fidelity | per-adapter gesture pacing tables (§9), validated by cross-framework gesture tests |
| License coupling (slint GPL/royalty-free) | distribution questions | driver/test-binary-only linking; no slint code in `guiwright-core`; docs spell out the boundary |
| Scope creep toward "another e2e language" | diluted core | Rust-first kit; GWP is stable and published, but the runner/kit stays Rust until v1 |

---

## 19. Appendix — evidence index (key files opened during recon)

- **egui:** `crates/egui_kittest/src/{lib.rs,node.rs,wgpu.rs,snapshot.rs,config.rs,recording.rs}`; `crates/egui/src/{context.rs,id.rs,widget_rect.rs,hit_test.rs,data/input/{raw_input.rs,event.rs}}`; `crates/egui_inspection/src/protocol.rs`; `crates/egui/src/plugin.rs`; root `kittest.toml`.
- **Xilem/Masonry:** `masonry_testing/src/{harness.rs,recorder_widget.rs,screenshots.rs}`; `masonry_core/src/{render_root.rs,passes/{accessibility.rs,event.rs,compose.rs,paint.rs},core/{widget.rs,widget_tag.rs,events.rs,contexts.rs}}`; `masonry_winit/src/{event_loop_runner.rs,convert_winit_event.rs,vello_util.rs}`; `masonry_imaging/{Cargo.toml,src/lib.rs,src/headless_wgpu.rs}`; `xilem/examples/external_event_loop.rs`.
- **Freya:** `crates/freya-testing/src/lib.rs`; `crates/freya-core/src/{tree.rs,runner.rs,accessibility/{tree.rs,id.rs},events/{platform.rs,measurer.rs,executor.rs},render_pipeline.rs,node_id.rs,elements/extensions.rs}`; `crates/freya-winit/src/{renderer.rs,window.rs,config.rs,drivers/{mod.rs,software.rs},extensions.rs,tests/smoke.rs}`; `crates/torin/src/{torin.rs,tree_adapter.rs}`; `crates/ragnarok/src/{event_buffer.rs,measurer.rs,measurement.rs}`.
- **Iced:** `test/src/{simulator.rs,emulator.rs,ice.rs,instruction.rs}`; `selector/src/{lib.rs,find.rs,target.rs}`; `runtime/src/{lib.rs,window.rs,task.rs}`; `winit/src/{lib.rs,proxy.rs,conversion.rs}`; `core/src/{layout.rs,renderer.rs,widget/{tree.rs,operation.rs,id.rs},window/screenshot.rs}`; `wgpu/src/lib.rs`; `tiny_skia/src/lib.rs`; `devtools/src/{time_machine.rs,comet.rs}`; `beacon/src/lib.rs`.
- **Slint:** `internal/backends/testing/{lib.rs,testing_backend.rs,search_api.rs,introspection/mod.rs,systest.rs,mcp_server.rs,README.md}`; `internal/backends/winit/{accesskit.rs,lib.rs,event_loop.rs,Cargo.toml}`; `internal/renderers/software/{api.rs,lib.rs,src/minimal_software_window.rs}`; `internal/core/{api.rs,item_tree.rs,renderer.rs}`; `tests/driver/driverlib/lib.rs`; `tests/driver/rust/build.rs`; `api/rs/slint/Cargo.toml`.
- **GPUI:** `crates/gpui/src/{platform.rs,app.rs,window.rs,window/a11y.rs,element.rs,taffy.rs,interactive.rs,executor.rs,input.rs}`; `crates/gpui/src/platform/test/{platform.rs,window.rs,dispatcher.rs}`; `crates/gpui/src/app/{test_context.rs,headless_app_context.rs,visual_test_context.rs}`; `crates/gpui/Cargo.toml`; root `Cargo.toml`/`Cargo.lock` (backend crate composition).
- **AccessKit/kittest:** `accesskit/src/lib.rs`, `accesskit_consumer/src/{tree.rs,node.rs,filters.rs,iterators.rs}`; `adapters/winit/src/lib.rs`; `kittest/src/{state.rs,node.rs,filter.rs,query.rs}`; `kittest/integration_example/src/bin/basic_integration.rs`.
