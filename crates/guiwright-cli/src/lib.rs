//! Guiwright CLI (architecture §5.2 `guiwright-cli` + §16 v0.4 "REPL /
//! debugging"): the interactive debugging loop over the GWP wire protocol.
//!
//! * [`repl`] — `guiwright repl`: attaches to a live remote-mode session
//!   (UDS or TCP) and drives it with Playwright-shaped commands —
//!   locators, actions, the virtual clock, state seams, codegen and video
//!   (§13 "a REPL attaches to a live session for ad-hoc locators/eval").
//! * [`show_trace`] — `guiwright show-trace`: renders a `gwtrace/v1` zip
//!   into the viewer's self-contained `trace.html` (§12 deep links).
//! * [`bless`] — `guiwright bless`: promotes `.new.png` screenshot
//!   artifacts onto their baselines (the §10 `GUIWRIGHT_BLESS` flow as an
//!   explicit command, for CI-artifact blessing outside a test run).
//! * [`deps`] — `guiwright install-deps`: prints the §11 CI provisioning
//!   commands for the host OS.
//!
//! Every REPL command maps onto an existing GWP request — the protocol is
//! unchanged (the M2 conclusion: the wire contract is adapter-agnostic,
//! so the same REPL attaches to every adapter, GPUI included).

#![forbid(unsafe_code)]

pub mod args;
pub mod bless;
pub mod deps;
pub mod repl;
pub mod session;
pub mod show_trace;

pub use args::{CliError, Command, help_text, parse_args};
pub use bless::{BlessReport, bless_dir};
pub use deps::{install_deps_command, platform_deps};
pub use repl::{Line, Repl, StepOutcome};
pub use session::{ConnectTarget, Connection};
pub use show_trace::show_trace;

/// The agent string sent in the GWP handshake (§7 `Hello.agent`).
pub const AGENT: &str = concat!("guiwright-cli/", env!("CARGO_PKG_VERSION"));
