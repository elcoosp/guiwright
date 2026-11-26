# M97 — The wire Trace domain: the §12 archive over any door (the Start/Stop arms the §7 sketch named)

**Status:** implemented, green end to end. The reconciliation's `missing`
verdict paid: the architecture doc's §7 protocol sketch names
`Trace { Start(opts) | Stop | Attach(stream) }` in the "trace &
inspector" domain block, and the repo shipped exactly one of the three
arms — the M18 live feed is the **Attach** half (the §7 event channel +
the viewer's `LiveFeed`), while the **Start/Stop** control arms never
landed: no `RequestV1::Trace` variant existed, the driver owned no
trace service, and the client seam answered `video(cmd)` (M55) with no
`trace` sibling. The kit's embedded `enable_trace` path started the same
`gwtrace/v1` recorder in-process — a remote client over UDS/TCP/WS had
no way to ask the session to record. This record pays the control half;
no earlier record scoped it out (grep of `docs/m*.md`: no "Trace
domain" deferral), the README's milestone table never claimed it, and
the §16 trace-viewer row's mechanism column says "§12" — the wire's
recording half was silently missing.

---

## 1. Why now

The reconciliation table (plan ↔ code ↔ history) surfaced three
unclaimed plan items, in doc order: §7's Trace domain (Start/Stop),
§12's WASM-viewer aside ("the WASM viewer is §12's full vision" — a
vision note, no milestone row, no deliverable claim: stale/vision, not
eligible) and §15.7's macro bench. The Trace domain is the earliest and
the most seam-shaped: every dependency it names already existed in code
— the `guiwright-trace` recorder (`TraceRecorder::start(TraceOptions)`,
the §12 zip writer), the driver session's taps (the pump integrates
tree diffs and presented frames; the dispatch sees every protocol
call; the clock service keeps the §14 ledger), and the M55 video
domain as the exact wire pattern to mirror (client-owned artifact dir,
single-slot start, "idempotence via error" stop, the reply union named
client-side). The gap was asymmetric by surface: the TS client spoke
`videoStart()`/`videoStop()` since M3 and the Rust seam since m55, but
the archive — the trace viewer's own artifact — rode no door.

## 2. Design: the driver records the session from its own taps

Three decisions carry the milestone:

* **The archive is the session's, not the client's.** `Trace`'s Start
  arms the `TraceRecorder` on the session (the same crate the kit's
  embedded path starts), fed from the taps the driver already runs:
  the pump's `TreeDiffSummary` folds into the recorder's
  `trees/diff.bin` rows (the diff the pump previously discarded), the
  presented frames ride the video capture's whole-frame readback into
  the hash-deduped `frames/` PNG set (gated on the adapter's §6.2
  pixel row — a pixel-less adapter records `frames: 0`, never a
  guessed raster), and every dispatched request lands as one seq'd
  `events.ndjson` protocol-call row (the dispatch tap records before
  the match consumes the request). The virtual-clock ledger folds into
  `clock.bin` at stop (§14 "clock.bin in traces replays timing") — the
  archive is finalized at stop, so the whole ledger lands once. The
  `logs.ndjson` row stays the honest cap it already is: the session's
  log tail streams `EventV1::Log` live (M14) and no surface today
  tees it into the archive — the kit's own archives don't either.
* **The wire shape mirrors the video domain byte-for-byte in spirit.**
  `RequestV1::Trace(TraceV1)` appends at discriminant 24 (the describe
  family ends at 23); `TraceV1::Start(TraceOptsV1) | Stop` at 0/1 —
  the `VideoV1` shape. `TraceOptsV1` carries the client-owned subset:
  `test` (the §12 `meta.json` label) and `dir` (Playwright parity: the
  client owns the artifact dir; `None` = `GUIWRIGHT_TRACE_DIR` or the
  OS temp dir). The reply appends at 27:
  `ResponseV1::TraceFile { path, events, frames, records_dropped,
  archive_bytes }` — the stats that name what the archive carries.
  The connection's window scope tags the stream (`TraceOptions::
  with_window`), the m6 §5 handle vocabulary the video scope rides.
  The client-side carrier is the M55 union shape:
  `ClientConn::trace(cmd) -> TraceReply { Started | File(TraceOutput) }`
  — never a collapsed `Option`.
* **No new hot path.** The recorder's SPSC ring absorbs the taps; the
  pump's only added cost while recording is the frame PNG encode the
  video capture already pays (and only while an archive records).
  The protocol tap is one `Option::is_some` check per request. No
  budget row: the archive is not a per-frame hot path (the m54-family
  rule) — the `client_trace`-shaped cycle is a stop-time finalize, and
  §15.1 gains nothing it did not already measure.

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — `RequestV1::Trace`
  (discriminant 24), `TraceV1`/`TraceOptsV1`, `ResponseV1::TraceFile`
  (discriminant 27); the m97 discriminant pins + round-trip test.
- `crates/guiwright-proto/src/client.rs` — the `TraceReply` /
  `TraceOutput` union + `ClientConn::trace` (every impl inherits).
- `crates/guiwright-proto/src/schemadoc.rs` — the request/response
  tables + the `domains.trace` table; `tests/schema-golden.json` and
  the published `clients/typescript/gwp-schema.json` blessed together.
- `crates/guiwright-proto/tests/golden_vectors.rs` — the
  `trace_start` / `trace_stop` request vectors + the `trace_file`
  response vector (55 requests, 33 responses in `golden.json`).
- `crates/guiwright-driver/src/session.rs` — the trace service:
  `trace_start_in` / `trace_start` / `trace_stop`, the pump's diff +
  frame feeds, the stop-time clock-ledger fold.
- `crates/guiwright-driver/src/server.rs` — the `Trace` routing arm +
  the dispatch's protocol-call tap (`protocol_kind`).
- `crates/guiwright-driver/tests/wire_client_trace.rs` — the seam over
  the in-process door: the ack arm, the finalized-archive arm, the
  pixel-less honest `frames: 0`, the double-start rejection, the bare
  stop rejection.
- `crates/guiwright-driver/examples/wire_client_trace.rs` — the
  consumer demo (`cargo run -p guiwright-driver --example
  wire_client_trace`).
- `clients/typescript/src/{schema,codec,client}.ts` — the `Trace: 24`
  / `TraceFile: 27` tables, the `TraceV1`/`TraceOptsV1`/`TraceFile`
  types, the codec rows, `traceStart()` / `traceStop()`.
- `clients/typescript/test/trace.test.ts` — the composition pins + the
  byte pins against the golden vectors; `test/codec.test.ts`'s vendored
  copy + frontier pins move with the schema.
- `clients/typescript/test/e2e-full-stack.test.ts` — the cross-language
  leg against the real Rust demo door (start → resolve inside the
  window → stop → the finalized stats + the archive on disk).
- `clients/typescript/examples/trace_archive.ts` — the consumer demo
  (`npm run example:trace`).

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_trace` — 4/4.
- `cargo test -p guiwright-proto --lib` — the m97 pins + round trip
  join the 100-test module.
- `GUIWRIGHT_BLESS=1 cargo test -p guiwright-proto --test
  golden_vectors` + `--test schema_golden` — both goldens blessed; the
  TS side re-pins the vendored copy (the two clients fail the build
  together on drift).
- `npm test` — the trace pins + the vendored-copy pins join the
  suite; `npm run e2e` — the full conversation green including the
  m97 trace leg over TCP.
- No budget row (the m54-family rule): the archive rides the existing
  session taps; no §15.1 row moves.

## 5. Remaining gaps (honest list)

- The archive's `logs.ndjson` row stays empty: no surface tees the
  session's `tracing` tap into the recorder (the kit's archives don't
  either) — the live `EventV1::Log` door (M14) remains the log path.
- The Stop arm finalizes the whole archive in one round trip: a
  session-sized zip is written on the caller's request thread (the
  recorder's writer thread assembles; the stop joins it). A streaming
  stop (chunked reply) would matter only for archives beyond the
  §12 budget cap — no consumer asks.
- The window-scope tag rides the connection's aimed window (the video
  scope rule); the archive's per-window *crop* does not — the trace
  archive records the whole session's tree diffs and frames, the tag
  names the stream in `meta.json`. A cropped archive would re-open
  §10's crop rules at event width; no consumer asks.
- §12's WASM-viewer aside ("the WASM viewer is §12's full vision")
  stays unclaimed drift: a vision note with no milestone row — the
  reconciliation reports it; no record scopes it in or out.
