//! The embedded App (§6.3 client-side API): `App::launch_fixture` for
//! tests, `App::launch(adapter)` for real harnesses — the same locator,
//! expect, clock, trace and screenshot surface on both (§5.1 "embedded
//! tests and remote sessions behave identically").
//!
//! ```ignore
//! let mut app = App::launch_fixture("login_flow");
//! app.get_by_role(Role::Button, "Save").click().unwrap();
//! expect(app.get_by_text("Welcome, Ada")).to_be_visible().unwrap();
//! app.screenshot("main", BlessMode::Off).unwrap();
//! let trace = app.finish(); // guiwright-<test>-<ts>.zip when tracing
//! ```

use std::path::PathBuf;

use accesskit::Role;
use guiwright_core::selector::{Selector, TextMatch};
use guiwright_core::tree::Resolved;
use guiwright_core::{CoreError, NodeKey};
use guiwright_driver::ActionOpts;
use guiwright_driver::SeamRegistry;
use guiwright_driver::adapter::{AdapterError, ClockCmd, FrameworkAdapter, SemAction};
use guiwright_driver::clock::ClockReport;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::session::Session;

use crate::input::{KeyboardHandle, MouseHandle};
use crate::window::WindowHandle;
use guiwright_inspect::codegen::{CodegenRecorder, NodeFacts};
use guiwright_shot::Image;
use guiwright_shot::baseline::{BlessMode, SnapshotOutcome};
use guiwright_shot::config::GuiwrightConfig;
use guiwright_trace::schema::{AdapterRecord, ClockCommand};
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
    #[error("mock error: {0}")]
    Mock(String),
}

/// A locator (§6.3 `Locator = Selector + retry policy`): every action
/// auto-waits through the session's event-driven loop (§15.3) — tests
/// never `sleep()`.
pub struct Locator<'a, A: FrameworkAdapter> {
    /// Crate-visible so the surface modules (the `expect` helper here, the
    /// M21 `wait_for` module) share one access path — the locator is a
    /// borrow-and-act handle, not a data record.
    pub(crate) app: &'a mut App<A>,
    pub(crate) selector: Selector,
    /// The optional window pin (§16 v0.4, the kit half of M6): when set,
    /// every resolution path this locator drives is scoped to that
    /// window's subtree — the same semantics the wire's `Window::Use`
    /// gives a remote connection. `None` resolves the whole tree (the
    /// historical single-window surface).
    pub(crate) window: Option<String>,
}

impl<A: FrameworkAdapter> std::fmt::Debug for Locator<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Locator")
            .field("selector", &self.selector.render())
            .finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> Locator<'_, A> {
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
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| session.click(&selector))?;
        app.record_scoped_action("click", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// `locator.dblclick()`.
    pub fn dblclick(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(&selector, SemAction::DoubleClick, &ActionOpts::default())
        })?;
        app.record_scoped_action("dblclick", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// `locator.fill(text)`.
    pub fn fill(self, text: &str) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.fill(&selector, text)
        })?;
        app.record_scoped_action("fill", &recorded, Some(text), window.as_deref());
        Ok(outcome)
    }

    /// `locator.hover()`.
    pub fn hover(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| session.hover(&selector))?;
        app.record_scoped_action("hover", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// `locator.press(key)` — the §9 keyboard vocabulary aimed at the
    /// resolved node (§16 actions row): the same actionability gate and
    /// auto-wait as every locator action, the plain (no-modifier) chord.
    pub fn press(self, key: &str) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(
                &selector,
                SemAction::Press {
                    key: key.to_string(),
                    modifiers: Vec::new(),
                },
                &ActionOpts::default(),
            )
        })?;
        app.record_scoped_action("press", &recorded, Some(key), window.as_deref());
        Ok(outcome)
    }

    /// `locator.check()` — the wire's `Check` semantic action.
    pub fn check(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(&selector, SemAction::Check, &ActionOpts::default())
        })?;
        app.record_scoped_action("check", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// `locator.uncheck()`.
    pub fn uncheck(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(&selector, SemAction::Uncheck, &ActionOpts::default())
        })?;
        app.record_scoped_action("uncheck", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// `locator.set_checked(checked)` — routes through the same
    /// `Check`/`Uncheck` vocabulary (Playwright parity: `true` checks).
    pub fn set_checked(
        self,
        checked: bool,
    ) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        if checked {
            self.check()
        } else {
            self.uncheck()
        }
    }

    /// `locator.focus()`.
    pub fn focus(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(&selector, SemAction::Focus, &ActionOpts::default())
        })?;
        app.record_scoped_action("focus", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// `locator.blur()`.
    pub fn blur(self) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(&selector, SemAction::Blur, &ActionOpts::default())
        })?;
        app.record_scoped_action("blur", &recorded, None, window.as_deref());
        Ok(outcome)
    }

    /// Raw resolution (no action) — for custom flows. Nothing is
    /// recorded: resolution without an action is not a replayable step.
    pub fn resolve(self) -> Result<Resolved, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), move |session| session.resolve(&selector))?)
    }

    /// `locator.screenshot(name, bless)` — the §10 element screenshot
    /// (M9): resolve (auto-wait), render the window frame once, **crop it
    /// by the resolved node's physical bounds** (§10 "crop from the window
    /// frame by the node's clip-aware rect — never re-render"), and
    /// assert against the same per-platform baselines
    /// [`App::screenshot`] uses (`tests/screenshots/<name>/`, per-OS
    /// tolerances, the bless flow). The first run needs
    /// `BlessMode::Failing` to write the baseline; a mismatch fails the
    /// test and leaves `.new.png` + `.diff.png` artifacts next to it.
    pub fn screenshot(self, name: &str, bless: BlessMode) -> Result<SnapshotOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        // Resolution is window-scoped when the locator is pinned (§16
        // v0.4); the frame itself is the adapter's single render surface,
        // so the crop path is unchanged.
        let rendered = selector.clone();
        let resolved =
            app.run_scoped(window.as_deref(), move |session| session.resolve(&selector))?;
        let rect = resolved.bounds.ok_or_else(|| {
            CoreError::other(format!(
                "{} has no bounds — nothing to crop (§10)",
                rendered.render()
            ))
        })?;
        let frame = app.session.adapter_mut().render_frame(None)?;
        let pixels = frame
            .pixels
            .ok_or_else(|| KitError::NoPixels(app.session.adapter().meta().name.to_string()))?;
        // Physical px → the crop window, clamped into the frame (§9: the
        // bounds are already physical; the frame is the window's buffer).
        // The clamp pins the value into `[0, max]` before the conversion
        // (the §10/§15 guarded-conversion pattern): the float→i64 step
        // keeps the sign domain (clamped ≥ 0), and the `u32::try_from`
        // proves the range instead of truncating — `unwrap_or(max)` is
        // the unreachable arm, kept honest by the clamp.
        let clamp = |v: f64, max: u32| {
            u32::try_from(v.round().clamp(0.0, f64::from(max)) as i64).unwrap_or(max)
        };
        let (x0, y0) = (clamp(rect.x0, frame.size.0), clamp(rect.y0, frame.size.1));
        let (x1, y1) = (clamp(rect.x1, frame.size.0), clamp(rect.y1, frame.size.1));
        let image = Image {
            width: frame.size.0,
            height: frame.size.1,
            rgba: pixels.to_vec(),
        };
        let element = guiwright_shot::crop(&image, x0, y0, x1, y1)?;

        let root = app
            .baseline_root
            .clone()
            .unwrap_or_else(|| app.context.baseline_dir());
        let store = guiwright_shot::baseline::BaselineStore::new(root);
        let tolerance = app.config.screenshots.effective();
        let outcome = store.assert_snapshot(
            name,
            app.session.adapter().meta().framework,
            &element,
            tolerance,
            bless,
        )?;
        app.record_scoped_action(
            "screenshot_element",
            &rendered,
            Some(name),
            window.as_deref(),
        );
        app.record_event(
            "screenshot_element",
            format!("{name} [({x0},{y0}) → ({x1},{y1})] → {outcome:?}"),
        );
        Ok(outcome)
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
    /// Clock-ledger rows already flushed to the trace (incremental flush,
    /// §12 `clock.bin`).
    ledger_written: usize,
    /// Codegen recorder (§13) — off unless `enable_codegen` ran.
    codegen: Option<CodegenRecorder>,
    /// Recorded actions awaiting fact resolution (deferred so the locator
    /// action path stays borrow-free; drained at read time). The fourth
    /// slot is the window pin (§16 v0.4) — `None` for the whole-tree
    /// surface.
    codegen_pending: Vec<(String, Selector, Option<String>, Option<String>)>,
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
            ledger_written: 0,
            codegen: None,
            codegen_pending: Vec::new(),
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

    /// Enable the codegen recorder (§13): subsequent locator actions are
    /// emitted as kit code against the minimal stable selector.
    pub fn enable_codegen(&mut self) {
        if self.codegen.is_none() {
            self.codegen = Some(CodegenRecorder::new());
        }
    }

    /// Is the kit codegen recorder active?
    pub(crate) fn codegen_active(&self) -> bool {
        self.codegen.is_some()
    }

    /// Land one raw-input codegen line (§13 over §9): pre-rendered by the
    /// driver's renderer (the input vocabulary has no selector to
    /// validate), plus an optional recorded-code warning.
    pub(crate) fn record_input_code(&mut self, line: String, warning: Option<String>) {
        let Some(mut recorder) = self.codegen.take() else {
            return;
        };
        if let Some(warning) = warning {
            recorder.warn(warning);
        }
        recorder.record_raw(line);
        self.codegen = Some(recorder);
    }

    /// The generated kit code + strict-mode warnings (§13); `None` while
    /// recording is disabled.
    pub fn codegen_code(&mut self) -> Option<(String, Vec<String>)> {
        self.record_codegen_pending();
        self.codegen
            .as_ref()
            .map(|r| (r.code(), r.warnings().to_vec()))
    }

    /// Stop codegen and consume the recorder (§7 `Inspect::StopRecording`).
    pub fn codegen_finish(&mut self) -> Option<(String, Vec<String>)> {
        self.record_codegen_pending();
        self.codegen.take().map(CodegenRecorder::finish)
    }

    /// `app.get_by_role(Role::Button, "Save")` (§8.1).
    pub fn get_by_role(&mut self, role: Role, name: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::role_named(role, name),
            window: None,
        }
    }

    /// `app.get_by_label(...)` — accessible label.
    pub fn get_by_label(&mut self, name: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::label(name),
            window: None,
        }
    }

    /// `app.get_by_text(...)`.
    pub fn get_by_text(&mut self, text: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::text(text),
            window: None,
        }
    }

    /// `app.get_by_test_id(...)` (§8.1 O(1) hash).
    pub fn get_by_test_id(&mut self, id: impl Into<String>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::test_id(id),
            window: None,
        }
    }

    /// Unnamed role locator — every widget of the role; strict mode
    /// applies (the architecture's ambiguity-error surface, §8.2).
    pub fn role_locator(&mut self, role: Role) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::role(role),
            window: None,
        }
    }

    /// `app.get_by_value(...)`.
    pub fn get_by_value(&mut self, value: impl Into<TextMatch>) -> Locator<'_, A> {
        Locator {
            app: self,
            selector: Selector::value(value),
            window: None,
        }
    }

    /// The session's window targets (§16 v0.4, M6): adapter-reported
    /// metadata (handle / title / focus). Single-window adapters report
    /// exactly one `main` target; multi-window adapters report one entry
    /// per grafted window. The same list `window list` prints in the REPL
    /// and `windowList()` returns in the TypeScript client.
    pub fn windows(&mut self) -> Vec<guiwright_driver::WindowInfo> {
        self.session.windows()
    }

    /// Aim a borrow-and-act handle at one window target (§16 v0.4, the kit
    /// half of M6): locators born from the handle resolve **only inside
    /// that window's subtree**, the same semantics the wire's
    /// `Window::Use` gives a remote connection. The handle is validated
    /// against the tree at aim time (the source of truth, §16 v0.4 — an
    /// adapter metadata entry without tree content is not a usable
    /// target), so a dead handle fails here with the typed
    /// `UnknownWindow` error instead of mid-flow.
    pub fn window(&mut self, handle: impl Into<String>) -> Result<WindowHandle<'_, A>, KitError> {
        let handle = handle.into();
        self.session.window_scope(&handle)?;
        // Metadata is the adapter's display word for the target; a tree
        // entry without a metadata row (possible for a grafted window the
        // adapter did not script) still aims, with the handle as the
        // title and focus unset.
        let (title, focused) = self
            .session
            .windows()
            .into_iter()
            .find(|w| w.handle == handle)
            .map_or_else(|| (handle.clone(), false), |w| (w.title, w.focused));
        Ok(WindowHandle {
            app: self,
            handle,
            title,
            focused,
        })
    }

    /// Run `op` under the locator's window pin (§16 v0.4): `None` is the
    /// historical whole-tree surface, `Some(handle)` validates + scopes +
    /// clears in one rendezvous (the session's `scoped`, the server's
    /// set/act/clear pattern published for embedded consumers).
    pub(crate) fn run_scoped<R>(
        &mut self,
        window: Option<&str>,
        op: impl FnOnce(&mut Session<A>) -> Result<R, CoreError>,
    ) -> Result<R, CoreError> {
        match window {
            None => op(&mut self.session),
            Some(handle) => self.session.scoped(handle, op),
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
        self.record_clock_ledger();
        Ok(())
    }

    /// The full clock surface (§14): `app.clock().install_virtual()`,
    /// `.fast_forward(dt)`, `.run_for(dt)`, `.pause()`, `.resume()`,
    /// `.set_time(t)`, `.state()`.
    pub fn clock(&mut self) -> ClockHandle<'_, A> {
        ClockHandle { app: self }
    }

    /// The mock domain (§14 state seams): `app.mock().install(id, resp)`,
    /// `.clear(id)`, `.call(id, scope)`, `.reset_state(scope)`. Seams must
    /// be registered first via [`App::register_seams`].
    pub fn mock(&mut self) -> MockHandle<'_, A> {
        MockHandle { app: self }
    }

    /// The inspector surface (§13, M8): `app.inspect().enable()`,
    /// `.highlight(&selector)`, `.clear_highlight()`, `.disable()` — the
    /// live overlay session. The adapter composites hover outlines +
    /// interactive-role badges at present time; the highlighted node's
    /// facts come back for debug prints, exactly what the REPL's `inspect`
    /// command and the TypeScript `inspectHighlight` render over the wire.
    pub fn inspect(&mut self) -> InspectHandle<'_, A> {
        InspectHandle { app: self }
    }

    /// The video surface (§10, M3): `app.video().start_in(dir, fps)` /
    /// `.start(fps)` / `.stop()` — records the presented frames between
    /// the two calls (hash-equality dedup collapses static spans) and
    /// finalizes a `.webm` (ffmpeg) or PNG frame folder on stop.
    pub fn video(&mut self) -> VideoHandle<'_, A> {
        VideoHandle { app: self }
    }

    /// The raw input surface (§9): `app.mouse().click(x, y)`,
    /// `.dblclick(x, y)`, `.drag(from_x, from_y, to_x, to_y)`,
    /// `.wheel(dx, dy)` and the raw `move_to` / `down` / `up` primitives —
    /// the same gesture vocabulary the REPL and the TypeScript client
    /// speak over the wire, embedded (the M7 "kit-embedded input" row).
    /// Coordinates are logical px relative to the target window;
    /// `.in_window(handle)` pins it for multi-window apps.
    pub fn mouse(&mut self) -> MouseHandle<'_, A> {
        MouseHandle::new(self)
    }

    /// The keyboard surface (§9): `app.keyboard().press(key)`,
    /// `.press_with(key, modifiers)` and `.type_text(text)` ride the
    /// driver's synthesis exactly like the wire clients' `key` / `text`
    /// commands.
    pub fn keyboard(&mut self) -> KeyboardHandle<'_, A> {
        KeyboardHandle::new(self)
    }

    /// Register the app's boot seams (§14). Use the
    /// `guiwright_kit::register_seams![...]` macro to build the registry:
    ///
    /// ```ignore
    /// app.register_seams(&guiwright_kit::register_seams![reset_db, seed_users]);
    /// ```
    pub fn register_seams(&mut self, registry: &SeamRegistry) {
        self.session.register_seams(registry);
    }

    /// Export the self-contained `trace.html` viewer for the finished
    /// trace (§12 "open on failure" deep links). `finish()` must have run.
    pub fn export_trace_viewer(
        &mut self,
        out_path: impl Into<PathBuf>,
    ) -> Result<PathBuf, KitError> {
        let Some(trace_path) = self.trace_path.clone() else {
            return Err(KitError::Trace(
                "no trace archive: finish() has not run or tracing was never enabled".into(),
            ));
        };
        let archive = guiwright_viewer::TraceArchive::open(&trace_path)
            .map_err(|e| KitError::Trace(e.to_string()))?;
        let timeline = guiwright_viewer::Timeline::from_archive(&archive);
        let html = guiwright_viewer::render_html(&archive, &timeline);
        let out = out_path.into();
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| KitError::Trace(format!("{}: {e}", parent.display())))?;
        }
        std::fs::write(&out, html)
            .map_err(|e| KitError::Trace(format!("{}: {e}", out.display())))?;
        Ok(out)
    }

    /// The window screenshot assertion (§10): render on demand, diff
    /// against the per-platform baseline, bless on demand — the element
    /// form is [`App::get_by_*` → `Locator::screenshot`].
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

    /// The action tap (§13): the trace event names the selector — plus the
    /// window pin when one is set (§16 v0.4) — and the codegen line
    /// records the `app.window(…).…` shape when a handle is set.
    pub(crate) fn record_scoped_action(
        &mut self,
        kind: &'static str,
        selector: &Selector,
        argument: Option<&str>,
        window: Option<&str>,
    ) {
        match window {
            None => self.record_event(kind, selector.render()),
            Some(handle) => {
                self.record_event(kind, format!("{} @ window {handle}", selector.render()));
            }
        }
        if self.codegen.is_some() {
            self.codegen_pending.push((
                kind.to_string(),
                selector.clone(),
                argument.map(str::to_string),
                window.map(str::to_string),
            ));
        }
    }

    pub(crate) fn record_event(&mut self, kind: &'static str, detail: String) {
        if let Some(recorder) = self.trace.as_mut() {
            let frame = self.session.frame_seq();
            recorder.event(frame, kind, detail);
        }
    }

    /// Flush **new** clock-ledger entries into the trace (`clock.bin`,
    /// §12/§14 — "clock.bin in traces replays timing"). Consecutive
    /// fast-forward steps are folded into single jumps with the delta.
    pub(crate) fn record_clock_ledger(&mut self) {
        let Some(recorder) = self.trace.as_mut() else {
            return;
        };
        let ledger = self.session.clock_ledger();
        let mut previous_us: Option<u64> = None;
        for entry in &ledger[self.ledger_written..] {
            let command = match entry.command {
                guiwright_driver::clock::LedgerCommand::InstallVirtual => {
                    ClockCommand::InstallVirtual
                }
                guiwright_driver::clock::LedgerCommand::FastForward => {
                    let delta = entry
                        .virtual_time_us
                        .saturating_sub(previous_us.unwrap_or(0));
                    ClockCommand::FastForward { micros: delta }
                }
                guiwright_driver::clock::LedgerCommand::RunFor => {
                    ClockCommand::RunFor { micros: 0 }
                }
                guiwright_driver::clock::LedgerCommand::Pause => ClockCommand::Pause,
                guiwright_driver::clock::LedgerCommand::Resume => ClockCommand::Resume,
                guiwright_driver::clock::LedgerCommand::SetTime => ClockCommand::FastForward {
                    micros: entry
                        .virtual_time_us
                        .saturating_sub(previous_us.unwrap_or(0)),
                },
            };
            recorder.clock_jump(guiwright_trace::ClockJump {
                frame_seq: entry.frame_seq,
                command,
                virtual_time_us: entry.virtual_time_us,
            });
            previous_us = Some(entry.virtual_time_us);
        }
        self.ledger_written = ledger.len();
    }

    /// Drain pending actions into the codegen recorder (§13): resolve the
    /// node, collect its facts, let the cost model pick the selector.
    /// Window-pinned actions resolve inside their subtree (§16 v0.4) and
    /// record the `app.window(…).…` shape.
    fn record_codegen_pending(&mut self) {
        if self.codegen_pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.codegen_pending);
        // Take the recorder out: resolution needs `&mut session`.
        let Some(mut recorder) = self.codegen.take() else {
            return;
        };
        for (kind, selector, argument, window) in &pending {
            // The recorded facts must be the scoped resolution's: a pinned
            // action that no longer resolves inside its window is a
            // warning, not a cross-window hit.
            let resolved = match window {
                None => self.session.resolve(selector),
                Some(handle) => self
                    .session
                    .scoped(handle, |session| session.resolve(selector)),
            };
            let Ok(resolved) = resolved else {
                recorder.warn(format!(
                    "action `{kind}` target no longer resolves: {}",
                    selector.render()
                ));
                continue;
            };
            // The cost model judges uniqueness where the action resolved:
            // the pinned subtree root rides along so a window-scoped
            // "Save" records without the cross-window `.first()` hedge
            // (§16 v0.4 + §13).
            let scope = match window {
                None => None,
                Some(handle) => self.session.window_scope(handle).ok().flatten(),
            };
            let pinned = window.as_deref().zip(scope.as_ref());
            let facts = self.collect_facts(&resolved.key, resolved.bounds);
            let _ = recorder.record(
                kind,
                argument.as_deref(),
                &facts,
                self.session.tree(),
                pinned,
            );
        }
        self.codegen = Some(recorder);
    }

    /// Node facts at the action tap (§13 "captured at the adapter tap").
    fn collect_facts(&self, key: &NodeKey, bounds: Option<accesskit::Rect>) -> NodeFacts {
        let tree = self.session.tree();
        let node = tree.node(key);
        NodeFacts {
            key: *key,
            role: node.as_ref().map(|n| format!("{:?}", n.role())),
            name: node.as_ref().and_then(accesskit_consumer::NodeRef::label),
            label: node.as_ref().and_then(accesskit_consumer::NodeRef::label),
            value: node.as_ref().and_then(accesskit_consumer::NodeRef::value),
            test_id: tree.test_id_of(key).map(str::to_string),
            bounds: bounds.map(|r| [r.x0, r.y0, r.x1, r.y1]),
        }
    }

    /// Node-key helper for tests that want the identity of a resolution.
    pub fn key_of(&mut self, selector: &Selector) -> Result<NodeKey, KitError> {
        Ok(self.session.resolve(selector)?.key)
    }
}

/// `app.clock()` — the §14 virtual clock surface.
pub struct ClockHandle<'a, A: FrameworkAdapter> {
    app: &'a mut App<A>,
}

impl<A: FrameworkAdapter> std::fmt::Debug for ClockHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClockHandle").finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> ClockHandle<'_, A> {
    fn push(&mut self, cmd: ClockCmd) -> Result<ClockReport, KitError> {
        let report = self.app.session.clock(cmd).map_err(KitError::Core);
        self.app.record_clock_ledger();
        self.app.record_event(
            "clock",
            format!("{cmd:?} → {:?}", report.as_ref().map(|r| r.state)),
        );
        report
    }

    /// `clock.install_virtual()` (§14 `InstallVirtual`).
    pub fn install_virtual(&mut self) -> Result<ClockReport, KitError> {
        self.push(ClockCmd::InstallVirtual)
    }

    /// `clock.fast_forward(dt)` — virtual time advances in frame-sized
    /// steps until drained (timers + animations settle between steps).
    pub fn fast_forward(&mut self, dt: std::time::Duration) -> Result<ClockReport, KitError> {
        self.push(ClockCmd::FastForward(dt))
    }

    /// `clock.run_for(dt)` — real-time pacing (refused while paused).
    pub fn run_for(&mut self, dt: std::time::Duration) -> Result<ClockReport, KitError> {
        self.push(ClockCmd::RunFor(dt))
    }

    /// `clock.pause()`.
    pub fn pause(&mut self) -> Result<ClockReport, KitError> {
        self.push(ClockCmd::Pause)
    }

    /// `clock.resume()`.
    pub fn resume(&mut self) -> Result<ClockReport, KitError> {
        self.push(ClockCmd::Resume)
    }

    /// `clock.set_time(t)` — absolute virtual time.
    pub fn set_time(&mut self, t: std::time::Duration) -> Result<ClockReport, KitError> {
        self.push(ClockCmd::SetTime(t))
    }

    /// Read the current clock state (probe, no mutation).
    pub fn state(&mut self) -> Result<guiwright_driver::adapter::ClockState, KitError> {
        Ok(self.push(ClockCmd::Probe)?.state)
    }
}

/// `app.mock()` — the §14 mock domain (state seams).
pub struct MockHandle<'a, A: FrameworkAdapter> {
    app: &'a mut App<A>,
}

/// `app.inspect()` — the §13 inspector overlay surface (M8).
pub struct InspectHandle<'a, A: FrameworkAdapter> {
    app: &'a mut App<A>,
}

impl<A: FrameworkAdapter> std::fmt::Debug for InspectHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectHandle").finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> InspectHandle<'_, A> {
    /// `inspect.enable()` (§7 `Inspect::EnableOverlay`): start the overlay
    /// session — the adapter composites hover outlines + interactive-role
    /// badges from the current tree at present time, refreshed every frame
    /// while it stays on. Adapters without an overlay surface report
    /// `Unsupported` (§6.2).
    pub fn enable(&mut self) -> Result<(), KitError> {
        let outcome = self
            .app
            .session
            .inspect_enable_overlay()
            .map_err(KitError::Core);
        self.app
            .record_event("inspect", format!("overlay → {outcome:?}"));
        outcome
    }

    /// `inspect.disable()` (§7 `Inspect::DisableOverlay`): stop
    /// compositing and drop the highlight.
    pub fn disable(&mut self) -> Result<(), KitError> {
        let outcome = self
            .app
            .session
            .inspect_disable_overlay()
            .map_err(KitError::Core);
        self.app
            .record_event("inspect", format!("overlay off → {outcome:?}"));
        outcome
    }

    /// `inspect.highlight(&selector)` (§7 `Inspect::Highlight`): aim the
    /// hover outline at the node the selector resolves to (strict mode)
    /// and return its facts — role, accessible name, bounds. Implies
    /// [`InspectHandle::enable`].
    pub fn highlight(
        &mut self,
        selector: &Selector,
    ) -> Result<guiwright_inspect::codegen::NodeFacts, KitError> {
        let outcome = self
            .app
            .session
            .inspect_highlight(selector)
            .map_err(KitError::Core);
        self.app.record_event(
            "inspect",
            format!("highlight {} → {outcome:?}", selector.render()),
        );
        outcome
    }

    /// `inspect.clear_highlight()`: drop the outline; the overlay session
    /// stays on (badges keep tracking the tree).
    pub fn clear_highlight(&mut self) -> Result<(), KitError> {
        self.app
            .session
            .inspect_clear_highlight()
            .map_err(KitError::Core)
    }

    /// Is the overlay session on?
    pub fn is_active(&self) -> bool {
        self.app.session.overlay_active()
    }
}

impl<A: FrameworkAdapter> std::fmt::Debug for MockHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockHandle").finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> MockHandle<'_, A> {
    /// `mock.install(id, response)` (§7 `Mock::InstallSeam`): preset a
    /// canned response; `None` clears the canned value (real hook runs).
    pub fn install(&mut self, id: &str, response: Option<String>) -> &mut Self {
        self.app.session.set_seam_response(id, response);
        self.app.record_event("mock", format!("install {id}"));
        self
    }

    /// `mock.clear(id)` (§7 `Mock::Clear`).
    pub fn clear(&mut self, id: &str) -> &mut Self {
        self.app.session.clear_seam(id);
        self.app.record_event("mock", format!("clear {id}"));
        self
    }

    /// `mock.call(id, scope)` — run a registered seam once.
    pub fn call(&mut self, id: &str, scope: &str) -> Result<String, KitError> {
        let detail = self
            .app
            .session
            .run_seam(id, scope)
            .map_err(|e| KitError::Mock(e.to_string()))?;
        self.app
            .record_event("mock", format!("call {id}({scope}) → {detail}"));
        Ok(detail)
    }

    /// `mock.reset_state(scope)` — fixture parity between tests (§14).
    pub fn reset_state(&mut self, scope: &str) -> Result<String, KitError> {
        let detail = self
            .app
            .session
            .reset_state(scope)
            .map_err(|e| KitError::Mock(e.to_string()))?;
        self.app
            .record_event("mock", format!("reset_state({scope}) → {detail}"));
        Ok(detail)
    }

    /// The registered seam ids (diagnostics).
    pub fn ids(&self) -> Vec<&str> {
        self.app.session.seam_ids()
    }

    /// Choose which registered seam `reset_state(scope)` runs (default
    /// convention: a seam named `"reset_state"`).
    pub fn with_reset_seam(&mut self, id: &str) -> &mut Self {
        self.app.session.with_reset_seam(id);
        self
    }
}

/// `app.video()` — the §10 video surface (M3).
pub struct VideoHandle<'a, A: FrameworkAdapter> {
    app: &'a mut App<A>,
}

impl<A: FrameworkAdapter> std::fmt::Debug for VideoHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoHandle").finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> VideoHandle<'_, A> {
    /// `video.start_in(dir, fps)` — record presented frames into `dir`
    /// (the client/test owns the artifact dir, Playwright parity). The
    /// adapter must expose a pixel path (§6.2); `fps = 0` selects the
    /// driver default (25).
    pub fn start_in(&mut self, dir: &std::path::Path, fps: u32) -> Result<(), KitError> {
        self.app.session.video_start_in(dir, fps)?;
        self.app
            .record_event("video", format!("start(dir={}, fps={fps})", dir.display()));
        Ok(())
    }

    /// `video.start(fps)` — record into the OS temp dir (the stop reply
    /// carries the finalized path).
    pub fn start(&mut self, fps: u32) -> Result<(), KitError> {
        self.app.session.video_start(fps)?;
        self.app.record_event("video", format!("start(fps={fps})"));
        Ok(())
    }

    /// `video.stop()` — finalize the recording; one last pump lands the
    /// latest requested frame before the encoder closes (§15.4).
    pub fn stop(&mut self) -> Result<guiwright_trace::video::VideoSummary, KitError> {
        let summary = self.app.session.video_stop()?;
        self.app.record_event(
            "video",
            format!(
                "stop → {} frames={} dropped={}",
                summary.path.display(),
                summary.frames,
                summary.dropped
            ),
        );
        Ok(summary)
    }
}

/// `expect(app.get_by_text("Welcome, Ada")).to_be_visible()` (§16
/// retry-until-timeout assertions). Free function, architecture shape:
/// it consumes the locator and returns the pending assertion. A
/// window-pinned locator (§16 v0.4) scopes the assertion's wait loop to
/// that window's subtree — the handle is re-validated at act time.
pub fn expect<A: FrameworkAdapter>(locator: Locator<'_, A>) -> core_expect::Pending<'_, A> {
    let Locator {
        app,
        selector,
        window,
    } = locator;
    core_expect::Pending {
        builder: app.session.expect(selector).with_window(window),
    }
}

/// Tiny namespace so the kit's expect reads like the architecture's
/// `expect(locator).to_*` while delegating to the session builder.
pub(crate) mod core_expect {
    use super::FrameworkAdapter;
    use guiwright_core::CoreError;
    use guiwright_core::selector::TextMatch;

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
    // assertion is single-shot, so `self` moves into the check (the
    // workspace lint table carries the `wrong_self_convention` carve-out).
    impl<A: FrameworkAdapter> Pending<'_, A> {
        /// Override the timeout for this assertion (the session builder's
        /// deadline — default 5 s, Playwright parity).
        pub fn with_timeout(self, timeout: std::time::Duration) -> Self {
            Self {
                builder: self.builder.with_timeout(timeout),
            }
        }
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
        pub fn to_be_checked(self) -> Result<(), CoreError> {
            self.builder.to_be_checked()
        }
        pub fn to_be_unchecked(self) -> Result<(), CoreError> {
            self.builder.to_be_unchecked()
        }
        pub fn to_be_checked_mixed(self) -> Result<(), CoreError> {
            self.builder.to_be_checked_mixed()
        }
        pub fn to_have_text(self, text: impl Into<TextMatch>) -> Result<(), CoreError> {
            self.builder.to_have_text(text)
        }
        pub fn to_have_value(self, value: impl Into<TextMatch>) -> Result<(), CoreError> {
            self.builder.to_have_value(value)
        }
    }
}
