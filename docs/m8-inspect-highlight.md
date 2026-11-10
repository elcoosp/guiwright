# M8 — Inspector overlay & highlight: the §13 interactive session

**Status:** implemented (this milestone lands the overlay half of the
`Inspect` domain — `EnableOverlay` / `DisableOverlay` / `Highlight` — the
`FrameworkAdapter::show_overlay` seam, the session-side plan builder, and
the `inspect` surface in the kit, the REPL and the TypeScript client).
**Scope:** the promise the `InspectV1` doc comment has carried since M2 —
"M2 ships the codegen recorder half (§13); the live highlight handles
land with the interactive overlay session". The substrate existed since
M2: `guiwright-inspect` owns the framework-free [`OverlayPlan`] (hover
outline + role/TestId badges), the egui adapter paints it (`paint_overlay`,
painter layer after the app closure) and the masonry adapter converts it
(`overlay_draw_ops`, the `VisualLayerPlan` surface) — but nothing could
ask for it. The §13 sketch named the requests from day one:
`Inspect { EnableOverlay | Highlight(handle) | StartRecording |
StopRecording }`; only the last two existed.

---

## 1. Why the overlay is the milestone after gestures

The debugging loop is the product (§13 "the debugging loop that makes
Playwright pleasant"): `guiwright repl` resolves locators and runs
actions, but *why did this locator match that rectangle?* had no answer
on the wire. Codegen records what you did; the overlay shows where you
are — hover outline on the resolved node, badges on every interactive
node. M6/M7 made the wire append-only discipline routine; the `Inspect`
domain is the last §7 domain with named-but-missing requests.

| Architecture hook | What it names | Status before M8 |
|---|---|---|
| §7 Inspect domain | `EnableOverlay \| Highlight(handle)` | defined, server answered nothing (variants absent) |
| §13 "Overlay" | "adapter-level composition at present time" | plan + renderers existed, no driver path |
| §13 "badges from the current tree indexes" | plan built from live facts | `NodeFacts` existed (codegen), not fed from the tree service |
| `InspectV1` doc | "the live highlight handles land with the interactive overlay session" | this milestone |

## 2. Design

### 2.1 The plan is data; the adapter composites at present time (§15.4)

`FrameworkAdapter::show_overlay(Option<&OverlayPlan>)` is the only new
adapter hook. The session builds the plan **data-side** (facts from the
tree service, [`OverlayPlan::from_facts`] cost model — interactive-only
badges, the §13 default for live sessions) and hands it over; the adapter
paints it inside its current pass (egui: painter layer after the app
closure; masonry: stores + converts on demand — the vello-scene
attachment stays downstream as its docs record). The default impl is
`Err(Unsupported("inspect overlay (§13)"))` — the §6.2 honest row: an
adapter without the surface reports it instead of silently drawing
nothing (the egui/masonry/mock adapters implement it; the iced/freya/
slint/floem/gpui engines inherit the default until their glue lands).

While the session is on, every pump rebuilds the plan from the current
tree — badges are never stale, and the zero-polling contract is intact
for sessions that never enabled it (the refresh is `overlay_active &&
pump`). The cost is guarded by a new budget row (`inspect_overlay_plan`,
1k nodes: p50 ≈ 280 µs — the plan build dominates and is O(nodes)).

### 2.2 Highlight = resolve + aim (the wire shape vs the kit shape)

- **Kit shape** — `Session::inspect_highlight(&Selector)`: strict-mode
  resolve, then aim. Implies enable (the fewest-states rule: highlight is
  an inspect-surface action; nothing else to keep in sync).
- **Wire shape** — `Session::inspect_highlight_key(Option<NodeKey>)`: the
  client resolves (its `Resolve` request already auto-waits and keeps
  generation bookkeeping) and sends the handle. `None` clears the outline
  and keeps the session. Unknown handles are `CoreError::UnknownNode` →
  wire `ErrorCode::UnknownNode` — the variant existed since the first
  schema and this is its first caller.
- The reply is `Pong` for all three requests: the client renders the
  highlighted node's facts from its own resolution (the REPL prints
  `highlighted Button "Save"` from its `CurrentNode`), so no new response
  variant rides the V1 envelope.

### 2.3 Wire discipline (append-only, byte-pinned)

`InspectV1` appends `EnableOverlay` (2), `DisableOverlay` (3),
`Highlight { node: Option<NodeKeyV1> }` (4); `StartRecording`/
`StopRecording` keep 0–1 and every pre-M8 client keeps its byte-exact
encodings. The pinning test (`nested_domain_pins`) and four new golden
vectors (`inspect_enable_overlay`, `inspect_disable_overlay`,
`inspect_highlight`, `inspect_highlight_clear`) pin the shape; the
TypeScript schema/codec mirror the tables and byte-match all four vectors.

### 2.4 Surfaces

- **kit** — `app.inspect()`: `enable()`, `disable()`,
  `highlight(&selector) -> NodeFacts`, `clear_highlight()`, `is_active()`.
- **REPL** — `inspect on | off | clear | <get-args…>`; the args form
  reuses the exact `get` resolution path (bookkeeping identical, `last`
  keeps working) then highlights the resolved handle.
- **TypeScript** — `client.inspectOverlay(enable)` /
  `client.inspectHighlight(node | null)`.
- **example** — `cargo run -p guiwright-kit --example inspect_highlight`.

### 2.5 Error-structure cleanup (the same seam, honest end-to-end)

Landing `show_overlay` exposed how adapter errors travel: every session
site mapped `AdapterError` through `CoreError::Other(e.to_string())`, so
`AdapterError::Unsupported` reached the wire as `ErrorCode::Internal`.
The mapping now goes through one `adapter_error` helper that keeps the
structure (`Unsupported → Unsupported`), and the clock's paused
rejection — previously *string-matched* in the server
(`msg.contains("paused")`) — is the structured `CoreError::ClockPaused`.
The wire semantics are unchanged; the codes are now earned, not guessed.

## 3. Tests

- `guiwright-driver/tests/inspect_overlay.rs` — the session contract:
  enable hands a plan to the adapter, highlight aims the outline +
  returns facts, plan follows tree changes on pump, disable clears,
  unknown selector/key errors, the §6.2 `Unsupported` default.
- `guiwright-adapter-egui/tests/egui_embedded.rs::inspect_overlay_paints_into_the_pass`
  — the overlay provably paints: the frame digest changes with the
  session on and returns to the app-only digest after disable.
- `guiwright-kit/tests/inspect.rs` — the embedded surface (facts, plan,
  round-trip, strict failures).
- `guiwright-cli/tests/repl_e2e.rs::inspect_overlay_session` — the wire
  conversation over a real socket: `inspect on` → `inspect role Button
  Save` → `last` → `inspect clear` → `inspect off`, and the missing-node
  failure leaves the session alive.
- `guiwright-proto` — nested pins + four golden vectors; the TypeScript
  suite byte-matches them and drives `inspectOverlay`/`inspectHighlight`
  over a real TCP mock.
- `guiwright-bench` — `inspect_overlay_plan` sample + budget rows
  (`bench/budgets.toml` + the packaged copy), enforced by `check_budgets`.

## 4. Remaining gaps

- Full-tree badges (the §13 "deep-inspect mode") land with the viewer;
  live sessions composite interactive-only badges.
- Masonry's plan → `VisualLayerPlan` attachment rides the adapter
  integration pass (the conversion is the stable surface, as its overlay
  docs record); egui paints end-to-end today.
- The overlay does not ride `gwtrace/v1` archives yet (the plan is
  present-time state; replayable overlays belong to the viewer).

[`OverlayPlan`]: ../../crates/guiwright-inspect/src/overlay.rs
[`OverlayPlan::from_facts`]: ../../crates/guiwright-inspect/src/overlay.rs
