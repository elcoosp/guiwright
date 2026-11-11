# M14 — The wire log tail: `EventKindV1::Log` served

**Status:** implemented (this milestone cashes the m12 remaining-gaps
note verbatim — *"a per-session `tracing` tap → `EventV1::Log`"* — and
lands the event channel's log half: the opt-in per-session tap, the
`Log` records streaming as one `logs.ndjson`-shape JSON line per frame,
the REPL's `events on` carrying the kind, byte-pinned schema rows, a
budget row, example and docs).

---

## 1. Why the log tail, and why now

The `Log` kind has been published since the first schema: `EventV1::Log
{ record }` rode the M0 tables, the M12 hub already matched it
(`event_matches`' third arm), and the TypeScript client typed it — while
the driver answered every subscription naming it with a typed
`Unsupported`. The honest capability answer was the right M12 shape; it
also pre-declared the milestone. The consumer case is the debugging
follow-up the other two kinds cannot cover: a remote session's tree and
frame events say **what** changed, the log records say **why** — the
session thread's own diagnostics (subscription lifecycle, overlay
refresh failures, codegen selector misses) and everything the adapter
emits while pumping. The §7 sketch named the topic `Logs` next to
`TreeChanged`/`FramePresented` from day one.

| Hook | What it names | Status before M14 |
|---|---|---|
| m12 §4 remaining gaps | "a per-session `tracing` tap → `EventV1::Log`" | the note this milestone cashes |
| `EventKindV1::Log` | published kind, `Unsupported` at the server | honest rejection, no feed |
| `events.rs::event_matches` | the `Log` fan-out arm | matching, never exercised |
| §12 `logs.ndjson` | the `tracing`-fed record vocabulary | archive-only, not on the wire |

## 2. Design

### 2.1 The tap is per-session and opt-in (`ServerOptions::log_tail`)

Enabling the tail wraps the session thread in a **thread-local
dispatcher** (`tracing::subscriber::set_default`) whose only layer is
the tap. Three consequences fall out of that one decision: other threads
keep the process subscriber (an app's global tracing setup is untouched
off the session thread), two servers never see each other's records (no
process-global state, tests spawn servers freely), and a session spawned
without the tail keeps the byte-exact pre-M14 conversation **and** the
process-wide tracing behavior — the default is the exact pre-M14 shape.
`TRACE`-level records stay off the wire (`DEBUG` and above); the tail is
remote diagnostics, not a second trace recorder — that is §12's job.

`spawn_session_server_with` / `GwpServerHandle::spawn_with_options` /
`spawn_tcp_with` carry the new `ServerOptions { seams, log_tail }`; the
existing `spawn_session_server` / `spawn_with_seams` / `spawn_tcp`
delegate with defaults, so every pre-M14 call site compiles unchanged.

### 2.2 A bounded live tap, never a replay buffer

`LogTail`/`LogTap` (`crates/guiwright-driver/src/log_tail.rs`) are a
bounded channel (`LOG_TAIL_CAPACITY` = 256) with the §15.6 discipline:
`try_send` never blocks, a full tail drops the incoming record and
counts it (`dropped()`), a stray tap clone after the tail is gone is
inert. The publish point drains the tail **unconditionally** — records
emitted while no `Log` subscriber exists are discarded, never replayed
to a later subscriber — and builds `EventV1::Log` frames only while
`EventHub::wants(Log)`. The all-or-nothing capability contract keeps its
M8 shape: a session without the tail still rejects a `Log` subscription
with `ErrorCode::Unsupported`, with a message that names the knob.

### 2.3 The record format is the archive's

Each record is one `logs.ndjson`-shape JSON line — `ts_ns`, `level`,
`target`, `pid`, `message`, plus the event's structured fields — the
same vocabulary `guiwright-trace`'s `TraceLogLayer` writes into the
archive, so a remote consumer parses one shape for both. The wire type
stays the pinned `EventV1::Log { record: String }` (no schema change);
the REPL pretty-prints `level target: message` and passes a non-JSON
record through raw.

## 3. Tests + bench

- `guiwright-driver` unit — `log_tail.rs`: tap→tail round-trip order;
  the full-tail drop + counter (both ends report it); stray clones inert
  after the tail is gone; the layer's `logs.ndjson` vocabulary (level,
  message, structured field, target, ts_ns, pid); `TRACE` filtered;
  `Debug`-field quoting.
- `guiwright-driver/tests/events.rs` — a `Log` subscription without a
  tail is a typed `Unsupported` (the M12 pin, adapted to the shipped
  contract); with the tail it is served and the session's own
  registration record arrives as one JSON line (level/target/message/
  ts_ns pinned); re-subscribing logs again; a mixed
  `TreeChanged + Log` filter delivers both kinds and lets no frame
  through.
- `guiwright-cli` — `events on` subscribes tree + frame + log and the
  drain pretty-prints the tap's own record ahead of the ack (pinned e2e
  over a real socket, the session spawned with `log_tail: true`).
- TypeScript — no wire change: the `log` kind and `{ kind: "log";
  record }` shapes were already typed; the vendored `gwp-schema.json`
  re-pinned against the updated `event_kind` description row (the
  `npm test` vendored-schema check + golden vectors stay green).
- `guiwright-bench` — `log_tail_batch` (tap send + drain + frame build
  for one 64-record batch; measured p50 ≈ 2.5 µs on the dev host — the
  wire round-trip stays the `wire_event_delivery` row) + the row in both
  `budgets.toml` copies.

## 4. Remaining gaps

- App-fed records: `LogTap` is public (tests, benches, custom hosts),
  but a spawned server's tap is created internally — an app-side
  `ServerOptions.log_tap(tap)` knob is a later append if a consumer
  shows up.
- A subscriber-visible dropped counter for the log tail itself (the
  m12 note's queue-drop counter and this tail's `dropped()` are two
  counters; surfacing either on the wire is a later append).
- The session thread's records route through the thread-local dispatcher
  while the tail is enabled — an app that also wants those records in
  its own subscriber composes the two explicitly (documented in
  `ServerOptions::log_tail`).

[`LogTail`]: ../../crates/guiwright-driver/src/log_tail.rs
[`EventV1::Log`]: ../../crates/guiwright-proto/src/schema.rs
