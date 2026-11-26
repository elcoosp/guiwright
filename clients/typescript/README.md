# @guiwright/gwp

The **Guiwright Protocol (GWP) client for TypeScript/JavaScript** — the
architecture §16 "other-language client" (M3). Zero dependencies; the
wire contract is the vendored [`gwp-schema.json`](./gwp-schema.json)
(regenerated from `guiwright-proto::schemadoc`, golden-pinned both here
and in Rust).

```ts
import { GwpClient } from "@guiwright/gwp";

const client = await GwpClient.connectTcp("127.0.0.1:43210");
const welcome = await client.handshake("my-agent/1.0");

const save = await client.resolve({
  kind: "role",
  role: "Button",
  name: { text: "Save", exact: false, ignore_case: true },
});
await client.action(save.node, save.generation, { kind: "click" });

await client.clock({ kind: "fast_forward", ms: 320 });
await client.callSeam("seed_users", "smoke");
await client.videoStart({ fps: 25, dir: "test-results/videos" });
const video = await client.videoStop();

await client.close();
```

## Wire contract (§7)

- **Framing** — 4-byte little-endian body length + postcard-encoded body.
- **Codec** — postcard rules are in `src/postcard.ts` (LEB128 varints,
  1-byte bools/options, varint-length strings/seqs, raw-LE f64s, enum
  variants = varint discriminant). The variant tables live in
  `src/schema.ts` and are pinned by tests against `gwp-schema.json`.
- **Transports** — TCP (`connectTcp`, the §7 CI-container door) and
  WebSocket (`connectWs`, the viewer live-mode door — the Rust side
  serves it since M15 via `GwpServerHandle::spawn_ws` / the demo app's
  `--ws` flag).
- **Conversation** — `handshake()` first; one request → one response;
  `ResponseV1::Error` rejects as a typed `GwpRejected` (code names in
  `ERROR_NAMES`); `bye` closes cleanly.

## Domain surface

| Domain | Methods | Architecture |
|---|---|---|
| Tree | `treeSnapshot()` | §8.2 |
| Locators | `resolve(selector, timeoutMs)` — server-side auto-wait | §8.2 |
| Actions | `action(node, generation, action)` | §8.4 |
| Clock | `clock(cmd)`, `fastForward(ms)` | §14 |
| Seams | `mock(cmd)`, `callSeam(id, scope)`, `resetState(scope)` | §14 |
| Video | `videoStart(opts)`, `videoStop()` | §10 |
| Inspector | `inspect` codec support | §13 |

The full domain vocabulary on `GwpClient` (each maps 1:1 onto a GWP request):

| Domain | Methods |
|---|---|
| Windows (M6) | `windowList()`, `windowUse(handle)` |
| Gestures / raw input (M7) | `mouse`, `keyboard`, `input` |
| Inspector overlay (M8) | `inspectOverlay`, `inspectHighlight` |
| Screenshots (M10) | `screenshotWindow()`, `screenshotElement(...)` |
| Tree dumps (M11/M13) | `treeDump()`, `treeDumpUnder(selector, timeoutMs?)` |
| Event channel (M12/M14) | `subscribe(kinds)`, `nextEvent()`, `unsubscribe()` |
| Set read (M30) | `resolveAll(selector)` — identities + rects, document order, no wait |
| Point read (M32) | `hitTest(x, y)` — the node under the point, `null` on a miss |
| Node facts (M36) | `describe(node, generation)` — one handle's identity facts (role, name, value, bounds) |
| Set facts (M39) | `describeAll(selector)` — the resolve-all walk's rows carrying the describe facts, one round trip |
| Bulk reads (M41) | `allTextContents(selector)` — label first (the text locators' source order); `allValues(selector)` — value first (the form's source order); both ride `describeAll`, no protocol change |
| Client-side wait (M45) | `waitFor(selector, state, timeoutMs?)` — the m42 composition over the TS client (subscribe / `describeAll()` / `nextEvent`), `attached`/`visible`/`hidden`/`stable`, the typed `GwpWaitTimeout` naming state, locator and last evaluation; no protocol change |
| Content assertions (M54) | `expectText(selector, expected, timeoutMs?)` / `expectValue(selector, expected, timeoutMs?)` — the kit's `toHaveText` / `toHaveValue` over any door: the M41 source orders made waitable on the M45 wake loop, the typed `GwpWaitTimeout` naming locator, expected and found; no protocol change |
| Count assertions (M60) | `expectCount(selector, expected, timeoutMs?)`, `expectCountGreaterThan(selector, min, timeoutMs?)`, `expectCountLessThan(selector, max, timeoutMs?)` — the kit's `toHaveCount` family over any door (m59's Rust rows mirrored on the identical `CountTracker` evaluator, `src/count.ts`): the M30 set read on the M54 wake loop, absence answering found 0 (Playwright parity), the typed `GwpWaitTimeout` naming the comparison and the count the last evaluation found; no protocol change |
| Identity assertions (M63) | `expectRole(selector, role, timeoutMs?)`, `expectName(selector, expected, timeoutMs?)` — the kit's `toHaveRole` / `toHaveAccessibleName` family over any door (m62's Rust rows mirrored on the same `ExpectTracker` evaluator, `src/expect.ts`): the role compared exactly (a structural fact — no matcher, no fuzz), the name read label-only (the value never answers), a missing name staying pending with the missing fact named, the M39 set read on the M54 wake loop, the typed `GwpWaitTimeout` naming `have role` / `have name` and both sides; no protocol change. Runnable demo: `npm run example:identity` |
| Description + set identity (M64/M65) | `describeAllIdentity(selector)` — the appended describe-all identity rows for the whole set (role, name, value, description per match); `expectDescription(selector, expected, timeoutMs?)` — the kit's `toHaveAccessibleDescription` over any door (the m65 `expectIdentityContent()` composition): the description-only read (the name and the value never answer), a missing description staying pending until the node's restatement, the satisfied answer handing back the full identity row; no protocol change beyond the appended identity domain. Runnable demo: `npm run example:description` |
| Node identity + state (M66–M69) | `describeIdentity(node, generation)` — one handle's full identity row at handle width (the Action boundary's generation guard); `describeIdentityState(node, generation)` — the handle's state facts (the checked fact the `CheckedState` four-way word); the m66/m68 Rust rows mirrored row-for-row (the m45/m60/m63/m65 pattern); no protocol change. Runnable demos: `npm run example:describe-identity`, `npm run example:identity-state` |
| Set state facts (M70/M71) | `describeAllIdentityState(selector)` — the resolve-all walk's rows carrying the state facts per match, the m70 Rust row mirrored; no protocol change beyond the appended domain. Runnable demo: `npm run example:identity-state-all` |
| State assertions (M75) | `expectChecked(selector, expected, timeoutMs?)`, `expectEnabled(selector, expected, timeoutMs?)`, `expectFocused(selector, timeoutMs?)` — the kit's `toBeChecked` / `toBeEnabled` / `toBeFocused` over any door (m73's Rust rows mirrored on the shared `expectState()` composition, the `StateTracker` evaluator in `src/expect.ts`): the tri-state demanded exactly, focus taking no expectation word (the m73 no-negation rule), the typed `GwpWaitTimeout` naming the state word and the fact the last evaluation found; no wire change. Runnable demo: `npm run example:state-assertions` |
| Count comparators, widened (m25's honest-list widening) | `expectCountAtLeast(selector, min, timeoutMs?)`, `expectCountAtMost(selector, max, timeoutMs?)`, `expectCountBetween(selector, min, max, timeoutMs?)` — the inclusive pair and the range shape the m25 record reserved, the Rust `expect_count_at_least` / `expect_count_at_most` / `expect_count_between` mirrored on the same `expectCountWith()` composition; no protocol change |
| Appended actions (m89/m90's named per-surface follow-on) | `selectOption(selector, option, timeoutMs?, force?)` — Playwright's `locator.selectOption`: the option by accessible name over the appended `SemActionV1::SelectOption` row; `scrollIntoViewIfNeeded(selector, timeoutMs?, force?)` — the a11y-first scroll request over `SemActionV1::ScrollIntoView`; both resolve-then-act through the M47 aim-and-act split (the strict auto-waited `resolve()`, the actionability gate, the `force`/`timeout_ms` opt-outs ride unchanged). Runnable demo: `npm run example:appended-actions` |
| Inspector recording (§13) | `inspectStartRecording()` / `inspectStopRecording()` — every resolution after the start is recorded against its minimal stable selector; the stop answers the emitted kit code plus its warnings |
| Selector helpers (m92) | `filter(source, opts)` (`src/filter.ts`) — the typed `hasText` / `hasNot` / `has` / `nth` opts composing the m34/m91 filter rows byte-identically for every typed method above (a `Selector` in, a narrowed `Selector` out — no locator object). Runnable demo: `npm run example:filter` |
| Regex text rows (m95) | `textRegex` / `labelRegex` / `valueRegex` / `roleNameRegex` / `hasTextRegex` (`src/filter.ts`) — the appended `SelectorV1` rows the m34 standing note opened, a `RegExp` folding the wire-readable flags inline (`i`/`m`/`s` → the pattern's own form; `g`/`y`/`d`/`u` are the typed refusal) and a string riding verbatim; `filter({ hasText: /re/ })` rides the appended regex row beside the string `Filter` form. Byte-pinned against the Rust golden vectors (`test/regex.test.ts`). Runnable demo: `npm run example:regex` |
| Liveness / misc | `ping()`, `gesture(...)` (the `mouse`/`keyboard` helper's request), `close()` |

## Tests

```sh
npm test   # codec golden vectors (captured from Rust) + client over TCP
npm run e2e  # spawns the Rust demo app (cargo) and drives it over real TCP
```

The golden vectors (`test/golden.json`) are blessed from Rust with
`GUIWRIGHT_BLESS=1 cargo test -p guiwright-proto --test golden_vectors`;
`test/codec.test.ts` fails whenever the vendored tables or the wire bytes
drift.
