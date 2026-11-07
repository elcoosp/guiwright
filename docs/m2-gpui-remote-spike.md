# M2 spike — remote-mode GPUI

**Status:** spike (§17 M2 "remote-mode GPUI spike")
**Scope:** validate the *remote* contract GPUI will plug into, without
building the GPUI adapter (that is M3: "GPUI adapter (test-support + a11y
hybrid)").
**Evidence base:** `docs/architecture.md` §4.5 (GPUI dossier), §5.1
(process model), §5.3 (cargo integration), §7 (GWP), §17 (roadmap), plus
the M2 test `crates/guiwright-driver/tests/remote_session.rs::
clock_and_mock_domains_over_the_wire`.

---

## 1. What a GPUI remote session needs

Every GPUI fact from §4.5 that touches remote mode:

| Fact (§4.5) | Remote-mode consequence |
|---|---|
| Main-thread confinement (`Rc` entities) | The GPUI adapter must run on GPUI's foreground thread; the driver session thread talks to it at frame boundaries (§5.1 rendezvous) — same split every adapter uses, so **no protocol change** is required. |
| `test-support` feature | The app's feature graph must add `gpui/test-support` (§5.3 `gw-gpui = ["dep:guiwright-driver", "guiwright-driver/gpui", "gpui/test-support"]`). Remote mode adds nothing on top. |
| `dispatch_after(0)` waker | The adapter's `pump` hook is a GPUI foreground-executor task; GWP commands wake it via `dispatch_after(Duration::ZERO)` — the §5.1 waker table row. |
| `WindowingRequest::Headless` (Linux) + `PlatformHeadlessRenderer` | Headless CI (§11 GPUI row) works *without* pixels: the remote session only needs tree + action lanes. Pixel rows (`last_frame`) stay `false` in `Caps` for the headless path; `render_frame` turns true under `test-support`'s off-screen window. |
| A11y hybrid (`Window::a11y` + active flag) | `tree_update` requires forcing the accessibility activation at driver attach (§18 mitigation: "ActivationHandler with immediate activation for tests"). |

## 2. What M2 actually validated

The M2 GWP additions are the ones GPUI's remote mode consumes:

1. **Clock domain** (`RequestV1::Clock`, §7/§14) — GPUI's clock row is
   "⚠️ test-support dispatcher": embedded mode uses the seeded virtual
   scheduler, live/remote mode is best-effort. The wire vocabulary
   (`InstallVirtual`/`FastForward`/`RunFor`/`Pause`/`Resume`/`SetTime`/
   `State`) is adapter-agnostic; a GPUI adapter reports its precision in
   `Caps.clock` and implements `clock()` with the dispatcher mapping.
   The frame-sized stepping and the `clock.bin` ledger live in the
   driver's `ClockService` — GPUI inherits them unchanged.

2. **Mock domain** (`RequestV1::Mock`, §7/§14) — GPUI app state lives in
   `Entity` fields on the UI thread. The seam surface
   (`FrameworkAdapter::run_in`, §14 "state writes need the runner's
   context") hands a `&mut dyn Any` runner context to app-registered
   hooks; a GPUI adapter implements `run_in` by dispatching the task on
   the foreground executor with the app's root entity as context. The
   end-to-end UDS test drives exactly this flow (canned override →
   clear → real hook in the runner context).

3. **Handshake → resolve → act** — unchanged from M0/M1; the spike's
   test asserts the full sequence over a real UDS socket.

`clock_and_mock_domains_over_the_wire` (mock adapter, real socket) is
therefore the GPUI launch-mode contract: replace
`spawn_with_seams(.., || MockAdapter::.., ..)` with the GPUI adapter
constructor and the wire conversation stays byte-identical.

## 3. Launch-mode wiring (what the app adds at M3)

```toml
# app Cargo.toml (§5.3, GPUI row)
[features]
gw-gpui = ["dep:guiwright-driver", "guiwright-driver/gpui", "gpui/test-support"]
```

```ignore
// app main (remote mode, §5.3):
//   GUIWRIGHT_DRIVER=uds:///tmp/app.gwp ./app
#[cfg(feature = "gw-gpui")]
fn guiwright_boot(cx: &mut gpui::App) {
    if let Ok(url) = std::env::var("GUIWRIGHT_DRIVER") {
        let seams = guiwright::app_seams();          // #[seam] output
        guiwright_driver::server::GwpServerHandle::spawn_with_seams(
            uds_path_from(url),                       // parse uds://…
            || GpuiAdapter::attach(cx),               // M3
            seams,
        )
        .expect("guiwright remote mode");
    }
}
```

Open items recorded for M3 (none block the protocol):

- GPUI's `ActionRequest` synthesis to real mouse events (§8.4 GPUI row)
  needs a gesture-pacing table entry in §9.
- `FramePresented` hashes: GPUI has no scene digest — use the §15.5
  strided-pixel xxh3 via `test-support`'s `rendered_frame`.
- Adapter metadata: report the GPUI *platform* crate versions (§18
  "platform-crate churn" mitigation) in `AdapterMeta.framework_version`.

## 4. Conclusion

The remote contract (§7 domains incl. clock & mock) is adapter-agnostic
and now has a passing end-to-end test over a real socket. M3's GPUI work
is confined to the adapter implementation (`GpuiAdapter` +
`run_in`/`clock` mappings + a11y activation), not to GWP, the driver or
the kit.
