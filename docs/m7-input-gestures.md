# M7 — Raw input & gestures: the §9 pipeline over the wire

**Status:** implemented (this milestone un-blocks the dead `Input` wire
domain, appends the `Gesture` domain to the V1 envelope, adds the
per-window injection surface to the adapter trait, and gives the REPL,
the runner and the TypeScript client the Playwright-shaped input
vocabulary).
**Scope:** the §9 input pipeline's wire half — the row M6 recorded as
deferred with the note "raw input over GWP … the gesture milestone will
route coordinates through the adapter's per-window surface, §9", and the
`Unsupported` hole the server answered every `Input` request with since
M0 ("raw input over GWP lands with the gesture synthesis milestone").
The substrate was already on the wire: `RequestV1::Input` /
`ResponseV1::InputQueued` and `InputEventV1` have existed since the M0
schema; the semantic `Scroll`/`Hover` actions have ridden the adapter's
`input()` hook internally since M0.

---

## 1. Why gestures are the milestone after multi-window

M6 closed the last version-tagged parity row below v1. Its deferred list
names what is left, and one item is load-bearing for the Playwright
experience: **raw input**. Semantic actions (§8.4) click a11y nodes;
Playwright also owns *coordinates* — `page.mouse.click(x, y)`,
`mouse.drag`, `mouse.wheel`, `keyboard.press` — the surface that drives
canvas apps, custom widgets and anything the a11y tree under-reports.
The architecture reserved the vocabulary for it since the first schema
(`InputEventV1`, §9) and the fidelity rule since the first trait
("synthetic events enter at the same choke point as OS events", §9).
M6 made the last piece available: per-connection window targets —
"the gesture milestone will take the target window from the
connection's active handle".

| Architecture hook | What it names | Status before M7 |
|---|---|---|
| §9 event vocabulary | `InputEventV1` on the wire | defined, but the server rejected every `Input` |
| §9 gesture synthesis | "click = move→down→(settle)→up; drag = down→interpolated moves (ratchet ~5 px/tick)→up" | not implemented anywhere |
| §9 coordinates | "logical px relative to the window; adapters own the scale conversion" | no window dimension on `input()` |
| §18 pacing tables | "per-adapter gesture pacing tables, validated by cross-framework gesture tests" | no table, no hook |
| M6 deferred | "per-window input routing (§9)" | this milestone |

## 2. Design

### 2.1 Synthesis lives in the driver (`guiwright-driver::gesture`)

The §9 recipes run **server-side**, once, in front of the adapter — not
duplicated in every client:

- **click** = `PointerMove(x, y)` → `PointerDown` → `PointerUp`, repeated
  `count` times at the same point (`count` follows Playwright's
  clickCount: 1 = click, 2 = double-click). A zero-count click
  synthesizes nothing and is rejected before any injection (a client
  bug, not a request).
- **drag** = `PointerDown(from)` → ratchet-interpolated `PointerMove`
  steps → `PointerUp(to)`. The step count is
  `ceil(max(|dx|, |dy|) / ratchet)`; the last move lands **exactly** on
  the target so the release coordinates are honest. A non-positive
  ratchet collapses to one move (no division hazards).
- **key** = `KeyDown{key, modifiers}` → `KeyUp`; **type** = one
  `TextInput` commit — the IME-vs-keys translation stays the adapter's
  §4.2 job, where it belongs.
- **wheel** = one `Wheel{dx, dy}`.

The press→release "settle" (§9) rides the adapter's canonical-order
frame flush — the sequence order here is the contract; frameworks that
quantize input per frame (freya's `EventBuffer`) already apply it in
order at the frame boundary (the freya harness test pins that).

Pacing comes from the adapter: `FrameworkAdapter::gesture_pacing()`
returns a `GesturePacing { drag_ratchet_px }` table, default 5.0 (§9
"ratchet ~5 px/tick, mirroring slint's flickable-aware drag pacing").
This is the §18 hook — per-adapter tables, validated by tests here and
by framework harnesses downstream.

### 2.2 Per-window routing (the M6 handoff)

The adapter surface gains **`input_window(window, ev)`** next to
`input(ev)`:

- the default ignores the handle and delegates to `input(ev)` — a
  single-window adapter has exactly one coordinate space, so **every
  existing adapter compiles unchanged** and stays honest (the M6
  `windows()` pattern: freya-testing's single-window cap is expressed,
  not worked around);
- multi-window adapters override it; the session validates the handle
  against the tree **first** (`window_scope` → `UnknownWindow` for dead
  targets), so a live handle always reaches the adapter.

The session resolves the target once per request
(`Session::input_target`): an explicit handle is validated; `None`
selects the **session default** — the first target the adapter reports
(a single-window app's `main`, a multi-window app's primary window).
`Session::send_input(events, window)` injects a raw batch;
`Session::perform_gesture(gesture, window)` synthesizes first (adapter
pacing) and rides the same injection. Both apply the batch inside **one
rendezvous pump** (§15.4) and report the frame — the `InputQueued` /
`GestureDone` frame_seq, no polling.

Scoping stays resolution-only (§16 v0.4, M6): the input target is
**per-request routing**, applied and consumed in the same rendezvous
step — it never narrows anyone's resolution. An unscoped connection
still resolves the whole tree; its coordinates land in the session's
default window.

### 2.3 The wire — `Gesture` appends, `Input` wakes up (append-only V1 revision #3)

```text
requests:  ... Video(10), Window(11), Gesture(12)
  GestureV1:  Click { x, y, button, count }(0) | Drag { from_x, from_y, to_x, to_y, button }(1)
            | Wheel { dx, dy }(2) | Key { key, modifiers }(3) | Type { text }(4)
responses: ... Windows(12), WindowUsed(13), GestureDone { frame_seq, detail }(14)
```

`Input { events }` keeps its discriminant (5) and payload byte-for-byte —
it just stopped being rejected: the bridge's per-connection window stamp
(M6) now also steers it, and the reply is the long-defined
`InputQueued { frame_seq }`. `GestureDone` mirrors `ActionDone`
(frame + detail); the detail line is the synthesis summary
("clicked at (12.5, 30) — 3 events"), pinned by the golden vectors. The
discriminant pins, the schemadoc domain table and the schema golden all
moved together (third application of the M3 discipline); the vendored
TypeScript schema copy and the wire vectors (`gesture_click`,
`gesture_drag`, `gesture_key`, `gesture_done`) ride the same bless.

### 2.4 The client half — one vocabulary everywhere

- **REPL**: `mouse click <x> <y> [right|middle]`, `mouse dblclick`,
  `mouse drag <x1> <y1> <x2> <y2>`, `mouse wheel <dx> <dy>` (gestures)
  and `mouse move | down | up` (raw primitives, the `Input` request) —
  plus `key <key> [modifiers…]` and `text <string>`. Every command rides
  the connection's active window target; replies print the synthesis
  detail and the applied frame (`clicked at (10, 20) — 3 events @ frame
  7`). The semantic vocabulary is untouched: `click`/`press`/`scroll`
  keep acting on the current node.
- **Runner**: `gwtest/v1` steps are REPL commands (the M5 design), so
  the gesture vocabulary is declarative today — a spec drives
  `mouse drag` + `key Enter` with no spec-format change.
- **TypeScript client**: Playwright-shaped `client.mouse.click(x, y,
  { button, clickCount })` / `dblclick` / `drag(from, to)` / `wheel(dx,
  dy)` and `client.keyboard.press(key, modifiers)` / `type(text)`, plus
  the low-level `client.input(events)` and `client.gesture(cmd)` for
  the full vocabulary. The codec paths are pinned byte-for-byte by the
  new golden vectors; the cross-language e2e drives the Rust demo app
  over TCP with the whole surface.

## 3. Testing posture

The M2/M4/M5/M6 pattern repeats — the framework-free mock adapter + a
real socket is the contract test, TDD throughout (proto pins and the
driver synthesis tests landed first).

- `crates/guiwright-driver/src/gesture.rs` — the synthesis math: exact
  click/double-click sequences, ratchet interpolation (endpoints honest,
  step counts per pacing, zero-length and zero-ratchet drags), keyboard
  and wheel shapes, count-0 emptiness, lossless wire conversions for
  every `InputEventV1` shape.
- `crates/guiwright-driver/tests/input_gestures.rs` — session level
  (synthesis outcomes and details, adapter pacing override, raw batch
  routing, empty-batch no-op, `UnknownWindow` validation with nothing
  injected, zero-count rejection, default-target resolution on a
  multi-window tree) and the §11 story over a real UDS socket:
  `Input` → `InputQueued` (the pre-M7 hole closed), `Gesture` →
  `GestureDone`, **two connections injecting into their own windows**
  (the M6 handoff, observed through the shared input log),
  a window closed mid-session failing `Gesture` with a clean
  `UnknownWindow`.
- `crates/guiwright-cli/tests/repl_e2e.rs` — the whole REPL vocabulary
  over a socket: every gesture and primitive command's reply line, named
  buttons, usage errors as single error lines, the session staying
  usable, the help text naming the domains.
- `crates/guiwright-cli/tests/runner_e2e.rs` — a `gwtest/v1` spec whose
  steps are pure gesture vocabulary, green end-to-end.
- `crates/guiwright-proto` — `gesture_domain_roundtrip`, the extended
  append-only pins (Gesture = 12, GestureDone = 14, the five
  `GestureV1` variants), the schemadoc domain table + counts, and the
  re-blessed golden; `golden_vectors.rs` gains the vectors the TS codec
  pins.
- `clients/typescript` — the vendored schema re-pinned (frontier moved
  to `Gesture`/`GestureDone`, the gesture domain table), the three new
  request vectors encoded byte-identically, `gesture_done` decoded, and
  the cross-language e2e driving the demo app's mouse/keyboard surface.
- `crates/guiwright-bench` — the §9 synthesis hot path bound
  (`gesture_synthesis`: one 200 px drag + one double-click per
  measurement; p95 budget 50 µs, measured ≈ 1 µs), enforced by the
  release guard like the M1–M3 micro rows.

## 4. Deferred (recorded)

- Per-window traces/videos: the trace/video services record the
  session; window-tagged streams ride the same handle vocabulary in a
  follow-up.
- Kit-embedded input: the embedded kit drives a `Session` directly, and
  `Session::perform_gesture` is already its door — a kit-side
  `app.mouse` locator surface rides a follow-up when an embedded
  harness needs it (the driver, wire, REPL, runner and TS client carry
  the row).
- The `guiwright codegen <app-cmd>` launcher wrapper (§13 overlay
  session), live trace-viewer streaming (§12 "live mode"), HTML
  per-scenario trace deep links — unchanged from M5/M6.
- Cross-framework gesture harness tests (§18 "validated by
  cross-framework gesture tests"): the pacing hook is in place; real
  framework adapters (egui/masonry/iced/freya) exercise it through
  their own harnesses downstream — the in-repo adapters are
  framework-free engines and carry no input pipeline yet.

## 5. Conclusion

M7 turns the §9 pipeline from a wire stub into the real input path:
coordinates are window-relative logical px steered by the connection's
active target, gestures are synthesized once in the driver under the
adapter's pacing table, and every client speaks the same
Playwright-shaped vocabulary over the same append-only wire. No protocol
break (the M0 `Input` domain simply woke up), no adapter rewrite (the
default `input_window` delegates, byte-compatible with every pre-M7
client), one session — many connections, each driving its own window
with raw input.
