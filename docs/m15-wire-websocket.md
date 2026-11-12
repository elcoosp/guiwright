# M15 — The WebSocket transport in Rust: `ws://` served

**Status:** implemented (this milestone cashes the m12 remaining-gaps
note verbatim — *"WebSocket transport in Rust (the TS client ships
`connectWs`; the §7 transports list records the constraint)"* — and
opens §7's third door server-side: an RFC 6455 server + client in
`guiwright-proto`, the driver's `spawn_ws` stack, the REPL/runner's
`ws://` target form, byte-identical sessions, a budget row, the
cross-language e2e leg and docs).

---

## 1. Why the WebSocket door, and why now

The §7 transport list named three doors from day one — *UDS (default;
also TCP for CI containers, WS for the viewer)* — and the M3 TypeScript
client shipped its half of the third door (`GwpClient.connectWs`, typed
as "the viewer live-mode door"). Until M15 the Rust side answered with
silence: `connectWs` had no server to open, the m12 note recorded the
gap, and every "live viewer" sketch in §12 dead-ends at *"streams over
the wire"* — a browser tab cannot dial a Unix domain socket. The
WebSocket upgrade exists precisely for that shape: browser-friendly,
proxy-traversable, and — for a protocol that is already length-prefixed
binary frames — a trivially thin pipe. Building it as milestone fifteen
rather than a mid-M3 side quest kept the wire schema frozen while the
transport caught up: zero schema edits, zero new `ResponseV1` variants,
zero client changes beyond the fix below.

The milestone also carried the micro-debt the third transport exposed:
the UDS and TCP clients were copy-paste twins (the m3 note's known
debt). A third copy would have made the drift permanent, so M15 first
extracted the client conversation into one `conn::ClientCore<S>` and
made all three transports delegate to it.

## 2. Design: WS as a byte-stream adapter, not a second protocol

The load-bearing decision is in `crates/guiwright-proto/src/ws.rs`:
`WsStream` implements tokio's `AsyncRead` + `AsyncWrite` over the RFC
6455 frame codec. The GWP conversation — `conn::run_connection`'s
handshake gate, dispatch, event multiplexing, and `crate::framing`'s
length-prefixed postcard — runs on it unchanged. Consequences:

- **Byte-identical sessions.** A WS session carries the same frames as
  the UDS and TCP sessions; the cross-language e2e asserts the same
  snapshot/resolve/act results on both doors ("M15 ws: snapshot →
  resolve → act byte-identical to TCP").
- **One server loop.** The driver's `GwpServerHandle::start` stays
  transport-blind; `spawn_ws` wraps the listener in a `WsListener`
  newtype whose accept loop upgrades each stream (`ws::accept`, RFC
  6455 §4.2) and then hands the session to the same connection loop the
  UDS and TCP doors run.
- **One client core.** `WsClient` owns the upgrade and delegates the
  conversation to `ClientCore` — the same struct behind `uds::Client`
  and `tcp::Client`. `connect` tolerates a `ws://` scheme prefix (the
  `ConnectTarget::Ws` storage keeps the URL verbatim, mirroring
  `--tcp`) and rejects `wss://` with an honest protocol error: the
  driver speaks to loopback peers in the clear, and TLS would be a
  dependency the workspace does not carry.

Frame handling follows the RFC where it matters and the driver's own
contracts where it can:

- Client frames are masked with a fresh key per message (§5.3); server
  frames are unmasked. The mask source is a hashed `RandomState` seed
  plus a counter — masks defeat intermediaries, not attackers, and the
  §7 threat model never included a hostile network.
- Inbound: pings are answered automatically (a lost Pong is always
  legal, so a partial control write drops it rather than corrupting the
  stream), close is parsed once and everything after it is ignored,
  and fragmented messages reassemble across continuation frames with
  interleaved control frames allowed.
- Outbound: every GWP frame ships as exactly one FIN-carrying binary
  WS message (the driver never fragments), control frames (the close
  reply) jump the application queue, and a partially-sent message
  survives the flush future being dropped — the offset is state, no
  byte is written twice or skipped.
- The 16-bit and 64-bit length forms are exercised by a test that
  forces a 300 KB response through them (`large_gwp_frames_ride_the_64_
  bit_length_form`), and declared lengths beyond the 1 GiB guard are
  rejected before buffering.

SHA-1 and base64 live in-module (pinned by RFC 3174 / RFC 4648 / RFC
6455 §1.3 vectors) so the transport stays dependency-free — the
workspace gained zero crates for its third transport.

## 3. The client fix the server exposed

`GwpClient.connectWs` (M3) never had a peer to test against, and it
showed: `onceEvent` assumed a node `EventEmitter` (`.once()`), while
the WHATWG `WebSocket` is an `EventTarget` (`addEventListener`) — the
first real `connectWs` call died on its own open await. The fix teaches
the helper both shapes; the TCP path (an `EventEmitter`) is untouched
and the unit tests still pin it. This is exactly the class of bug the
cross-language e2e exists to catch, and the reason the M15 leg now runs
in every `npm run e2e`.

## 4. Surface

- `guiwright-proto`: `ws::{WsStream, WsClient, Client, accept, Mode}`,
  exported as `guiwright_proto::WsClient`; `conn::ClientCore<S>` (the
  extracted client conversation, private to the crate).
- `guiwright-driver`: `GwpServerHandle::{spawn_ws, spawn_ws_with}`,
  `Endpoint::Ws(SocketAddr)`; the demo app gained `--ws` (prints
  `GWP_WS_ADDR=`); the `wire_ws` example walks the live-viewer flow
  (upgrade → subscribe → app-side rename → pushed `TreeChanged` →
  resolve → bye).
- `guiwright-cli`: `--ws ws://host:port` on `repl` and `test` (mutually
  exclusive with `--uds`/`--tcp`), `ws://host:port` bare targets, and
  the spec-file `target` form (§5.3 `GUIWRIGHT_DRIVER`).
- `guiwright-bench`: the `ws_ping_roundtrip` budget row (canonical
  `bench/budgets.toml` + the in-crate copy).
- TypeScript: the `connectWs` EventTarget fix and the e2e leg.

## 5. Tests and gates

- `guiwright-proto` (in `ws.rs`): RFC vectors (SHA-1 ×3, base64 ×7,
  accept-key ×1), frame-grammar unit tests (round-trips across all
  three length forms, partial frames, fragmentation + interleaved
  controls, orphan continuation, post-close bytes, masked client
  frames, oversized lengths), and socket integration (handshake →
  ping → bye, a GWP frame split across flushes, WS-level ping →
  auto-Pong, the 300 KB length-form response, a rejected plain-HTTP
  upgrade, and the M12 event leg) — 58 lib tests total.
- `guiwright-driver`: `tests/wire_ws.rs` — the M3 remote-session twin
  (handshake → resolve → act → clock → bye) and the M12 event leg over
  `spawn_ws`.
- `guiwright-cli`: `repl_conversation_over_ws` (the same REPL steps as
  the TCP parity test) plus the target-parsing unit tests.
- Cross-language e2e (`npm run e2e`): the M3 TCP conversation, then the
  M15 leg against `gwp_demo_app --ws`.
- Budgets: `ws_ping_roundtrip` bound in `check_budgets` (release).

## 6. Remaining gaps (honest list)

- **wss:// / TLS** — intentionally not served: loopback peers, no TLS
  dependency in the workspace. A viewer that needs TLS should terminate
  it in a reverse proxy; the wire bytes do not change.
- **Compression extension** (`permessage-deflate`) — not negotiated;
  GWP payloads already ride LZ4 on the blob side-channel for anything
  large, and control latency does not compress meaningfully.
- **Viewer (§12/§16 v0.6)** — the transport is served; the viewer
  itself remains the unstarted milestone. `connectWs` and `spawn_ws`
  are its doors.
- **Autobahn test suite** — the RFC vectors + integration tests cover
  the grammar the driver actually speaks (unfragmented sends, masked
  client, one close code); a full fuzz pass is worthwhile before any
  third-party peer claims compatibility.
