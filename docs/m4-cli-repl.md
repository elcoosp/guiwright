# M4 — the Guiwright CLI & REPL (§5.2, §11, §12, §13, §16)

**Status:** implemented (this milestone ships `crates/guiwright-cli`).
**Scope:** §16 v0.4 "REPL / debugging — `guiwright repl` over GWP" plus the
§5.2 `guiwright-cli` crate row (`show-trace`, `bless`, `install-deps`) and
the §13 interactive session's missing half: **remote codegen recording**.
**Evidence base:** `docs/architecture.md` §5.2 (crate table), §7 (GWP),
§11 (`install-deps`), §12 (`show-trace` deep links), §13 ("a REPL
(`guiwright repl`) attaches to a live session for ad-hoc locators/eval"),
§16 (parity map v0.4 row); the M2 conclusion (`docs/m2-gpui-remote-spike.md`)
"no protocol change is required" carries over unchanged.

---

## 1. Why the CLI is the next milestone

After M3 every §17 roadmap row through M3 is implemented (floem + gpui
engines, video, the TypeScript client), but the workspace is still missing
one crate the architecture names: `guiwright-cli`
(`guiwright codegen|test|show-trace|bless|install-deps`, §5.2). The
wire-level pieces it needs all exist since M2/M3 — GWP over UDS **and**
TCP, the clock/mock domains, the video domain, the inspect domain — but
the two *interactive* entry points they were built for are absent:

| Architecture hook | What it names | Status before M4 |
|---|---|---|
| §5.2 crate table | `guiwright-cli` | crate did not exist |
| §11 | "Guiwright CLI `install-deps` provisions CI" | no CLI |
| §12 | "`guiwright show-trace …` deep-links from assertion errors" | `App::export_trace_viewer` emitted HTML, nothing opened it |
| §13 | "a REPL (`guiwright repl`) attaches to a live session" | no REPL |
| §16 | "REPL / debugging — `guiwright repl` over GWP — v0.4" | open |

`guiwright test` (the §5.2 nextest/report glue, §16 "v0.5 own runner")
and multi-window targets (§16 v0.4) stay deferred — see §6.

## 2. What ships

New workspace member `crates/guiwright-cli` — a lib (the testable core)
plus the `guiwright` binary. Dependencies stay in the workspace's
established set (proto, viewer, driver as a *dev*-dep for tests); no new
external crates.

### 2.1 `guiwright repl` — the live session debugger (§13)

`guiwright repl --uds /tmp/app.gwp` or `--tcp 127.0.0.1:41234` attaches to
a running remote-mode app, performs the §7 handshake
(`guiwright-cli/<semver>` agent), and reads commands from stdin — one per
line, `#` comments, blank lines ignored, `exit`/`quit`/`bye` closes the
session cleanly. The command core is a pure `Repl` structure
(`step(line) -> Vec<Line>`); the binary is a thin stdin/stdout loop
(prompt on stderr only for TTYs), so scripted use
(`printf 'snapshot\nbye\n' | guiwright repl --uds …`) and tests share one
code path.

Command surface — every command maps 1:1 onto an existing GWP request;
**the protocol is unchanged** (the M2 conclusion holds):

| Command | Wire request | Notes |
|---|---|---|
| `ping` | `Ping` | liveness/latency |
| `snapshot` | `TreeSnapshot` | node count + tree seq |
| `get role <Role> [name]` / `get text <T>` / `get label <T>` / `get testid <Id>` / `get value <T>` | `Resolve` | auto-waits `timeout` ms; stores the node handle + generation as the REPL's "current node" (like a shell's `cd` for the tree) |
| `click` `dblclick` `hover` `focus` `blur` `check` `uncheck` | `Action` | applied to the current node at a frame boundary |
| `fill <Text>` / `setvalue <Text>` | `Action(SetValue)` | |
| `press <Key> [Mods…]` | `Action(Press)` | e.g. `press Enter`, `press A Shift` |
| `scroll <Dx> <Dy>` | `Action(Scroll)` | |
| `clock install` / `ff <ms>` / `runfor <ms>` / `pause` / `resume` / `set <ms>` / `state` | `Clock` | ms on the wire (§14) |
| `mock install <Id> [Response]` / `clear <Id>` / `call <Id> [Scope]` / `reset <Scope>` | `Mock` | state seams (§14) |
| `codegen start` / `codegen stop` | `Inspect` | §13 recording — see §3 |
| `video start [fps] [dir]` / `video stop` | `Video` | §10 |
| `last` | — | shows the current node handle |
| `timeout <ms>` | — | resolve/action deadline (default 5000) |
| `help` | — | command reference |

Rejections (strict-mode violations, timeouts, stale generations) print as
`error: <code>: <message>` lines and never panic — the REPL is a
debugging tool, so a failed command must not lose the session.

### 2.2 Remote codegen (§13, closing the Inspect domain)

Before M4, `Inspect(StartRecording|StopRecording)` over the wire answered
`Unsupported` ("remote codegen recording lands with the interactive
inspector session"). The REPL **is** that session, so M4 wires the
driver-side recorder:

- `Session` gains an optional `CodegenRecorder` (guiwright-inspect;
  driver depends on inspect — no cycle, inspect depends only on core).
  `codegen_start()` / `codegen_stop() -> (code, warnings)` manage it.
- Every `Action` request applied while recording emits one kit-code line:
  facts are collected from the session's `TreeService` exactly like the
  embedded kit tap (`role`, accessible name, label, value, test id —
  `App::collect_facts` is the precedent), and the §13 cost model picks
  the minimal stable selector (`TestId` > role+name > label > value >
  text) with live strict-mode narrowing (`.first()` + warning).
- Action → kit-method mapping follows the kit's locator vocabulary:
  `Click → click`, `DoubleClick → dblclick`, `SetValue → fill` (with the
  text as the recorder argument), `Hover → hover`, `Press → press` (the
  recorder learned argument support for `press`, the same shape
  `fill` already had), `Check/Uncheck/Focus/Blur → check/uncheck/focus/blur`.
  `Scroll` is not recorded — it is a raw-input gesture (§9), not a
  locator action; recorded in the gesture milestone.
- `codegen stop` replies `Codegen { code, warnings }` (the §7 shape,
  already in the schema — **no wire change**).

### 2.3 `guiwright show-trace` (§12)

`guiwright show-trace <archive.zip> [--out <path>]` reads a `gwtrace/v1`
zip via `guiwright-viewer` and writes the self-contained `trace.html`
(default: next to the archive, `<stem>.html`). `--open` hands the file to
the platform opener (`xdg-open`/`open`/`start`) — best-effort, a missing
opener degrades to printing the path. This is the deep-link target the
assertion-error messages name.

### 2.4 `guiwright bless` (§10 bless flow as a CLI)

The kit's `GUIWRIGHT_BLESS=1` flow leaves `.new.png`/`.diff.png`
artifacts next to failing baselines. In CI the artifacts are uploaded,
and a human blesses afterwards — on a machine where no test runs. The
command closes that loop:

```
guiwright bless <baseline-dir> [--dry-run]
```

For every `<name>/<os>-<arch>-<renderer>.png.new.png` under the dir it
promotes the artifact to the baseline path (replacing the stale
baseline), removes the sibling `.diff.png`, and prints a per-file report.
`--dry-run` lists the promotions without touching anything. The layout is
the §10 store shape (`BaselineStore::baseline_path`), matched
layout-based so the CLI stays independent of the shot crate's image
stack.

### 2.5 `guiwright install-deps` (§11)

Prints the §11 CI provisioning command for the host OS
(`apt-get install libwayland-client0 libxkbcommon0 mesa-vulkan-drivers
libvulkan1 vulkan-tools` on Debian/Ubuntu — the lavapipe software-Vulkan
route the CI matrix uses). `--run` executes it (never the default:
tests and docs never shell out). Unknown platforms exit with a clean
error listing the §11 table instead of guessing.

## 3. Testing posture

The M2/M3 pattern repeats: the framework-free mock adapter + a real
socket is the contract test (§ "the GPUI launch-mode contract" — replace
the adapter constructor, keep the conversation). New integration tests in
`crates/guiwright-cli/tests/`:

- `repl_e2e.rs` — a full REPL conversation over a real UDS socket
  (handshake → snapshot → get → click → fill → clock → mock → codegen →
  bye) and the same conversation over TCP, asserting transport parity;
  strict-mode violation, timeout and stale-generation rejections surface
  as error lines without killing the session.
- `show_trace.rs` — record a `gwtrace/v1` archive with the trace
  recorder (the viewer e2e's helper pattern), run `show-trace`, assert
  the emitted HTML contains the timeline and tree payloads.
- `bless.rs` — a synthetic baseline dir with stale baselines + artifacts;
  dry-run lists, real run promotes and cleans diffs.
- `deps.rs` — the emitted provisioning command names the §11 packages;
  `--run` is rejected outside an explicit opt-in path.

## 4. Deferred (recorded)

- `guiwright test` — nextest/report glue and the §16 "v0.5 own runner"
  row. The kit + nextest already cover the MVP runner story.
- `guiwright codegen <app-cmd>` app-launching wrapper — launching the
  app binary is the app's own feature graph (`gw-*` features, §5.3); the
  REPL covers the attach + record half, the wrapper lands with the
  overlay session (§13 highlight handles).
- Multi-window / tabs (§16 v0.4) — needs per-window targeting on
  `NodeKeyV1`/`Resolve`/`Action` plus multi-window harnesses; a protocol
  revision (`V1` append-only discipline applies), so it ships as its own
  milestone with its own spike doc.
- Live trace-viewer streaming (§12 "live mode") — WS attach to a running
  session; the viewer's static emission is the M4 baseline.

## 5. Conclusion

M4 turns the M2/M3 wire surface into the interactive debugging loop the
architecture promised: a developer attaches to any remote-mode app —
including the future GPUI adapter, whose launch contract is byte-identical
(M2) — resolves locators, steps the virtual clock, mocks state seams,
records code, captures video and opens traces, all through one binary
with no framework dependencies.
