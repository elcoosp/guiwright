# M23 — The live-capture HTML emitter

**Status:** implemented (§12's recorded follow-on: "the `trace.html`
emitter for live captures stays a follow-on" — the M18 live feed built
the archive's scrubber model but could not emit the HTML, so a live
capture lived only inside the process that held the feed).

---

## 1. Why now

M18 closed the model half of the live viewer: `LiveTimeline` accumulates
frames, events, logs and wire dumps in the archive's own vocabulary and
[`LiveTimeline::timeline`] hands back the exact scrubber the archive path
builds. The §12 record kept one piece explicitly open: the self-contained
`trace.html` emitter. Without it, a live capture could not leave the
viewer process — nothing to attach to a bug report, nothing to open in a
browser after the session ends. The emitter is the last §12 row the live
half owed.

## 2. Design: one payload builder, two headers

The load-bearing decision: **the emitter is not rewritten — it is
shared**. `build_payload` takes the four things that differ between the
archive and a live capture (the test label, the adapter name, the schema
stamp, the logs) plus the same `Timeline` both paths already build, so
live and archive emit the byte-identical template — one code path for
the scrubber, the §12 discipline the timeline itself already follows.

- **The schema stamp is honest**: a live capture reads
  `gwtrace/v1 (live)` — a capture point of a running session, not a
  finished archive. The header line the browser composes carries it
  exactly where the archive header carries `gwtrace/v1`.
- **The adapter name rides the attach report** (the wire-up): attach
  records the `Welcome`'s `AdapterInfoV1::name` into the timeline —
  before M23 the attach report was printed and discarded. A model
  without an attach (hand-built `LiveTimeline`s, the unit-test shape)
  renders `adapter (unknown)`.
- **No new wire surface** — the emitter consumes the accumulated model;
  nothing on the GWP side changes, and `render_html` on the feed is a
  pure read of what has landed so far.

## 3. Surface

- `crates/guiwright-viewer/src/html.rs` — `render_live_html(&LiveTimeline)`
  (the §12 follow-on) and the shared `build_payload(test, adapter,
  schema, timeline, logs)` builder both entry points feed.
- `crates/guiwright-viewer/src/live.rs` — `LiveTimeline` carries the
  adapter name (`set_adapter` / `adapter` / `test` / `logs` accessors),
  `LiveFeed::attach` records it, and `LiveFeed::render_html` renders the
  capture so far.
- `crates/guiwright-viewer/src/lib.rs` — `render_live_html` re-exported
  beside `render_html` at the crate root.

## 4. Tests

- `crates/guiwright-viewer/src/html.rs` (inline) — a live capture renders
  the same template with the test label, adapter name and `(live)` stamp
  riding the payload JSON (the browser composes the header line), the
  dump rows and angle-escaped logs included, still self-contained; and
  the honest `(unknown)` adapter before one is recorded.
- `crates/guiwright-viewer/tests/live.rs` — end-to-end over the embedded
  door: attach records the adapter name (`feed.live_timeline().adapter()`)
  and `feed.render_html()` produces the scrubber mid-session.
- `crates/guiwright-viewer/examples/live_tree.rs` — the consumer example
  now writes the live capture to a file and prints the path.

## 5. Remaining gaps (honest list)

- The live HTML is a **capture point**, not a stream: it replays what
  the model held when it was written. A browser-side auto-refresh
  (re-fetch or a WS-fed page) stays out of scope — the §12 WASM vision
  is the long-term shape, and the archive remains the file format.
- No budget row: the emitter reuses the archive payload builder (one
  `serde_json` pass over the model), the live steady state is already
  pinned by the `viewer_live_sync` row, and `render_html` is off the
  per-frame path by construction.
