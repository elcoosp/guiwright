# M16 — The embedded door: in-process transport + the public `Transport` seam

**Status:** implemented (this milestone opens §7's fourth door — the
in-process embedded transport in `guiwright-proto`, the driver's
`spawn_inprocess` stack, and the public `Transport` seam that turns the
transport-blind bring-up into a downstream extension point — with
byte-identical sessions, a budget row, an example and docs).

---

## 1. Why an embedded door, and why now

The §7 transport list read *UDS (default; also TCP for CI containers, WS
for the viewer)* — three doors, every one of them a socket. Each door
answers a real hosting shape, but the shape §5.2 actually spends most of
its time on — the in-process harness, the kit, downstream test
tooling — had to borrow the UDS door and pay for a filesystem socket +
a path it never needed. The m15 extraction of `conn::ClientCore` made
the cost of a fourth door embarrassingly low: the conversation is
already transport-blind, so "embed the whole remote-mode stack in the
process" is a listener plus a client wrapper. What was missing was the
seam: the driver's bring-up bridged listeners through a **private**
`IntoServe` trait, so a downstream TLS listener or named-pipe bridge
could not plug in even though nothing in the conversation cared.

M16 does both halves at once because they are the same decision: a
public `Transport` trait with the shipped doors as impls, and the
in-process door as its first downstream-shaped consumer.

## 2. Design: the seam is the trait; the door is one impl

The load-bearing decision is in `crates/guiwright-driver/src/server.rs`:
the private `IntoServe<S>` became `pub trait Transport`, carrying the
boxed serve future as its return type (`BoxServeFuture`) instead of a
trait-level parameter — downstream implementors see one method
(`into_serve`) and one type alias, not a generic puzzle. Consequences:

- **One bring-up path.** `GwpServerHandle::start` loses its `S`
  parameter entirely; `spawn_transport_with(listener, make_adapter,
  options)` is the public entry for any `Transport`, and every shipped
  spawn (`spawn_session_server_with`, `spawn_tcp_with`, `spawn_ws_with`,
  `spawn_inprocess_with`) is now just `start` with a different impl.
- **Endpoint honesty.** `Endpoint::InProcess` joins the enum: the
  embedded door and a custom transport expose no socket address for the
  handle to know, and the doc says so — a custom listener that does know
  its address surfaces it to its clients itself.
- **Byte-identical sessions.** The in-process door hands
  `tokio::io::duplex` pipe halves to the same `conn::run_connection`
  loop; a session over the pipe is the same frame sequence as over UDS,
  TCP or WS (`tests/wire_inprocess.rs` walks the full
  handshake → resolve → act → clock → bye conversation plus the M12
  event leg).

In `crates/guiwright-proto/src/duplex.rs` the door follows the house
transport shape exactly (accept loop + `ClientCore` client wrapper):

- `duplex::channel()` returns the two halves — a `DuplexListener` (the
  queue of pre-connected server pipe halves, fed to `duplex::serve` or
  the `Transport` impl) and a cloneable `DuplexConnect` handle clients
  dial with `DuplexClient::connect(&connect)`.
- **A dropped server fails connects honestly**: the connect handle's
  queue send errors with a `BrokenPipe` io error ("the in-process
  listener is gone") instead of queueing into the void — pinned by a
  proto unit test and by `connect_after_shutdown_fails_cleanly`.
- The pipe capacity (64 KiB) is documented against the frame guard and
  the blob side-channel: in-process traffic is command-and-event shaped,
  and a slow reader should not buffer a whole tree dump.

## 3. Surface

- `guiwright-proto`: `pub mod duplex` (`channel`, `serve`,
  `DuplexListener`, `DuplexConnect`, `DuplexClient`,
  `DUPLEX_BUFFER_BYTES`), root re-exports `DuplexClient` and `Handler`
  (a downstream transport author needs the bound nameable).
- `guiwright-driver`: `pub trait Transport` + `BoxServeFuture` in
  `server` (re-exported at the crate root), `GwpServerHandle::
  spawn_inprocess` / `spawn_inprocess_with` (return the handle **plus**
  the connect handle), `GwpServerHandle::spawn_transport_with`,
  `Endpoint::InProcess`.
- `guiwright-bench`: the `inprocess_ping_roundtrip` budget row
  (canonical `bench/budgets.toml` + the in-crate copy) — the embedded
  door's steady-state latency floor.
- Example: `crates/guiwright-driver/examples/wire_inprocess.rs`.

## 4. Tests and gates

- `guiwright-proto` (in `duplex.rs`): handshake → ping → bye round-trip,
  connect-after-listener-drop honesty, and the non-Hello rejection (the
  handshake gate behaves like the socket doors).
- `guiwright-driver`: `tests/wire_inprocess.rs` — the full remote
  session, the M12 event leg, a downstream-style `CountingTransport`
  wrapper (proves the seam is implementable outside the crate: the
  wrapper hooks `make_handler` per connection and delegates to the
  inner listener), and connect-after-shutdown.
- Budgets: `inprocess_ping_roundtrip` bound in `check_budgets`
  (release) — 16.5 µs p50 against a 25 µs design row.
- Example: `cargo run -p guiwright-driver --example wire_inprocess`.

## 5. Remaining gaps (honest list)

- **TLS / named-pipe doors** — the seam accepts them; none ship. A TLS
  transport is a downstream `Transport` impl away (or a reverse proxy in
  front of the TCP door, per the m15 note).
- **A kit-level embedded handle** — the kit drives sessions through its
  own adapter surface; wiring `spawn_inprocess` into a kit-side
  "remote-to-self" handle is a follow-on, deliberately not squeezed into
  this milestone.
- **The viewer live mode (§12/§16 v0.6)** — unchanged: the m15 doors
  plus this one are all its transport options; the viewer client is
  still the unstarted milestone.
