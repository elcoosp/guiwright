//! Guiwright [`FrameworkAdapter`] engine for GPUI (architecture §4.5, M3).
//!
//! GPUI ranks 7th in the §17 maturity table — the highest ceiling
//! (Zed-scale apps) behind the heaviest constraints: platform-crate churn,
//! main-thread confinement (`Rc` entities, `ForegroundExecutor: !Send`)
//! and test-support-gated capture. The M2 spike (`docs/m2-gpui-remote-
//! spike.md`) validated the *remote* contract; M3 adds the adapter engine.
//!
//! GPUI is a **hybrid** (§4.5): Level-1 queries (unconditional AccessKit,
//! per-frame `TreeUpdate`s with node bounds) plus Level-2 input
//! (`Window::dispatch_event(PlatformInput)` is the canonical seam —
//! synthetic events are bit-identical to OS input):
//!
//! * [`a11y`] — the a11y-hybrid activation model: force-enable at driver
//!   attach (§18 "`ActivationHandler` with immediate activation for tests"),
//!   then the per-frame `TreeUpdate`s flow unchanged.
//! * [`input`] — [`InputEvent`](guiwright_driver::adapter::InputEvent) →
//!   `PlatformInput` steps, including the `VisualTestContext` keystroke
//!   vocabulary (`"cmd-shift-p enter"` → modifier+key sequence).
//! * [`clock`] — the test-support dispatcher plan (§6.2 GPUI row "⚠️
//!   test-support dispatcher"): install swaps to the seeded virtual
//!   scheduler; steps are `dispatch_after(Duration::ZERO)` rounds — the
//!   §5.1 waker row.
//! * [`run_in`] — the state-seam contract (§14): the task dispatches on
//!   the foreground executor with the app's root entity as the runner
//!   context.
//! * [`frames`] — the §15.5 strided-pixel xxh3 digest over
//!   `test-support`'s `rendered_frame` (§4.5 "no scene digest — use the
//!   §15.5 strided-pixel xxh3").
//! * [`contract`] — the downstream app glue contract. This crate ships
//!   **framework-free** (the `guiwright-adapter-slint` precedent, §18):
//!   gpui 0.2.2 is on crates.io, but its Linux platform stack needs
//!   system libraries (xkbcommon, wayland) a default build must never
//!   require. The app-side wiring is fully specified in the contract and
//!   the M2 spike doc §3 (`gw-gpui = ["dep:guiwright-driver", …]`).
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter

#![forbid(unsafe_code)]

pub mod a11y;
pub mod clock;
pub mod contract;
pub mod frames;
pub mod input;
pub mod run_in;

pub use a11y::{A11yHybrid, ActivationPolicy};
pub use contract::{GpuiAdapterContract, GpuiCaps};
pub use frames::{FrameDigest, strided_xxh3};
pub use input::{GpuiButton, GpuiInputStep, KeyStroke, Modifier, parse_keystrokes, translate};
pub use run_in::{Dispatch, RunInPlan};
