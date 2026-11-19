# M58 — The client seam's bye row (`ClientConn::bye`)

**Status:** implemented. One default method on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) — the §7 lifecycle close as
typed sugar, the one-milestone-per-surface pattern (m46→…→m57) carried to
the farewell domain. A downstream Rust consumer says goodbye with one
call and receives the wire's own answer as `Ok(())`, instead of
hand-building the `RequestV1::Bye` arm and matching the reply. The same
round trip the TypeScript client's `close()` has made since its M3
landing before its own transport teardown. No wire change: the farewell
rides the `Bye` domain byte-for-byte as M0 blessed it; the schema, the
golden vectors and the variant tables are untouched. One budget row
(`client_bye`, the farewell round trip over the in-process door).

---

## 1. Why now

The seam's lifecycle has typed ends on the *opening* side — `handshake`
validates the driver's `Welcome` and answers the adapter's caps — while
the *closing* side stayed raw: the wire's oldest courtesy arm (`Bye`,
request discriminant 9, reply discriminant 10, blessed at M0) had no
sugar, so a Rust consumer said goodbye by hand and a downstream
implementer reading the trait had no row naming the lifecycle's end.
Every other surface speaks it: the TypeScript client's `close()` does
exactly this round trip before its own `transport.close()`, and the
driver's dispatch arm has answered `Bye` with `Bye` since M0 (the
`RequestV1::Bye => ResponseV1::Bye` one-liner). The m51 rule — take the
wire's own vocabulary — applies with nothing to even map: the sugar is
the arm, typed.

## 2. Design: the farewell, as sugar

* **One method, the wire's own arm.** `bye()` — no arguments, no
  carrier: the arm answers `Bye` with `Bye`, so the sugar answers
  `Ok(())`. A carrier would name facts the arm does not carry; the
  honest shape is the unit answer (the `Ok(())` IS the receipt).
* **The conversation's end is the wire's own contract.** `run_connection`
  returns right after writing the `Bye` frame — a further request on the
  farewelled connection cannot land. The doc says so, and the wire test
  pins it: the post-bye read errs on the spent connection. This is the
  mechanism the TS `close()` already knows: await the bye, then close
  the transport.
* **The session is not the conversation.** What the farewell does not
  end is the session under the connection: the wire test connects a
  successor and reads the same 8-node tree — the driver kept serving,
  untouched by the goodbye. The transport close stays the door's
  business (socket drop, the in-process channels), exactly where §7
  draws the line.
* **No wire change and no new capability flags.** The farewell rides the
  `Bye` domain byte-for-byte; the dispatch arm is the M0 one-liner,
  untouched; every pre-existing door answer surfaces verbatim.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` — `ClientConn::bye`.
- `crates/guiwright-driver/tests/wire_client_bye.rs` — the
  conversations: the farewell answering `Ok(())`, and the farewell
  ending the conversation (the spent connection's read errs) while the
  session keeps serving (the successor connection's read lands).
- `crates/guiwright-driver/examples/wire_client_bye.rs` — the consumer
  example over the in-process door (the farewell's answer, the spent
  door's honest error, the successor's summary read).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_bye` row.
- `docs/architecture.md` + `README.md` — the §15.7 row count (60), the
  parity-map M58 sentence, the status-table M58 row and the records
  range.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_bye` — 2/2 over the
  in-process door.
- Red→green: the first conversation test failed against the unfixed
  seam (`no method named bye` — the m51/m55/m56/m57 record's red shape),
  and the first *passing* draft caught its own wrong assumption: a test
  asserting the connection keeps serving after the bye failed with
  `Io(Kind(BrokenPipe))` — the wire's own contract teaching the test
  that `run_connection` ends the conversation on the `Bye` arm. The test
  was rewritten to pin that contract (the conversation ends, the session
  keeps serving) and passed after the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_bye` — the demo
  prints the farewell's answer, the spent door's honest error line and
  the successor's summary read (`tree_seq 1 at 8 nodes (the session kept
  serving)`).
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (each measured iteration opens a fresh connection — the open is
  setup, the farewell round trip is the measured operation).

## 5. Remaining gaps (honest list)

- No `close()` on the trait: the transport half is each door's own
  teardown path (drop / your client's close), the same line §7 draws —
  the sugar names the wire's half of the goodbye and refuses to
  smuggle a transport API into the conversation trait.
- No session-level reset semantics: the wire's `Bye` is a per-connection
  courtesy, not a session shutdown; a host that owns the session tears
  it down through its own handle (`GwpServerHandle::shutdown`), the
  same boundary the kit's embedded mode already rides.
- The TS client is an unchanged surface the sugar mirrors (`close()`
  predates the row); the goldens and the schema are untouched
  byte-for-byte.
