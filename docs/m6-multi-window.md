# M6 — Multi-window / tabs: the `Window` domain (§16 v0.4, §7, §11)

**Status:** implemented (this milestone extends `guiwright-core` with
window-scoped resolution, `guiwright-driver` with the adapter window
surface + per-connection targeting, and the wire with the append-only
`Window` domain; the REPL and the runner gain the `window` vocabulary).
**Scope:** the §16 parity-map row "Multi-window / tabs — targets per
window (all frameworks support multi-window; freya-testing
single-window initially) | v0.4" — the row both M4 and M5 deferred with
the note "needs the `V1` append-only protocol revision and its own spike
doc"; the §7 session → targets story ("sessions -> targets (windows) ->
domains" — the schemadoc description has named this shape since M3); and
the §11 sentence "one driver session serves all connections", now with a
per-connection lens.

---

## 1. Why multi-window is the next milestone

After M5 every version-tagged row below v1 of the §16 parity map is
closed except one: **multi-window / tabs**. Every real GUI app the
Playwright experience targets has windows (Zed panels, editor +
preview splits, dialogs); a driver that can only see "the tree" cannot
express "the tree of window 2". The M4/M5 deferral records the two
preconditions: an **append-only protocol revision** (the M3 `Video`
append established the discipline) and a **spike doc** (this file,
`docs/m2-gpui-remote-spike.md` precedent). The enabling fact was already
in the codebase: `NodeKey::AccessKit { tree, node }` — every node
identity has carried a per-tree component since M0, and AccessKit models
multi-window apps as **grafted subtrees** (window nodes in the root tree
carry a `tree_id` property naming the subtree that holds the window's
content; `accesskit_consumer` 0.39 grafts them into one walkable tree).

| Architecture hook | What it names | Status before M6 |
|---|---|---|
| §16 parity map | "Multi-window / tabs — targets per window" | open (v0.4, the last row below v1) |
| §7 schemadoc | "sessions -> targets (windows) -> domains" | no target surface on the wire |
| §11 | "one driver session serves all connections" | all connections shared one undifferentiated view |
| M4/M5 deferred | "needs the `V1` append-only protocol revision and its own spike doc" | this milestone |

The other deferred rows keep their recorded state: the
`guiwright codegen <app-cmd>` launcher wrapper (lands with the §13
overlay session), live trace-viewer streaming (§12 "live mode") and the
HTML per-scenario trace deep links are unchanged.

## 2. Design: targets per window, per connection

### 2.1 The tree half — grafted subtrees are windows (`guiwright-core`)

`TreeService::windows()` enumerates the window targets by walking the
root tree and collecting graft nodes (`NodeRef::is_graft()`); each
window's **handle is its subtree `TreeId` string** — the same identity
`NodeKey::AccessKit` already carries, so keys never collide across
windows and no new id space is invented. A graft-free (single-window)
tree reports exactly one target: handle `"main"`
(`guiwright_core::MAIN_WINDOW`), rooted at the tree root.

- `TreeService::window_scope(handle)` maps a handle to its scope root —
  `Ok(None)` for `main` (the whole tree needs no filter),
  `Err(UnknownWindow)` for anything the tree does not know. The **tree
  is the source of truth**: an adapter metadata entry without tree
  content is not a usable target (a closed window's handle stops
  resolving immediately).
- Scoping is a **TreeService-level filter** (`set_scope`), applied in
  `resolve_all` — one injection point that every resolution path
  honors for free: strict `resolve`, `resolve_all`, the waiters
  (`ChecksWaiter` re-resolves through the same service) and `expect`.
  The scope root itself matches (a locator may target the window node).
- `resolve_scoped(selector, scope)` is the explicit one-call form the
  tests use; the session form (`set_resolve_scope` / `clear_resolve_scope`)
  is what the server wraps around a scoped request.

Generation counters stay per-`NodeKey`: closing a window removes its
subtree, which bumps only that window's identities (§8.2 ABA guard
unchanged — and window reuse cannot alias across the TreeId boundary).

### 2.2 The adapter half — one default, one contract (`guiwright-driver`)

`FrameworkAdapter::windows()` defaults to one `main` target, so **every
existing adapter compiles unchanged** and honestly reports what it has
(freya-testing's single-window cap is thereby expressed, not worked
around — the §16 note). A multi-window adapter returns one `WindowInfo`
per window with the contract pinned in the trait docs: **handles MUST
equal the AccessKit subtree `TreeId` strings the adapter pushes** — the
adapter metadata (title/focus) and the tree scopes are two views of the
same windows, and `Window::Use` validates against the tree. The mock
adapter's multi-window persona (`with_multi_window`) scripts both from
one `MultiWindowFixture`, the way a real adapter mints both from its
window bookkeeping.

### 2.3 The wire half — the `Window` domain (append-only V1 revision)

Variants append at the end of the enums; the discriminant pinning test
and the schemadoc golden both enforce it (the M3 discipline, second
application — "old clients get clean errors"):

```text
requests:  ... Bye(9), Video(10), Window(11)
  WindowV1:  List(0) | Use { handle }(1)
responses: ... VideoFile(11), Windows { windows: seq<WindowInfoV1> }(12),
           WindowUsed { handle }(13)
ErrorCode: ... Internal(7), UnknownWindow(8)
```

`Welcome` and every pre-existing payload are byte-identical; the
published `gwp-schema.json` (golden + the vendored TypeScript copy) and
the TS golden wire vectors moved together with the enums.

**`Use` aims the calling connection, not the session.** Playwright's
model is per-page handles; CDP's is per-target sessions. Guiwright keeps
§11's "one driver session serves all connections" and adds the lens:
each connection gets its own `GwpBridge` instance (the serve loop calls
`make_handler` per socket), the bridge holds that connection's active
handle, and it **stamps every request** with it. `Window::Use` is
forwarded to the session (which validates the handle against the tree)
and only on the `WindowUsed` reply does the bridge record the handle —
a failed `Use` applies nothing. The session stays a single honest
interpreter: it reads the per-request scope, applies it, clears it in
the same rendezvous step. No cross-connection interference is possible
by construction, and the wire surface for scoping is zero beyond the
domain itself.

**Scoping concerns resolution only.** `Action` node handles are already
window-unique (the TreeId is in the key) and session-wide services
(clock, mock, inspect, video) have no window semantics; `TreeSnapshot`
reports the whole session deliberately (a debugging view). Raw `Input`
over GWP remains recorded as deferred (the gesture milestone will route
coordinates through the adapter's per-window surface, §9).

### 2.4 The client half — one vocabulary everywhere

- **REPL**: `window list` (marks the active target with `*`, prints
  title + focus) and `window use <handle>` (confirmation names the
  title). The debugging loop is unchanged otherwise: after `use`, the
  existing `get`/`expect`/action commands see only that window — no new
  locator syntax.
- **Runner**: `gwtest/v1` steps are REPL commands (the M5 design), so
  `window use` is declarative today — a spec re-aims mid-scenario and
  the scoped steps follow. No spec-format change.
- **TypeScript client**: `windowList()` / `windowUse(handle)` plus the
  `WindowInfoV1` type, pinned byte-for-byte by the new golden vectors
  (`window_list`, `window_use`, `windows`, `window_used`) captured from
  the Rust codec; the cross-language e2e exercises the domain against
  the Rust demo app (single-window → one `main` target).

## 3. Testing posture

The M2/M4/M5 pattern repeats — the framework-free mock adapter + a real
socket is the contract test, TDD throughout (each module's tests landed
first; the core `windows()`/scoping tests drove the fixture design).

- `crates/guiwright-core/tests/multi_window.rs` — the grafted
  `MultiWindowFixture` (two windows reusing local NodeIds, same-named
  "Save" buttons): enumeration (handles = TreeIds, titles from the
  window nodes, `main` for graft-free trees), strict-mode across
  windows, scope narrowing per window, scope hiding the other window's
  text, incremental window-local updates, `UnknownWindow` errors,
  `main` semantics on both shapes, window-close bumping only its own
  generations, window-relative bounds, cross-window hit-testing.
- `crates/guiwright-driver/tests/multi_window.rs` — session-level
  (`windows()`, scoped resolve + scoped `wait_for` honoring the scope,
  transient scope, unknown handles) and the §11 story end-to-end over a
  real UDS socket: **two connections, one session, two targets** — each
  `Use` aims only its connection, the same locator resolves to different
  nodes per connection; unscoped connections keep strict mode across
  windows; `Use` of an unknown handle fails clean (`UnknownWindow`) and
  applies nothing; a window-local diff arriving after `Use` satisfies a
  scoped auto-wait (the shared-script pump channel).
- `crates/guiwright-cli/tests/repl_e2e.rs` — the REPL `window` domain
  over a real socket: list (with titles, no active marker), use
  (confirmation), active-marker in the list, scoped `get` resolving the
  window's node (identified by its rect), failed use leaving the target
  unchanged, usage errors.
- `crates/guiwright-cli/tests/runner_e2e.rs` — `window use` inside
  `gwtest/v1` specs: the unscoped strict violation fails, the
  preview-window scenario passes, and the editor scenario proves the
  scope hides the other window's label from `expect`.
- `crates/guiwright-proto` — `window_domain_roundtrip`, the extended
  `wire_discriminants_are_append_only` pins (Window = 11, Windows = 12,
  WindowUsed = 13, WindowV1 List/Use, UnknownWindow = 8), the schemadoc
  counts + window domain table + error-codes tail, and the re-blessed
  golden; `golden_vectors.rs` gains the four window vectors the TS
  codec pins.
- `clients/typescript` — the vendored `gwp-schema.json` re-pinned by
  `schema.test`-style table checks (frontier assertions moved to
  `Window`/`WindowUsed`), window encode vectors byte-identical to the
  Rust frames, window response decoding, and the cross-language e2e
  step.

## 4. Upstream finding (recorded)

While building the fixture, one `accesskit_consumer` behavior surfaced:
an incremental update that **restates a subtree root** clears the root's
graft parent (`local_node_id == root` sets `parent_and_index = None`),
and only full updates re-link the graft — so an incremental restatement
of the window-content root silently detaches that window's subtree from
its window node for parent-chain purposes (children traversal still
works; ancestor walks like `is_descendant_of` stop). Real adapters shape
windows as *window → panel → content* and restate interior nodes, so
the fixture models that shape (window content hangs one level below the
subtree root; `MultiWindowFixture` docs carry the detail). Candidate
upstream contribution: re-link the graft parent on subtree-root
restatement instead of clearing it.

## 5. Deferred (recorded)

- Kit-embedded window targets: the embedded kit drives a Session
  directly and single-window is its contract today (component testing =
  widget-level, §16 "Component testing | MVP"); a kit-side
  `app.window(handle)` locator surface rides a follow-up when an
  embedded harness needs it — the driver, wire, REPL, runner and TS
  client carry the row.
- Per-window input routing (§9): raw input over GWP is still the
  gesture milestone's row; it will take the target window from the
  connection's active handle.
- Per-window traces/videos: the trace/video services record the
  session; window-tagged streams are a follow-up on top of the same
  handle vocabulary.
- `guiwright codegen <app-cmd>` (§13 overlay session), live trace-viewer
  streaming (§12 "live mode"), HTML per-scenario trace deep links —
  unchanged from M5.

## 6. Conclusion

M6 closes the last open version-tagged row of the §16 parity map below
v1. The multi-window story lands where AccessKit already lives: windows
are grafted subtrees, handles are TreeIds, scoping is a resolver filter
— no new id space, no protocol break, no adapter rewrite (the default
`windows()` is one `main` target, byte-compatible with every pre-M6
client). One session, many connections, each aimed at its own window —
the CDP targets shape with Playwright's ergonomics, over the same wire
that has served the kit, REPL, runner and TypeScript client since M0.
