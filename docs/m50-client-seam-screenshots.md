# M50 — The client seam's screenshot rows (`ClientConn::screenshot_window` / `screenshot_element`)

**Status:** implemented. Two default methods on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) plus the client-side [`Shot`]
carrier — the m13-style read half of the M10 wire surface, closed in the
one-milestone-per-surface pattern m46/m47/m48/m49 established. A
downstream Rust consumer pulls the whole window frame — or the clip-aware
crop of one resolved element — as typed PNG evidence answering the §10
capture rule (one render, never a re-render), the same vocabulary the
TypeScript client has spoken since M9 (`screenshotWindow()` /
`screenshotElement()`) and the REPL since M10 (`screenshot window` /
`screenshot element`). No wire change: both ride the `Screenshot` domain
byte-for-byte as M10 blessed them; the schema, the golden vectors and the
variant tables are untouched. One budget row (`client_screenshot`, the
resolve + render + crop over the in-process door).

---

## 1. Why now

The §10 capture contract has promised "crop from the window frame by the
node's clip-aware rect — never re-render" since M9, and the shot is the
wire's visual-evidence read — the "CI container fetches a screenshot from
a remote app" flow. The REPL has printed the rows since M10 and the
TypeScript client has carried `screenshotWindow()` / `screenshotElement()`
since M9; the Rust seam — the mirror every downstream implementer sees —
stopped at the raw `RequestV1::Screenshot` arm, so a consumer wanting PNG
evidence hand-built the request and destructured `ResponseV1::Shot` by
variant (the exact shape
`crates/guiwright-driver/examples/wire_screenshot.rs` has carried since
M10). The m49 rule — the dump-row promise made typable — was applied to
the tree reads; the screenshot rows are the same promise on the
render-heavy half.

## 2. Design: the §10 capture contract, as a carrier

* **Two shots, one carrier.** `screenshot_window()` is the full frame
  (one §10 render pass, the headless on-demand path — the same path the
  kit's window-level `App::screenshot` uses); `screenshot_element(selector,
  timeout_ms)` is the M9 element arm: the driver's auto-wait resolve
  inside the budget (the connection's window scope narrowing the match —
  the same resolve [`ClientConn::resolve`] and `tree_dump_under` run),
  then the crop by the resolved node's clip-aware physical bounds. Both
  rebuild the wire reply into [`Shot`] `{ frame_seq, width, height, png }`,
  field-for-field the M10 payload — the frame the pixels came from (the
  seq space clients can correlate an action receipt against), the crop's
  pixel dimensions and the PNG bytes.
* **The carrier, not the variant.** `Shot` is a client-side type mirroring
  the reply's fields — never serialized itself, so the schema stays
  byte-pinned (the `ActionReceipt` rule). The PNG rides the response frame
  directly (already losslessly compressed; the §15.6 LZ4 blob stage would
  double-encode without shrinking it) — the carrier changes nothing about
  that transport decision.
* **The resolve's honesty rides the filtered arm.** A locator that never
  matches inside the budget answers the typed `Timeout` naming the stuck
  check (`attached` — the Element arm's own auto-wait); an ambiguous one
  answers `StrictModeViolation` (§8.2 strict mode is the wire's own
  policy) — pinned by conversations, not re-mapped. A node without bounds
  (laid out virtually) answers the honest error the M9 wording records:
  nothing to crop, not an empty crop. Adapters without a pixel path
  answer the typed `Unsupported` (§6.2 capability row) — the sugar
  surfaces that rejection rather than degrading.
* **No wire change.** Both rides are byte-for-byte the M9/M10 domains;
  the golden vectors and the discriminant tables are untouched.

## 3. Surface

- `crates/guiwright-proto/src/client.rs` —
  `ClientConn::screenshot_window`, `ClientConn::screenshot_element` and
  the `Shot` type.
- `crates/guiwright-driver/tests/wire_client_screenshot.rs` — the
  conversations: the window shot over the scripted 480×240 frame (the
  carrier naming frame + dimensions, the PNG magic proving a real encode),
  the Save button's 120×32 element crop, the missing locator's typed
  `Timeout` inside the budget, and the ambiguous locator's typed
  `StrictModeViolation`.
- `crates/guiwright-driver/examples/wire_client_screenshot.rs` — the
  consumer example over the in-process door (the window shot, the element
  crop written to disk, the typed timeout).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_screenshot` row.

## 4. Tests

- `cargo test -p guiwright-driver --test wire_client_screenshot` — 4/4
  over the in-process door.
- Red→green: the conversation tests failed to compile against the
  unfixed seam (`no method named screenshot_window`,
  `screenshot_element` ×3 — the m35 record's red shape), passed after
  the sugar landed.
- `cargo run -p guiwright-driver --example wire_client_screenshot` —
  the demo writes `window.png` (480×240, frame 4) and
  `save-button.png` (120×32, frame 7), then prints the typed timeout line.
- `cargo test -p guiwright-bench --lib` — the budgets parse with the
  new row bound (the copies-match check green); `check_budgets` reports
  the row OK (median ≈ 181 µs against the 400 µs design target).

## 5. Remaining gaps (honest list)

- The raw `wire_screenshot.rs` example stays: it is the M10-era record of
  the raw request shape, and the m48 record's scope rule keeps the raw
  domain visible next to the sugar (the raw `RequestV1::Screenshot` arm
  remains the byte-pinned surface; the sugar rides it).
- No visual-diff over the wire: the `Shot` carrier ships PNG bytes, and
  the perceptual baseline machinery (YIQ diff, bless flow) stays kit-side
  where §10 put it — a remote diffing client composes from the bytes
  exactly like the TypeScript one does.
- No TypeScript change: `screenshotWindow()` / `screenshotElement()` are
  the M9 surfaces the sugar mirrors; the goldens and the schema are
  untouched byte-for-byte.
