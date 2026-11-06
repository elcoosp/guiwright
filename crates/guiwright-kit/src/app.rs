//! The embedded App (§6.3 client-side API): `App::launch_fixture` for
//! tests, `App::launch(adapter)` for real harnesses — the same locator,
//! expect, clock, trace and screenshot surface on both (§5.1 "embedded
//! tests and remote sessions behave identically").
//!
//! ```ignore
//! let mut app = App::launch_fixture("login_flow");
//! app.get_by_role(Role::Button, "Save").click().unwrap();
//! expect(app.get_by_text("Welcome, Ada")).to_be_visible().unwrap();
//! app.screenshot("main").unwrap();
//! let trace = app.finish(); // guiwright-<test>-<ts>.zip when tracing
//! ```

use std::path::PathBuf;

use accesskit::Role;
use guiwright_core::selector::{Selector, TextMatch};
use guiwright_core::tree::Resolved;
use guiwright_core::{CoreError, NodeKey};
use guiwright_driver::adapter::{AdapterError, FrameworkAdapter, SemAction};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::session::Session;
use guiwright_shot::Image;
use guiwright_shot::baseline::{BlessMode, SnapshotOutcome};
use guiwright_shot::config::GuiwrightConfig;
use guiwright_trace::schema::AdapterRecord;
use guiwright_trace::{TraceOptions, TraceRecorder};

use crate::context::TestContext;

/// Kit-level error (kit methods span core, driver, shot and trace).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KitError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    #[error(transparent)]
    Shot(#[from] guiwright_shot::ShotError),
    #[error("screenshot unsupported by this adapter: {0}")]
    NoPixels(String),
    #[error("trace error: {0}")]
    Trace(String),
}

/// A locator (§6.3 `Locator = Selector + retry policy`): every action
/// auto-waits through the session's event-driven loop (§15.3) — tests
/// never `sleep()`.
pub struct Locator<'a, A: FrameworkAdapter> {
    app: &'a mut App<A>,
    selector: Selector,
}

impl<A: FrameworkAdapter> std::fmt::Debug for Locator<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Locator")
            .field("selector", &self.selector.render())
            .finish_non_exhaustive()
    }
}

impl<'a, A: FrameworkAdapter> Locator<'a, A> {
    /// `locator.first()` (§8.1 filters).
    pub fn first(mut self) -> Self {
        self.selector = self.selector.first();
        self
    }

    /// `locator.last()`.
    pub fn last(mut self) -> Self {
        self.selector = self.selector.last();
        self
    }

    /// `locator.nth(index)`.
    pub fn nth(mut self, index: usize) -> Self {
        self.selector = self.selector.nth(index);
        self
    }

    /// `locator.has_text(...)` narrowing.
    pub fn has_text(mut self, text: impl Into<TextMatch>) -> Self {
        self.selector = self.selector.has_text(text);
        self
    }

    /// `locator.click()` — actionability-checked, auto-waits (§8.3–§8.4).
    pub fn click(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let outcome = self.app.session.click(&self.selector)?;
        self.app.record_action("click", &self.selector);
        Ok(outcome)
    }

    /// `locator.dblclick()`.
    pub fn dblclick(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let outcome = self.app.session.perform_action(
            &self.selector,
            SemAction::DoubleClick,
            &Default::default(),
        )?;
        self.app.record_action("dblclick", &self.selector);
        Ok(outcome)
    }

    /// `locator.fill(text)`.
    pub fn fill(self, text: &str) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let outcome = self.app.session.fill(&self.selector, text)?;
        self.app.record_action("fill", &self.selector);
        Ok(outcome)
    }

    /// `locator.hover()`.
    pub fn hover(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let outcome = self.app.session.hover(&self.selector)?;
        self.app.record_action("hover", &self.selector);
        Ok(outcome)
    }

    /// Raw resolution (no action) — for custom flows.
    pub fn resolve(self) -> Result<Resolved, KitError> {
        Ok(self.app.session.resolve(&self.selector)?)
    }

    pub fn selector(&self) -> &Selector {
        &self.selector
    }
}

/// The embedded app (§6.3).
pub struct App<A: FrameworkAdapter = MockAdapter> {
    pub(crate) session: Session<A>,
    context: TestContext,
    config: GuiwrightConfig,
    baseline_root: Option<PathBuf>,
    trace: Option<TraceRecorder>,
    trace_path: Option<PathBuf>,
    finished: bool,
}

impl<A: FrameworkAdapter> std::fmt::Debug for App<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("context", &self.context)
            .field("finished", &self.finished)
            .field("tracing", &self.trace.is_some())
            .finish_non_exhaustive()
    }
}

impl App<MockAdapter> {
    /// Launch the standard fixture app — the M1 embedded default used by
    /// `#[guiwright_kit::test]` and the kit's own tests.
    pub fn launch_fixture(test_name: &str) -> Self {
        Self::launch_fixture_with(test_name, |adapter| adapter)
    }

    /// Launch the fixture app with the adapter customized before attach
    /// (pixels scripting for capture tests, extra scripted updates, …).
    pub fn launch_fixture_with(
        test_name: &str,
        customize: impl FnOnce(MockAdapter) -> MockAdapter,
    ) -> Self {
        let fixture = guiwright_core::test_util::Fixture::buttons();
        let mut adapter = customize(MockAdapter::with_fixture(&fixture));
        adapter.push_frame(1);
        adapter.push_frame(1);
        Self::launch(test_name, adapter)
    }
}

impl<A: FrameworkAdapter> App<A> {
    /// Launch any adapter in embedded mode (§5.1: the kit calls the
    /// adapter directly in the test's process/thread).
    pub fn launch(test_name: &str, adapter: A) -> Self {
        let context = TestContext::from_env(test_name);
        let config =
            guiwright_shot::config::load_from_dir(&context.baseline_dir()).unwrap_or_default();
        // Tracing is opt-in per §12 (GW_TRACE=1) — the writer thread must
        // never spin for tests that do not ask for it.
        let trace = if std::env::var("GW_TRACE").ok().as_deref() == Some("1") {
            let path = context.trace_path();
            let options = TraceOptions::new(
                path.clone(),
                &context.test_name,
                AdapterRecord {
                    name: adapter.meta().name.to_string(),
                    framework: adapter.meta().framework.to_string(),
                    framework_version: adapter.meta().framework_version.clone(),
                },
            );
            Some(TraceRecorder::start(options))
        } else {
            None
        };
        Self {
            session: Session::new(adapter),
            context,
            config,
            baseline_root: None,
            trace,
            trace_path: None,
            finished: false,
        }
    }

    /// Override the baseline root (tests never write into the repo tree).
    pub fn with_baseline_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.baseline_root = Some(dir.into());
        self
    }

    /// Override the config (tests pin tolerances without Guiwright.toml).
    pub fn with_config(mut self, config: GuiwrightConfig) -> Self {
        self.config = config;
        self
    }

    /// Enable tracing with an explicit archive path (the env-driven path
    /// is `GW_TRACE=1`; tests pin the path for determinism, §12).
    pub fn enable_trace(&mut self, path: impl Into<PathBuf>) {
        if self.trace.is_some() {
            return;
        }
        let path = path.into();
        let options = TraceOptions::new(
            path.clone(),
            &self.context.test_name,
            AdapterRecord {
                name: self.session.adapter().meta().name.to_string(),
                framework: self.session.adapter().meta().framework.to_string(),
                framework_version: self.session.adapter().meta().framework_version.clone(),
            },
        );
        self.trace = Some(TraceRecorder::start(options));
    }

    pub fn context(&self) -> &TestContext {
        &self.context
    }

    pub fn session(&mut self) -> &mut Session<A> {
        &mut self.session
    }

    /// `app.get_by_role(Role::Button, "Save")` (§8.1).
    pub fn get_by_role(&mut self, role: Role, name: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::role_named(role, name),
        }
    }

    /// `app.get_by_label(...)` — accessible label.
    pub fn get_by_label(&mut self, name: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::label(name),
        }
    }

    /// `app.get_by_text(...)`.
    pub fn get_by_text(&mut self, text: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::text(text),
        }
    }

    /// `app.get_by_test_id(...)` (§8.1 O(1) hash).
    pub fn get_by_test_id(&mut self, id: impl Into<String>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::test_id(id),
        }
    }

    /// Unnamed role locator — every widget of the role; strict mode
    /// applies (the architecture's ambiguity-error surface, §8.2).
    pub fn role_locator(&mut self, role: Role) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::role(role),
        }
    }

    /// `app.get_by_value(...)`.
    pub fn get_by_value(&mut self, value: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::value(value),
        }
    }

    /// `app.wait_for_settle()` — frames drained, timers idle (§6.3).
    pub fn wait_for_settle(&mut self) {
        self.session
            .settle(std::time::Instant::now() + std::time::Duration::from_secs(5));
    }

    /// `app.clock().fast_forward(dt)` (§14) — through the adapter's clock
    /// service, then settle.
    pub fn clock_fast_forward(&mut self, dt: std::time::Duration) -> Result<(), KitError> {
        self.session.clock_fast_forward(dt)?;
        self.record_clock(dt);
        Ok(())
    }

    /// `expect(window).to_have_screenshot(name)` (§10): render on demand,
    /// diff against the per-platform baseline, bless on demand.
    pub fn screenshot(
        &mut self,
        name: &str,
        bless: BlessMode,
    ) -> Result<SnapshotOutcome, KitError> {
        let frame = self.session.adapter_mut().render_frame(None)?;
        let pixels = frame
            .pixels
            .ok_or_else(|| KitError::NoPixels(self.session.adapter().meta().name.to_string()))?;
        let image = Image {
            width: frame.size.0,
            height: frame.size.1,
            rgba: pixels.to_vec(),
        };
        let root = self
            .baseline_root
            .clone()
            .unwrap_or_else(|| self.context.baseline_dir());
        let store = guiwright_shot::baseline::BaselineStore::new(root);
        let tolerance = self.config.screenshots.effective();
        let outcome = store.assert_snapshot(
            name,
            self.session.adapter().meta().framework,
            &image,
            tolerance,
            bless,
        )?;
        self.record_event("screenshot", format!("{name} → {outcome:?}"));
        Ok(outcome)
    }

    /// Stop the trace and return the archive path (§12
    /// `app.trace().stop()`; `None` when tracing was not enabled).
    pub fn finish(&mut self) -> Option<PathBuf> {
        if self.finished {
            return self.trace_path.clone();
        }
        self.finished = true;
        let recorder = self.trace.take()?;
        match recorder.stop() {
            Ok(file) => {
                self.trace_path = Some(file.path.clone());
                self.trace_path.clone()
            }
            Err(e) => {
                tracing::warn!("trace stop failed: {e}");
                None
            }
        }
    }

    /// The trace path once `finish` ran (for assertions).
    pub fn trace_path(&self) -> Option<&PathBuf> {
        self.trace_path.as_ref()
    }

    pub(crate) fn record_action(&mut self, kind: &'static str, selector: &Selector) {
        self.record_event(kind, selector.render());
    }

    pub(crate) fn record_event(&mut self, kind: &'static str, detail: String) {
        if let Some(recorder) = self.trace.as_mut() {
            let frame = self.session.frame_seq();
            recorder.event(frame, kind, detail);
        }
    }

    fn record_clock(&mut self, dt: std::time::Duration) {
        if let Some(recorder) = self.trace.as_mut() {
            let frame = self.session.frame_seq();
            recorder.clock_jump(guiwright_trace::ClockJump {
                frame_seq: frame,
                command: guiwright_trace::ClockCommand::FastForward {
                    micros: dt.as_micros() as u64,
                },
                virtual_time_us: dt.as_micros() as u64,
            });
        }
    }

    /// Node-key helper for tests that want the identity of a resolution.
    pub fn key_of(&mut self, selector: &Selector) -> Result<NodeKey, KitError> {
        Ok(self.session.resolve(selector)?.key)
    }
}

/// `expect(app.get_by_text("Welcome, Ada")).to_be_visible()` (§16
/// retry-until-timeout assertions). Free function, architecture shape:
/// it consumes the locator and returns the pending assertion.
pub fn expect<A: FrameworkAdapter>(locator: Locator<'_, A>) -> core_expect::Pending<'_, A> {
    let Locator { app, selector } = locator;
    core_expect::Pending {
        builder: app.session.expect(selector),
    }
}

/// Tiny namespace so the kit's expect reads like the architecture's
/// `expect(locator).to_*` while delegating to the session builder.
pub(crate) mod core_expect {
    use super::*;

    pub struct Pending<'a, A: FrameworkAdapter> {
        pub builder: guiwright_driver::session::ExpectBuilder<'a, A>,
    }

    impl<A: FrameworkAdapter> std::fmt::Debug for Pending<'_, A> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Pending").finish_non_exhaustive()
        }
    }

    // Consuming builder by design — mirrors session::ExpectBuilder and the
    // architecture's `expect(locator).to_be_visible()` shape (§16): the
    // assertion is single-shot, so `self` moves into the check.
    #[allow(clippy::wrong_self_convention)]
    impl<'a, A: FrameworkAdapter> Pending<'a, A> {
        pub fn to_be_visible(self) -> Result<(), CoreError> {
            self.builder.to_be_visible()
        }
        pub fn to_be_hidden(self) -> Result<(), CoreError> {
            self.builder.to_be_hidden()
        }
        pub fn to_be_enabled(self) -> Result<(), CoreError> {
            self.builder.to_be_enabled()
        }
        pub fn to_be_disabled(self) -> Result<(), CoreError> {
            self.builder.to_be_disabled()
        }
        pub fn to_be_focused(self) -> Result<(), CoreError> {
            self.builder.to_be_focused()
        }
        pub fn to_have_text(self, text: impl Into<TextMatch>) -> Result<(), CoreError> {
            self.builder.to_have_text(text)
        }
        pub fn to_have_value(self, value: impl Into<TextMatch>) -> Result<(), CoreError> {
            self.builder.to_have_value(value)
        }
    }
}
