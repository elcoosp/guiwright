//! Guiwright CLI (architecture §5.2 `guiwright-cli`): the interactive
//! debugging loop (§16 v0.4) and the own runner (§16 v0.5) over the GWP
//! wire protocol.
//!
//! * [`repl`] — `guiwright repl`: attaches to a live remote-mode session
//!   (UDS, TCP or WebSocket) and drives it with Playwright-shaped commands —
//!   locators, actions, the virtual clock, state seams, codegen and video
//!   (§13 "a REPL attaches to a live session for ad-hoc locators/eval").
//! * [`show_trace`] — `guiwright show-trace`: renders a `gwtrace/v1` zip
//!   into the viewer's self-contained `trace.html` (§12 deep links).
//! * [`bless`] — `guiwright bless`: promotes `.new.png` screenshot
//!   artifacts onto their baselines (the §10 `GUIWRIGHT_BLESS` flow as an
//!   explicit command, for CI-artifact blessing outside a test run).
//! * [`deps`] — `guiwright install-deps`: prints the §11 CI provisioning
//!   commands for the host OS.
//! * [`runner`] — `guiwright test`: the own runner (M5, §16 v0.5) —
//!   discovers `gwtest/v1` scenario specs and drives them over GWP with
//!   workers, retries, sharding and json/junit/html reports (§5.2
//!   "nextest/report glue, sharding", §11 "multiple apps per host").
//! * [`spec`] — the `gwtest/v1` spec format: scenarios whose steps are
//!   strings in the [`repl`] command language, so a session rehearsed in
//!   the REPL pastes into a spec verbatim.
//!
//! Every REPL command maps onto an existing GWP request — the protocol is
//! unchanged (the M2 conclusion: the wire contract is adapter-agnostic,
//! so the same REPL attaches to every adapter, GPUI included).

#![forbid(unsafe_code)]

pub mod args;
pub mod bless;
pub mod deps;
pub mod repl;
pub mod report;
pub mod runner;
pub mod session;
pub mod show_trace;
pub mod spec;

pub use args::{CliError, Command, help_text, parse_args};
pub use bless::{BlessReport, bless_dir};
pub use deps::{install_deps_command, platform_deps};
pub use repl::{Line, Repl, StepOutcome};
pub use runner::{RunOutcome, RunnerError, TestOptions};
pub use session::{ConnectTarget, Connection, TargetParseError};
pub use show_trace::show_trace;

/// The agent string sent in the GWP handshake (§7 `Hello.agent`).
pub const AGENT: &str = concat!("guiwright-cli/", env!("CARGO_PKG_VERSION"));
