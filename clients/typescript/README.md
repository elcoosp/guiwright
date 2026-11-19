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
