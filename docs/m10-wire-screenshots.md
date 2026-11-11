# M10 — Wire screenshots: the §7 render domain

**Status:** implemented (this milestone lands the GWP render domain:
`Screenshot { Window | Element { selector, timeout_ms } }` → `Shot` —
the §10 capture path rendered driver-side, PNG over the wire, with the
REPL command, the TypeScript client, byte-pinned goldens, a budget row,
example and docs).

---

## 1. Why the wire, and why now

`docs/m9-locator-element-screenshots.md` §4 recorded the gap verbatim:
*"No wire transport for screenshots. `Screenshot { Window | Element }`
over GWP needs the blob side channel for PNG payloads (§7/§15.6) plus
per-connection target semantics; the embedded surface ships first because
it is where baselines live (repos, not remote hosts). Recorded as the
next screenshot milestone."* This is that milestone.

The consumer case is CI-shaped: a container runs the app in remote mode,
and the harness (Rust or the TypeScript client) pulls visual evidence off
the wire — window states between actions, element crops for artifact
uploads. The §7 sketch named the domain from the start
(`Screenshot { target: Window|Element(handle), opts } → Shot { blob_ref,
meta }`); the driver-side ingredients all existed: the mock adapter's
`with_frame_pixels` capture contract (M1), `guiwright-shot`'s crop +
PNG codec (M1/M9), and the resolve-with-auto-wait the `Resolve` arm
already runs (§8.2 over the wire).

| Hook | What it names | Status before M10 |
|---|---|---|
| §7 render domain | `Screenshot { Window \| Element } → Shot` | sketched, never wired |
| §10 capture | "prefer `render_frame` for headless on-demand" | embedded kit only |
| §6.2 caps | `render_frame` / `last_frame` capability rows | advertised, unconsumed over GWP |
| m9 §4 | "recorded as the next screenshot milestone" | this document |

## 2. Design

### 2.1 One render pass, cropped data-side (§15.4)

`Session::screenshot_window` renders **once** (`render_frame(None)` — the
§10 headless on-demand path the kit's `App::screenshot` already uses) and
PNG-encodes the frame. `Session::screenshot_element(key)` takes the
resolved identity, reads the node's physical bounds, clamps them into the
frame (the kit's guarded `clamp` + `u32::try_from` conversion, verbatim)
and crops the *same single frame* — the "never re-render" rule is a
data-side invariant, exactly like the kit's M9 path. The GWP server
resolves `Element { selector, timeout_ms }` through the same
`resolve_result` the `Resolve` arm uses (auto-wait `Attached` within the
timeout, honoring the connection's window scope, §16 v0.4), so element
shots behave exactly like the locators they aim at.

### 2.2 The PNG rides the frame — not the LZ4 blob stage

m9 §4 expected "the blob side channel for PNG payloads". Decision:
**`Shot.png` carries the PNG bytes directly** in the response frame.
PNG is already losslessly compressed (DEFLATE); running the §15.6 LZ4
stage over it double-encodes without shrinking it, and an LZ4 decoder
would make the TypeScript client dependency-full for zero gain. The
§15.6 intent — command latency independent of payload size — is met by
the length-prefixed frame itself, with `MAX_FRAME_BYTES` (64 MiB)
guarding hostile sizes. The blob stage stays with the traffic it was
sized for (trace frames, where static UI repeats). This is recorded in
the schema docs and the `Shot` variant docs; the transport-level side
channel (§15.6 "future: shared-memory ring") remains future work for
both payload kinds.

### 2.3 Capability rows are the contract (§6.2)

Adapters without a pixel path answer `ErrorCode::Unsupported` — the same
structure-preserving error discipline M8 pinned for the overlay
(`AdapterError::Unsupported` reaches the wire typed, never a string
match). A node without bounds is an honest `CoreError::Other` carrying
the M9 wording ("nothing to crop (§10)"), and a dead selector expires as
`ErrorCode::Timeout` like every other auto-wait.

## 3. Tests + bench

- `guiwright-driver/tests/wire_screenshot.rs` — window shot byte-checked
  against the scripted gradient; element crop equal to the frame region;
  element auto-wait on a late node (the shared-script pattern);
  `Unsupported` without a render surface; honest no-bounds error;
  timeout expiry.
- `guiwright-cli` REPL — `screenshot [path]` writes the artifact
  client-side (Playwright parity: the client owns the path); the e2e
  asserts the summary line shape, decodes the PNG, and the honest
  `Unsupported` on a bare adapter. Element shots stay programmatic (the
  REPL holds no selector to re-resolve honestly).
- TypeScript — `screenshotWindow()` / `screenshotElement(selector,
  timeoutMs?)`, golden-pinned request/response vectors, vendored-schema
  re-pin; the decoder maps `Shot.png` to a `Uint8Array`.
- `guiwright-bench` — `wire_screenshot_window` sample (pump + one render
  + PNG encode of the 480×240 fixture; measured p50 ≈ 3.4 ms on the dev
  host) + the `wire_screenshot_window` row in both `budgets.toml` copies.
- Goldens: `screenshot_window` / `screenshot_element` requests
  (discriminant 13), the `shot` reply (discriminant 15); the published
  schema gains the `screenshot` domain table; bless flow re-run.

## 4. Remaining gaps

- Baselines over the wire: `Shot` returns PNG; the *assertion* flow
  (per-OS baseline stores, bless artifacts) stays embedded-side by design
  — baselines live in the repo, not the container. A remote-bless
  protocol would need artifact upload, not just a fetch.
- Multi-layer popups (§10 composites intersecting layers) crop the
  window frame only — the same caveat m9 recorded; layer-aware
  composition lands with the capture-pipeline pass.
- Video-style frame streaming for live vieweres rides the event
  channel's future milestones; `Shot` is request/response on purpose.

[`Session::screenshot_element`]: ../../crates/guiwright-driver/src/session.rs
[`Shot`]: ../../crates/guiwright-proto/src/schema.rs
