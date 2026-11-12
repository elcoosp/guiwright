# M18 — The viewer live mode over a client-side connection seam

**Status:** implemented (this milestone cashes the gap both the M15 and
M16 records name first: the §12 viewer's live mode — "the transport is
served (`connectWs`, `spawn_ws`, `spawn_inprocess` are its doors), but
the viewer's live-mode client itself remains the unstarted milestone".
The unstarted client is now the shipped one: a [`ClientConn`] seam on
the proto side and a live feed + live timeline on the viewer side, plus
a budget row, wire tests, an example and the docs rows).

---

## 1. Why the seam, and why now

Every door the live mode needs is already served: the M12 event channel
pushes `TreeChanged`/`FramePresented` between responses, the M11
`Snapshot::Full` dump returns the whole tree as typed rows, and the
M15/M16 transports give a running session a WebSocket door and an
in-process door. What was missing was the client that consumes those
pieces as one moving picture — and the reason it was missing is a
duplicated API: `uds::Client`, `tcp::Client`, `ws::WsClient` and
`duplex::Client` expose four hand-copied copies of the same six-method
conversation surface. A transport-blind consumer would have had to pick
a concrete door, defeating the §7 point.

The fix mirrors the server side. `conn::Handler` is the server-side
seam — implement it and any listener serves the conversation. M18 adds
`client::ClientConn` as the client-side seam — implement it and any
consumer (the live feed, a downstream recorder) runs over any door. One
seam per conversation half; the symmetry is the design.

## 2. Design: the archive timeline, fed live

The load-bearing decision: the live mode does **not** invent a second
scrubber model. `viewer::live::LiveTimeline` accumulates frames, events,
logs and wire dumps into `ArchiveParts`-shaped buffers — the archive
reader's own record collections, now a public in-memory assembly path —
and `Timeline::from_parts` hands them to the *same* tick-union +
snapshot-diffs resolution `Timeline::from_archive` runs. The §12 "tree
inspector at any point, resolved from snapshot+diffs" logic exists once,
for both modes. Consequences:

- **Dump-per-change, not diff tracking.** The wire's `TreeChanged` event
  carries a seq, not a diff payload, so the feed fetches a full
  `Snapshot::Full` dump per change and records it as a snapshot. Every
  dump is self-contained; `tree_at` needs no intermediate diff records.
  The cost is one dump per change (the `viewer_live_sync` budget row
  guards it), the benefit is a live timeline that never drifts from the
  session.
- **Parent links rebuilt from the documented contract.**
  `SnapshotNodeV1::children` documents "DFS order + this count rebuilds
  the tree" — the live conversion is the first consumer to cash it
  (`rows_to_nodes`): a stack of (id, remaining children) walks the
  depth-first rows, and a row whose ancestors are all consumed starts
  the next window root. The rows land as `TreeNode`s — the trace
  schema's minimum vocabulary, byte-for-byte the archive's payload.
- **The frame axis is event-fed.** `FramePresented { seq, hash }`
  advances the axis (and the stamp the next record takes); the initial
  dump stamps at the feed's origin frame (0) until the first tick
  arrives — the same "records land on the frame they happened on"
  discipline the trace recorder uses.
- **Quiet is an answer, not an error.** `LiveFeed::pump(timeout)` wraps
  the wire's `next_event` in a tokio timeout and maps the quiet window
  to `Ok(None)` — pacing is the caller's, exactly like the client's own
  contract. A door that closes under the feed surfaces as
  `ViewerError::Wire` (the new `#[from] GwpError` variant) at the next
  pump, never as a silent frozen scrubber.
- **`ResponseV1` vocabulary is enumerated, not wildcarded.** The
  wire-misuse paths (`sync_tree` receiving a non-dump, `bye` receiving
  a non-Bye) name the offending response through an exhaustive variant
  match — a new append-only `ResponseV1` variant breaks that match on
  purpose, the same discipline the schema's discriminant pins enforce.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `pub trait ClientConn`
  (handshake / request / next_event / subscribe / unsubscribe / ping),
  implemented by the four shipped clients. Root re-export
  `guiwright_proto::ClientConn`; the crate doc names the seam next to
  `Handler`.
- `crates/guiwright-viewer/src/live.rs` — `LiveTimeline`
  (`record_frame` / `record_event` / `record_log` / `record_snapshot` /
  `timeline` / `tree_at` / `current_frame`), `LiveFeed`
  (`attach` / `sync_tree` / `pump` / `timeline` / `tree_at` / `bye` /
  `live_timeline` / `into_client`), `LiveChange`, `LiveInfo`.
- `crates/guiwright-viewer/src/archive.rs` — `ArchiveParts` published
  (the in-memory assembly path) and `ViewerError::Wire(#[from]
  GwpError)` appended.
- `crates/guiwright-viewer/src/timeline.rs` — `Timeline::from_parts`
  (the shared `build` constructor serves both `from_archive` and
  `from_parts`; no behavior change for archives).
- `crates/guiwright-viewer/Cargo.toml` — `guiwright-proto` + tokio
  (`time`) as dependencies; the async substrate rule (§5.2) holds: the
  viewer speaks the workspace's one runtime.
- `crates/guiwright-bench/src/samples.rs` + `bench/budgets.toml` — the
  `viewer_live_sync` sample and its budget row (both table copies).
- `crates/guiwright-viewer/examples/live_tree.rs` — the consumer
  example: attach over the embedded door, rename mid-session through
  the mock's shared-script channel, pump, resolve, print the scrubber.

## 4. Tests and gates

- `crates/guiwright-proto/tests/client_seam.rs` — the seam contract
  over all four doors (UDS, TCP, WS, embedded): one `exercise`
  conversation written against the trait runs through each door's
  client, handshakes counted per door.
- `crates/guiwright-viewer/tests/live.rs` — the fixture-backed live
  session (attach → sync → 8-node resolve), the mid-session rename
  (pump → auto-sync → renamed row), the session-less scripted door
  (attach takes the initial dump, sync takes one more, parent links
  rebuilt, quiet pump), the honest-failure pump after `Bye`, and the
  sync model unit tests.
- `crates/guiwright-bench` — `viewer_live_sync` bound in the guard
  registry; `cargo run --release -p guiwright-bench --bin check_budgets`
  guards the row in CI (§15.7).

## 5. Remaining gaps (honest list)

- **`trace.html` for live captures** — the HTML emitter wants a
  `TraceMeta` (os/arch/resolution), which a live feed has no honest
  source for mid-session; the follow-on decides the metadata story
  (probe at attach, or a live-flavored meta record).
- **The feed is one-shot per connection.** After `Bye` the connection
  is done; a reconnecting viewer re-attaches (and re-dumps) — the
  session-side resume vocabulary (a `Subscribe` replay cursor) stays a
  §7 follow-on.
- **The kit's `export_trace_viewer` still archives first.** Wiring
  `LiveFeed` into the kit's trace flow (live export during a test run)
  is a kit-side follow-on; M18 ships the viewer-side capability.
- The M16-era list (TLS / named-pipe doors, the kit's "remote-to-self"
  handle) is unchanged — M18 consumes the existing doors, it adds none.
