# M9 — Element screenshots: the §10 locator crop through the kit

**Status:** implemented (this milestone lands the locator-level element
screenshot: `locator.screenshot(name, bless)` — resolve → render once →
crop by the resolved node's physical bounds → the §10 baseline flow —
plus its budget row, example and docs).
**Scope:** the §7/§10 sketch named element screenshots from the start —
`Screenshot { target: Window | Element(handle) }` on the wire, "crop from
the window frame by the node's clip-aware rect; never re-render" in §10.
The kit half existed for windows only: `App::screenshot(name, bless)`
asserts the whole frame against `tests/screenshots/<name>/`. The element
half was dormant-but-prepared: `guiwright-shot::crop` shipped in M1 and
is budget-bound (`element_screenshot_crop`, "bound at M1 (CPU crop)"),
and the capture pipeline (render-once frames) has been in since the video
milestone. This milestone wires the two together on the locator surface —
the consumer shape Playwright's `locator.screenshot()` made standard.

---

## 1. Why the locator, and why now

Visual regression for *components* is the day-to-day flow: a button's
baseline survives unrelated page noise (clock text, avatars, ads) that a
window-level baseline never does. Every ingredient was in place; the only
missing piece was the resolve→crop→assert seam on the locator. M8 kept
the wire untouched by design — the embedded kit is the screenshot home
(§10 has no wire transport yet: pixel payloads ride neither the command
channel nor the LZ4 blob side channel today, and a `Screenshot` domain
with blob refs is its own future milestone, recorded in §4).

| Architecture hook | What it names | Status before M9 |
|---|---|---|
| §10 "Element screenshots" | "crop from the window frame by the node's clip-aware rect (`visible_bounds`); never re-render" | `crop` existed, nothing called it |
| §15.1 `element_screenshot_crop` | the crop hot path, budget-bound | bound at M1, unconsumed by any user-facing surface |
| §10 baselines | per-platform stores, bless flow, artifacts | window-level only |
| Playwright parity map | `locator.screenshot()` | window-level `toHaveScreenshot` analog only |

## 2. Design

### 2.1 Resolve once, render once, crop (§15.4 discipline)

`Locator::screenshot(name, bless)` consumes the locator (the same
single-shot semantics as `click`): a strict-mode
[`Session::resolve`] (auto-waited, pump-consistent — the same call every
locator action makes), **one** `render_frame(None)`, then
`guiwright-shot::crop` by the resolved node's AccessKit bounds — already
physical px (§9 convention), rounded and clamped into the frame. No
per-element re-render, no second pass: the render-once invariant (§15.4)
holds because the crop is data-side.

The bounds-less case (nodes without geometry — the fixture's hidden
button, or a window root without bounds) is a clean `CoreError` ("has no
bounds — nothing to crop, §10"); an adapter without a pixel path surfaces
its honest `render_frame` `Unsupported` (the §6.2 row — never a silent
empty image).

### 2.2 The baseline flow is the window flow

The cropped element goes through the same
[`BaselineStore::assert_snapshot`] as `App::screenshot`: per-platform
directories (`<name>/<os>-<arch>-<renderer>.png`), the app's
`Guiwright.toml` tolerances (per-OS overrides included), `BlessMode` from
the caller, and the same artifact contract on mismatch — `<stem>.new.png`
+ `<stem>.diff.png` next to the baseline. The renderer token stays the
adapter's framework name, so element and window baselines of one adapter
never fight each other.

### 2.3 The §10 metric caveat, made testable

The perceptual metric is *forgiving by design* (weighted squared YIQ,
kittest's checked-in 0.6 threshold): mid-luma repaints — even solid
recolors — can pass, and that is the anti-flake feature, not a bug. The
milestone tests pin both sides of that behavior: byte-exact crops on the
happy path (the gradient fixture makes every pixel checkable), and a
deterministic beyond-threshold tamper (pure-white pixels against the
gradient) for the mismatch path with its artifacts.

## 3. Tests + bench

- `guiwright-kit/tests/locator_screenshot.rs` — crop correctness
  (byte-checked against the scripted gradient), fresh-baseline/Matched
  round-trip, mismatch artifacts, the no-pixels and bounds-less honest
  errors, and a smoke-shaped budget bound.
- `guiwright-bench` — `locator_element_screenshot` sample (kit
  end-to-end: resolve + render + crop + baseline PNG encode/IO) +
  budget rows in both `budgets.toml` copies; measured p50 ≈ 100 µs on
  the fixture, enforced by `check_budgets` in CI.
- Example: `cargo run -p guiwright-kit --example locator_screenshot`.

## 4. Remaining gaps

- **No wire transport for screenshots.** `Screenshot { Window | Element }`
  over GWP needs the blob side channel for PNG payloads (§7/§15.6) plus
  per-connection target semantics; the embedded surface ships first
  because it is where baselines live (repos, not remote hosts). Recorded
  as the next screenshot milestone.
- Multi-layer popups (§10 "composites the layers intersecting the rect")
  crop the window frame only — layer-aware composition lands with the
  capture-pipeline pass.
- `expect(locator).to_have_screenshot(name)` — the assertion-shaped
  wrapper over this seam (retry semantics do not apply to deterministic
  frames, so the locator method is the honest surface for now).

[`Session::resolve`]: ../../crates/guiwright-driver/src/session.rs
[`BaselineStore::assert_snapshot`]: ../../crates/guiwright-shot/src/baseline.rs
