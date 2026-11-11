# M12 — The event channel: `Subscribe` over GWP

**Status:** implemented (this milestone lands the §7 server-push event
channel: the append-only `Subscribe { On { kinds } | Off }` domain,
pushed `Event` frames riding the response envelope, the live session
thread, the Rust + TypeScript client surfaces, the REPL's `events
on | off`, byte-pinned goldens, a budget row, example and docs).

---

## 1. Why the channel, and why now

The event machinery has been pre-seeded since the first schema: `EventV1`
(`TreeChanged` / `FramePresented` / `Log`) was published in the M0
tables, `conn.rs` carried a bounded event queue that "flushes between
requests" — and `_event_tx` was never wired to anything. Clients
"tolerated and dropped" event frames (in practice: a pushed frame would
have desynced a sequential reader); the m10 remaining-gaps note said
video-style streaming "rides the event channel's future milestones".
Every consumer case the architecture names — live viewers (§12), CI
harnesses watching an app under test, the REPL's live-debug view —
needs the server to talk first.

| Hook | What it names | Status before M12 |
|---|---|---|
| §7 event sketch | `TreeSubscribe { topics }` ⇒ `Evt::*` | published vocabulary, no transport |
| `conn.rs` event queue | bounded flush between responses | pre-wired, inert (`_event_tx` unused) |
| `uds.rs` client | "events tolerated and dropped for M0" | dropped nothing — nothing was sent |
| §16 | trace viewer "(time travel, **live**)" | the live half had no feed |

## 2. Design

### 2.1 Events ride the response envelope

The M0 sketch suggested a separate event frame stream. Decision:
**`ResponseV1::Event(EventV1)` (discriminant 17)** — every server→client
frame stays a `GwpResponse`, so a client demultiplexes one enum by
variant, never by position. An event and a reply may arrive in either
order; the Rust clients buffer `Event` frames while awaiting a request's
reply (`next_event` drains the buffer first), the TypeScript client
routes them to a bounded queue + waiter. A connection that never
subscribes never receives one — byte-identical conversations for
pre-M12 clients. The append-only discipline holds: `Subscribe` appends
at 15, `Event`/`Subscribed` at 17–18; `EventKindV1` mirrors `EventV1`'s
variant order.

### 2.2 The multiplexed connection loop is cancellation-safe

Activating the inert queue would have exposed a latent hazard: the
`select!` loop dropped its `read_frame` future whenever an event branch
won — bytes consumed from a partially-read request were lost. M12
replaces the ad-hoc reads with an incremental `FrameReader`
(the Rust mirror of the TypeScript client's), state in the struct, so a
half-read request survives any number of interleaved event flushes. The
pinned test delivers a request frame in two halves with an event queued
in between.

### 2.3 The session thread owns the hub; the idle tick is frame-cadence

`EventHub` (session-thread-owned, lock-free like `Session`) maps
connection ids to (filter, queue). Subscriptions arrive through the
command queue; the session thread publishes after every pump — one
**coalesced** `TreeChanged`/`FramePresented` per batch ("the tree is at
seq N now"; seq numbers expose gaps). While anyone is subscribed the
thread pumps at frame cadence (`FRAME_STEP`, the clock's step) so
events flow with no request in flight; with no subscribers the loop
blocks on the queue exactly like the pre-M12 thread (zero cost, §15.3).
Drops are the §15.6 backpressure: bounded per-connection queues,
`try_send` never blocks the UI thread.

### 2.4 Capability honesty

`EventKindV1::Log` is published but **not served**: naming it fails the
whole request with `ErrorCode::Unsupported` (all-or-nothing filters,
the M8 typed-error discipline). `Subscribed` echoes the deduplicated
kinds now active; re-subscribing replaces.

## 3. Tests + bench

- `guiwright-proto` — pins for the new variants; round-trips; connection
  loop tests: event flushed while idle, events interleaved with
  responses (order-agnostic by design), the half-read-request test,
  event-free conversations for handlers without an event stream,
  `FrameReader` split/coalesced/EOF tests.
- `guiwright-driver/tests/events.rs` — subscribe → events arrive with
  **no request in flight**; unsubscribed connections see nothing; the
  kind filter is honored; re-subscribe replaces; `Off` stops delivery;
  `Log` → typed `Unsupported`; TCP parity.
- `guiwright-cli` — `events on | off`; events flushed between commands
  ahead of the next command's output (pinned e2e).
- TypeScript — `subscribe()`/`nextEvent()`/`unsubscribe()`, golden
  vectors (`subscribe_on`, `subscribe_off`, `event_*`, `subscribed`),
  the client demux test against a hand-rolled TCP server, vendored
  schema re-pin; the cross-language e2e subscribes and reads a real
  event off the wire.
- `guiwright-bench` — `wire_event_delivery` (publish → receipt; the
  frame tick is the designed liveness bound; measured p50 ≈ 17.7 ms on
  the dev host) + the row in both `budgets.toml` copies.

## 4. Remaining gaps

- `Log` streaming rides the log-tail milestone (a per-session
  `tracing` tap → `EventV1::Log`); the kind is published so appending
  it later is an append, not a reshape.
- A subscriber-visible "dropped" counter (the hub counts drops per
  queue; surfacing them on the wire is a later append if a consumer
  shows up).
- WebSocket transport in Rust (the TS client ships `connectWs`; the §7
  transports list records the constraint).

[`EventHub`]: ../../crates/guiwright-driver/src/events.rs
[`FrameReader`]: ../../crates/guiwright-proto/src/framing.rs
