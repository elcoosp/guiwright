# M44 — The cancel-safe client read half (`FrameBuffer` + the buffered `ClientCore`)

**Status:** implemented. One reusable reassembler core
([`FrameBuffer`](crates/guiwright-proto/src/framing.rs)) + the client
conversation core's reads riding it
([`ClientCore`](crates/guiwright-proto/src/conn.rs)) — the m42 record's
named conn-level follow-up ("the deadline races the event read the way
the live feed's timeout already does; the honest fix is a cancel-safe
client read half, a buffered `conn` read"), closed exactly in the shape
the record stated. Every door inherits it: the UDS, TCP, WebSocket and
in-process duplex clients all share [`ClientCore`], so a deadline
dropping a read future mid-frame can no longer desync any session. No
wire change (the read half is byte-level state behind the same
conversation vocabulary), no schema change, no budget row (no new
conversation to measure — the existing `client_wait_satisfied` row is
the wait conversation's floor, and this pass only hardens its read
path; a second row would measure the same round trips twice, the m43
rule).

---

## 1. Why now

M42 shipped the client-side wait: `wait_for` subscribes, then races
`next_event` against a deadline with `tokio::time::timeout` inside its
wake loop (and the m43 REPL row runs the same seam over every
transport). The viewer's live feed has raced the same read since M18
(`LiveFeed::pump`'s pacing window). Both races sit on the client read
path's weakest joint: the reads went through the stateless
`framing::read_frame`, which consumes bytes straight off the socket —
a future dropped mid-frame (the deadline winning while a partial frame
sat half-consumed) lost those bytes, and the next read misinterpreted
the stream's tail as a length prefix. The server loop never had this
hole: M12 gave it the incremental `FrameReader` for exactly the same
hazard (`a_half_read_request_survives_a_concurrent_event`). The client
half rode the stateless path anyway — the m42 record flagged it and
deferred it; this milestone closes it.

## 2. Design: one reassembler core, both conversation halves

* **`FrameBuffer` is the seam.** The incremental reassembler's state
  (bytes read but not yet consumed as a frame) leaves `FrameReader`'s
  private field and becomes a named component in
  `crates/guiwright-proto/src/framing.rs`: `extend` appends one raw
  chunk, `try_take_frame` pops a complete body when one is buffered
  (`Ok(None)` mid-frame, the typed `FrameTooLarge` on a hostile
  prefix). `FrameReader` rides it unchanged — the server loop's
  contract and its tests are untouched — and the client core holds the
  same state, so "a dropped read future keeps its bytes" is one
  documented contract, not a per-side habit.
* **The client read half asks `AsyncRead`, not `AsyncWrite`.**
  `ClientCore`'s read side (`next_frame_body`, `read_response_demuxing`,
  `read_next_event`) moves into its own impl block over
  `S: AsyncRead + Unpin` — the buffered loop mirrors
  `FrameReader::next_frame` exactly (chunk in, frames out, EOF
  mid-frame as the typed `Io` error). The write side (`handshake`,
  `request`, `subscribe`, …) keeps the `AsyncWrite` bound. The two
  free functions that used to take `(&mut stream, &mut events)` are
  methods of the read half now — the conversation core owns its whole
  read path.
* **Every door inherits the fix.** `uds::Client`, `tcp::Client`,
  `ws::WsClient` and `duplex::Client` all wrap `ClientCore`; none of
  their code changes. The consumers that race the deadline — the m42
  `wait_for` wake loop, the REPL's wait row over `Connection` (the m42
  sugar), the viewer's `LiveFeed::pump` — become cancel-safe without
  touching their code.
* **Fail honest, fail bounded.** A desynced stream previously hung
  forever (a garbage length prefix under the 64 MiB guard just waits
  for bytes that never come). The hazard tests bound the recovery
  reads: the green path completes in milliseconds; the red path (the
  un-buffered read half) fails the bounded wait with the named
  "no desync" assertion instead of hanging the suite.

## 3. Surface

- `crates/guiwright-proto/src/framing.rs` — the `FrameBuffer` core
  (derive(Debug), `extend` / `try_take_frame` / the Debug-only
  `len`), `FrameReader` refactored to ride it, module doc stating the
  shared contract.
- `crates/guiwright-proto/src/conn.rs` — `ClientCore` gains the
  `buf: FrameBuffer` read half; the read-side impl block
  (`AsyncRead`-only bounds) with `next_frame_body` /
  `read_response_demuxing` / `read_next_event` as methods; the
  conversation impl (`handshake` / `request` / `next_event` / …)
  calls them; the doc comments name the m42/m44 races and the
  cancellation contract.
- `crates/guiwright-driver/examples/wire_wait_deadline.rs` — the
  consumer example: a wait that expires prints the typed timeout
  naming state/locator/last-evaluation, then the SAME connection
  serves a ping, a describe-all set read, and a second expiring wait.
- `docs/m44-*.md` — this record.

## 4. Tests

- `crates/guiwright-proto/src/conn.rs` (unit, byte-staged over a raw
  duplex — the client-side twins of the server's
  `a_half_read_request_survives_a_concurrent_event`):
  `a_half_read_event_survives_a_spent_deadline` — an event frame
  delivered halfway, the deadline expiring on it, the tail completing
  the frame on the next call, and a Pong following in order; and
  `a_timed_out_request_read_keeps_the_stream_aligned` — the same
  staging against `request` (a consumer wrapping the round trip in
  its own budget), the half-read Pong completing and the next frame
  arriving whole in order.
- Red→green: both tests failed against the un-buffered read half
  (the recovery read never completing — the consumed prefix lost to
  the dropped future; the bound turned the old infinite hang into the
  named failure), passed after the buffered half landed; the full
  proto suite went 66 lib + 10 integration tests green with zero
  warnings.
- `crates/guiwright-proto/tests/client_seam.rs` —
  `deadline_raced_event_reads_keep_the_door_conversation_aligned`:
  the composed shape over the embedded door — a quiet window expires,
  the pushed event lands whole on the next window, ping and bye keep
  flowing (the byte-level staging stays in the unit tests; this leg
  pins the composed behavior on a real connection).
- `crates/guiwright-driver/tests/wire_wait.rs` —
  `a_spent_wait_leaves_the_connection_healthy`: the consumer-level
  contract — a `wait_for` that expires leaves the same connection
  serving a ping, a full describe-all read and a second expiring wait
  (no stale subscription, no stale events).

## 5. Remaining gaps (honest list)

- No TypeScript change: the TS client's `FrameReader` already buffers
  (`clients/typescript/src/framing.ts`), so its `waitFor()` row (the
  m42/m43 records' named follow-up) never had this hole — the Rust
  half was the odd one out.
- The read buffer has no backpressure bound: a peer streaming frames
  nobody reads grows it unbounded until the 64 MiB frame guard
  rejects a hostile prefix. Honest by the §15.6 reasoning (the server
  loop's `FrameReader` runs the same shape), and the event queue's
  64-entry bound already degrades slow consumers driver-side.
- No wire/protocol surface: the fix is behind the conversation core,
  so the golden vectors and the schema are untouched byte-for-byte.
