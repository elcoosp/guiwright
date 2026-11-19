# M55 — The client seam's video row (`ClientConn::video`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`VideoReply`] union and its [`VideoOutput`] carrier — the §10 recording
half as typed sugar, the one-milestone-per-surface pattern
(m46→…→m54) carried to the video domain, the §10 half the screenshot
rows (m50) left as the wire's raw arm. A downstream Rust consumer starts
the presented-frame recorder and stops it with one call taking the
wire's own `VideoV1` enum — the same vocabulary the TypeScript client
has spoken since M3 (`videoStart()` / `videoStop()`) and the REPL since
M3 (`video start` / `video stop`). No wire change: the command rides the
`Video` domain byte-for-byte as M3 blessed it; the schema, the golden
vectors and the variant tables are untouched. One budget row
(`client_video`, the record cycle over the in-process door).

---

## 1. Why now

The §10 capture story has two halves on the wire: the still read
(`Screenshot`, sugared by m50) and the recording
(`Video { Start(opts) | Stop }` → `VideoFile { path, frames, dropped,
duration_ms }`). Every other surface speaks the second half — the REPL's
`video start` / `video stop` rows and the TypeScript client's
`videoStart()` / `videoStop()` — but the Rust seam, the mirror every
downstream implementer sees, stopped at the raw `RequestV1::Video` arm:
a remote Rust suite wanting Playwright-style video artifacts
hand-built the request, matched `ResponseV1::Pong` for the start ack and
destructured `ResponseV1::VideoFile` by field (the exact shape
`remote_session.rs`'s `video_domain_over_the_wire` spike has carried
since M3). The m51 rule — take the wire's own enum, no parallel
client-side vocabulary — was applied to the clock, mock and inspect
surfaces; the video row is the same promise on the §10 recording half,
the last §10 arm the seam left raw.

## 2. Design: the §10 recorder, as sugar

* **One method, the whole §10 video vocabulary.** `video(cmd: VideoV1)`
  takes the wire's own enum (start with the client-owned artifact dir +
  fps cap, stop) — no parallel client-side vocabulary to keep in sync,
  the same rule the m48 sugar set for `GestureV1` and the m51/m52/m53
  sugars set for the rest.
* **The union, not the variant.** The domain's replies are heterogeneous
  by wire design — `Start` rides the shared `Pong` ack (the recorder is
  live), `Stop` carries the `VideoFile` payload. `VideoReply` names that
  shape client-side (`Started` / `File(VideoOutput)`) instead of
  collapsing it into a bare `Option` that hides which command answered
  what — the same honesty the m53 `InspectReply` union draws. The
  `VideoOutput` carrier mirrors the wire payload field-for-field (path,
  encoded frames, dedup-dropped frames, wall-clock span); never
  serialized itself, so the schema stays byte-pinned (the
  `ActionReceipt` rule).
* **The honesty is pinned as shipped.** The recorder needs the adapter's
  §6.2 pixel capability row, so an adapter without a pixel path answers
  the typed `Unsupported`; a second `Start` while a recording runs
  answers the session's typed rejection (single-slot, the M3 rule); a
  `Stop` with nothing running answers the same rejection ("idempotence
  via error", mirroring the `Clock` rules). The sugar surfaces, never
  re-maps. The wire tests pin all three.
* **No wire change and no new capability flags.** The command rides the
  `Video` domain byte-for-byte; a driver that predates the video domain
  answers the typed `Unsupported` and the sugar surfaces that rejection
  like every other pre-domain door.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::video` and the
  `VideoReply` / `VideoOutput` types.
- `crates/guiwright-driver/tests/wire_client_video.rs` — the
  conversations: start ack + the finalized artifact (the file really
  exists in the client-owned dir, at least the first presented frame
  encoded), the double-start typed rejection, and the bare-stop typed
  rejection.
- `crates/guiwright-driver/examples/wire_client_video.rs` — the consumer
  example over the in-process door (the start ack, the double-start
  rejection, the finalized artifact summary, the bare-stop rejection).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_video` row.
- `docs/architecture.md` + `README.md` — the §15.7 row count (57), the
  §16 video row, the parity-map M55 sentence, the status-table M55 row
  and the records range.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_video` — 3/3 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named video` ×5, `cannot find type
  VideoReply` ×2 — the m51 record's red shape), passed after the sugar
  landed.
- `cargo run -p guiwright-driver --example wire_client_video` — the demo
  prints the start ack, the double-start rejection
  (`a video recording is already running`), the stop's finalized
  artifact (`… (1 frames, 4 dropped, 0ms)`, `exists → true`) and the
  bare-stop rejection (`no video recording is running`).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (median ≈ 120 ms against the 500 ms design target, the cycle
  being heavyweight by design and host-dependent at the Auto encoder: an
  ffmpeg-present host pays the M3 probe + pipe spawn, a fallback host
  runs the deterministic PngSequence finalize in ~ms — the targets bound
  both orders, the guard still trips on order-of-magnitude regressions).

## 5. Remaining gaps (honest list)

- No convenience split (`video_start(opts)` / `video_stop()`): the TS
  client keeps both helpers on top of its raw wire calls and the REPL
  keeps the `video start` / `video stop` rows; the Rust seam stays at the
  vocabulary level until a consumer asks (the same scope rule the
  m48/m51/m52/m53 sugars applied).
- The REPL's `video` rows and the TS helpers are unchanged surfaces the
  sugar mirrors; the goldens and the schema are untouched byte-for-byte.
- The row's number bounds the Auto encoder's two orders (ffmpeg probe +
  spawn vs the PngSequence fallback); a dedicated per-path row would need
  an encoder override on the wire — out of scope here, the §10 `Auto`
  policy is the shipped contract.
