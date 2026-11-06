//! Guiwright kit — the user-facing crate (architecture §5.2 `guiwright-kit`).
//!
//! * [`App`] — the embedded app: launch, locators, clock, screenshots.
//! * [`expect`] — retry-until-timeout assertions (§16).
//! * [`test`] — the `#[guiwright_kit::test]` attribute: launches the
//!   fixture app, wires per-test artifacts (§11 nextest naming), reports
//!   unit and `Result` bodies.
//! * [`TestContext`] — nextest environment facts (run id, attempt).
//!
//! Embedded mode (§5.1): the kit calls the adapter directly in the test's
//! process; the same trait boundary the remote host exercises.

#![forbid(unsafe_code)]

pub mod app;
pub mod context;
pub mod run;

pub use app::{App, KitError, Locator, expect};
pub use context::TestContext;
pub use guiwright_macros::test;
pub use guiwright_shot::baseline::{BlessMode, SnapshotOutcome};
pub use run::TestOutcome;
