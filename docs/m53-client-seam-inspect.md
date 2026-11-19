# M53 — The client seam's inspect row (`ClientConn::inspect`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side
[`InspectReply`] / [`CodegenOutput`] carriers — the §13 inspector half as
typed sugar, the one-milestone-per-surface pattern (m46→…→m52) carried
to the inspect domain. A downstream Rust consumer starts and stops the
interaction → kit-code recorder, drives the overlay session and aims or
clears the hover outline with one call taking the wire's own `InspectV1`
enum — the same vocabulary the TypeScript client has spoken since M3
(`inspectOverlay()` / `inspectHighlight()`) and the REPL since M4/M8
(`codegen start` / `codegen stop`, `inspect` / `highlight`). No wire
change: the command rides the `Inspect` domain byte-for-byte as §13
blessed it; the schema, the golden vectors and the variant tables are
untouched. One budget row (`client_inspect`, the overlay-enable round
trip over the in-process door).

---

## 1. Why now

The §13 inspector story is the crate's authoring loop — record a
session's interactions, take the generated kit code, drive the overlay
while you explore — and two surfaces speak it over the wire: the REPL's
`codegen` / `inspect` rows and the TypeScript client's
`inspectOverlay()` / `inspectHighlight()`. The Rust seam — the mirror
every downstream implementer sees — stopped at the raw
`RequestV1::Inspect` arm, so a remote Rust suite wanting the recorder
hand-built the request and destructured `ResponseV1::Codegen` by field
(the exact shape `remote_session.rs`'s raw conversations have carried
since M4). The m48 rule — take the wire's own enum, no parallel
client-side vocabulary — was applied to the gesture (m48), clock (m51)
and mock (m52) surfaces; the inspect row is the same promise on the §13
half, leaving the raw `Input` batch and `Video` domain as the only
un-sugared arms (the §9 escape hatch by design, and the §10 recorder
whose downstream surface is the trace file, not a typed call).

## 2. Design: the §13 inspector, as sugar — and the union carrier

* **One method, the whole §13 vocabulary.** `inspect(cmd: InspectV1)`
  takes the wire's own enum (recording start / stop, overlay
  enable / disable, highlight aim / clear) — no parallel client-side
  vocabulary to keep in sync, the same rule the m48/m51/m52 sugars set.
* **The reply union is named, not collapsed.** The domain's replies are
  heterogeneous by wire design: the ack-only commands ride the shared
  `Pong` (the same frame `Ping` answers), and only the recording stop
  carries the `Codegen` payload. A bare `Option<CodegenOutput>` would
  hide which command answered what; `InspectReply { Applied, Codegen }`
  names the two arms client-side, the same honesty the carriers above
  draw. `CodegenOutput { code, warnings }` carries the `Codegen` payload
  as a named type — never serialized itself, so the schema stays
  byte-pinned (the `ActionReceipt` rule).
* **The honesty is pinned as shipped.** The highlight takes the
  dump-row handle form (a resolve row; `None` clears while the session
  stays on), a ghost handle answers the M8 boundary's typed
  `UnknownNode`, a second start while a recording is active and a stop
  with nothing recorded answer the recorder's typed rejection naming the
  cause (single-slot recorder, §13), and an adapter without an overlay
  surface answers the typed `Unsupported` (§6.2) — surfaced, never
  degraded.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::inspect` and the
  `InspectReply` / `CodegenOutput` types.
- `crates/guiwright-driver/tests/wire_client_inspect.rs` — the
  conversations: the overlay acks, the highlight aim/clear + the ghost
  rejection, the recorder end to end (start → the aim-and-act sugar →
  stop carrying the locator-naming code) + the idle-stop rejection, and
  the double-start rejection.
- `crates/guiwright-driver/examples/wire_client_inspect.rs` — the
  consumer example over the in-process door (overlay session, highlight
  aim/clear, the full recording loop printing the generated kit code,
  and the typed rejections).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_inspect` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_inspect` — 4/4 over
  the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named inspect`, unknown `InspectReply` — the
  m51 record's red shape), passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_inspect` — the
  demo prints the overlay/highlight acks, the recorder loop
  (`app.get_by_role(Role::Button, "Save").click();` generated from the
  acted surface), and the typed rejections (idle stop, ghost handle).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (median ≈ 21 µs against the 50 µs design target).

## 5. Remaining gaps (honest list)

- No split convenience methods (`codegen_start()` / `codegen_stop()` /
  `overlay(on)`): the TS client keeps `inspectOverlay()` /
  `inspectHighlight()` on top of its raw inspect surface and the REPL
  keeps the `codegen` / `inspect` rows; the Rust seam stays at the
  vocabulary level until a consumer asks (the same scope rule the
  m48/m51/m52 sugars applied).
- The `Video` domain stays raw on the seam (the §10 recorder's
  downstream surface is the finalized trace file, and its start/stop
  answers one variant — the sugar's union would carry one arm; the m52
  scope rule again).
- The REPL's `inspect`/`codegen` rows and the TS helpers are unchanged
  surfaces the sugar mirrors; the goldens and the schema are untouched
  byte-for-byte.
