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
use guiwright_core::actionability::Check;
use guiwright_core::selector::{Selector, TextMatch};
use guiwright_core::tree::Resolved;
use guiwright_core::{CoreError, NodeKey};
use guiwright_driver::ActionOpts;
use guiwright_driver::SeamRegistry;
use guiwright_driver::adapter::{AdapterError, ClockCmd, FrameworkAdapter, SemAction};
use guiwright_driver::clock::ClockReport;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::session::Session;
use guiwright_driver::{Gesture, codegen_call, codegen_line};

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
    /// [`Locator::input_value`] on a node that carries no value fact —
    /// the getter reads the value fact only (the label never answers,
    /// never a silent fallback); the message names the locator the
    /// strict resolve answered.
    #[error(
        "input_value: {0} carries no value fact (the getter reads the value fact only — the label never answers)"
    )]
    NoValue(String),
    /// The remote-to-self door's wire refusals ([`crate::remote`]):
    /// every typed `GwpError` the seam answers — the strict-mode
    /// rejection, the act-time `Timeout` naming the stuck check, the
    /// stale-generation/unknown-node boundary errors — surfaced as the
    /// kit's own error, never stringly-flattened.
    #[error(transparent)]
    Wire(#[from] guiwright_proto::GwpError),
    /// The live capture's viewer-side refusals ([`crate::live`]): the
    /// M18 feed's attach/pump errors that are not wire errors (the
    /// archive half's own surfaces), surfaced typed.
    #[error(transparent)]
    Viewer(#[from] guiwright_viewer::ViewerError),
    /// The remote-to-self door's own I/O ([`crate::remote`], the pump
    /// thread's runtime, the capture file write): the kit's honest
    /// std-io surface, surfaced typed, never flattened into a string.
    #[error(transparent)]
    Io(#[from] std::io::Error),
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

    /// `locator.filter({ hasNot })` narrowing (§16 locators row — the m34
    /// filter surface): keep only the matches whose **subtree** — the node
    /// included — contains no node matching `inner` (the inverted
    /// [`Self::has_text`], which reads the same subtree walk in the
    /// positive). The inner selector is a full [`Selector`]: a role+name
    /// constraint, a test id, a nested chain — a text probe only when
    /// built as one.
    ///
    /// The rule that makes the inversion honest: a leaf's subtree is
    /// itself, so `get_by_role(Role::Button, "Save").has_not(&Selector::
    /// role(Role::Button))` excludes Save — `has_not(button)` keeps only
    /// the containers that hold no button. Like every narrowing, it
    /// composes ([`Self::first`]/[`Self::nth`] wrap the composed selector
    /// in another filter, the resolver nests them), rides the window pin
    /// (§16 v0.4), and is consulted at act time by the same strict-mode
    /// resolve every locator action runs.
    pub fn has_not(mut self, inner: &Selector) -> Self {
        self.selector = self.selector.has_not(inner.clone());
        self
    }

    /// `locator.filter({ has })` narrowing (§16 locators row — the m34
    /// record's named follow-on): keep only the matches whose **subtree**
    /// — the node included — contains a node matching `inner` (the
    /// positive [`Self::has_not`], which reads the same subtree walk with
    /// the opposite verdict). Distinct from [`Self::locator`], which
    /// narrows to the *descendant's* rows — `has` keeps the *ancestor's*.
    /// The inner selector is a full [`Selector`]: a role+name constraint,
    /// a test id, a nested chain — a text probe only when built as one.
    ///
    /// The leaf rule carries over from the inverted filter: a leaf's
    /// subtree is itself, so `get_by_role(Role::Button,
    /// "Save").has(&Selector::role(Role::Button))` keeps Save — `has`
    /// and `has_not` read the same walk. Like every narrowing, it
    /// composes ([`Self::first`]/[`Self::nth`]/[`Self::has_text`]/
    /// [`Self::has_not`]/[`Self::locator`] nest by value, the resolver
    /// walks them), rides the window pin (§16 v0.4), and is consulted at
    /// act time by the same strict-mode resolve every locator action
    /// runs — an empty `has` answer is the valid snapshot read
    /// (`count()` answers 0), a failed act-time resolve is the
    /// strict-mode error naming the miss.
    pub fn has(mut self, inner: &Selector) -> Self {
        self.selector = self.selector.has(inner.clone());
        self
    }

    /// `locator.locator(child)` (§16 locators row — the m34 filter
    /// surface): the Playwright chained-descendant shape — resolve the
    /// locator's own set first, then narrow to `child` nodes inside each
    /// match's subtree, document order kept (`all`/`count` enumerate the
    /// same composed set). Chaining composes: `.locator(&a).locator(&b)`
    /// builds the `a >> b` chain, `first`/`nth`/`has_text`/`has_not` wrap
    /// it in filters, and the window pin scopes the whole composition.
    ///
    /// The child is a [`Selector`], not a locator value, and that is the
    /// borrow rule again (the [`Self::all`] and [`Self::drag_to`]
    /// doctrine): every locator holds the `&mut App` it acts through, so
    /// two locator values cannot coexist — the selector form composes by
    /// value. Actions through the chained locator ride the same
    /// strict-mode resolve + actionability gate as any locator action;
    /// an empty chain answer is the valid snapshot read (`count()`
    /// answers 0), a failed act-time resolve is the strict-mode error
    /// naming the miss.
    pub fn locator(mut self, child: &Selector) -> Self {
        self.selector = self.selector.then_descendant(child.clone());
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

    /// `locator.select_option(option)` (§16 actions row's `select` — the
    /// m19 record's named §16 follow-on, m89): the Playwright
    /// `locator.selectOption(...)` shape. The option rides by accessible
    /// name — the same string a text locator matches — through the same
    /// actionability gate, auto-wait and codegen recording every other
    /// locator action takes. The adapter translates per framework (§8.4):
    /// a11y-first stacks take the option as the choice widget's value,
    /// pixel-first stacks resolve the option row and click it; adapters
    /// whose framework cannot express the selection report the typed
    /// `Unsupported` the other vocabulary carries.
    pub fn select_option(
        self,
        option: &str,
    ) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(
                &selector,
                SemAction::SelectOption {
                    option: option.to_string(),
                },
                &ActionOpts::default(),
            )
        })?;
        app.record_scoped_action("select_option", &recorded, Some(option), window.as_deref());
        Ok(outcome)
    }

    /// `locator.scroll_into_view_if_needed()` (§16 actions row's
    /// scroll-into-view — the m19 record's named §16 follow-on, m90):
    /// the Playwright `locator.scrollIntoViewIfNeeded()` shape. The
    /// a11y-first honest form: the request rides the framework's own
    /// `ScrollIntoView` semantic where the stack carries it (egui,
    /// masonry and freya honor AccessKit's native action), through the
    /// same actionability gate, auto-wait and codegen recording every
    /// other locator action takes; adapters without the dispatch report
    /// the typed `Unsupported` the other vocabulary carries.
    pub fn scroll_into_view_if_needed(
        self,
    ) -> Result<guiwright_driver::adapter::ActionOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let recorded = selector.clone();
        let outcome = app.run_scoped(window.as_deref(), move |session| {
            session.perform_action(&selector, SemAction::ScrollIntoView, &ActionOpts::default())
        })?;
        app.record_scoped_action(
            "scroll_into_view_if_needed",
            &recorded,
            None,
            window.as_deref(),
        );
        Ok(outcome)
    }

    /// `locator.drag_to(target)` (§16 actions row — the m19 parity
    /// stream's named drag item, M33): the Playwright
    /// `locator.dragTo(target)` shape, adapted to the kit's
    /// borrow-and-act doctrine. Both ends resolve with strict mode and
    /// auto-wait in **one window context** — the source locator's own
    /// (its pin, §16 v0.4; unscoped locators aim the whole tree): each
    /// end waits `Attached` on the zero-polling loop (the wire
    /// `Resolve` discipline), then the drag runs from the source's
    /// bounds center to the target's — one [`Gesture::Drag`] through
    /// the session's synthesis, the adapter's pacing table ratcheting
    /// the pointer path (§9), one rendezvous pump applying it.
    ///
    /// The target is a [`Selector`], not a locator value, and that is
    /// the honest contract twice over: every locator holds the `&mut
    /// App` it acts through (the [`Self::all`] rule — two locator
    /// values, two mutable borrows of one app, cannot coexist), and a
    /// drag is one gesture in one window's coordinate space, so the aim
    /// resolves exactly where the source does. A cross-window drag has
    /// no coordinate answer here by construction: the aim resolves
    /// inside the source's subtree or it fails with the strict-mode
    /// `NoMatch` that names the miss.
    ///
    /// A drag needs geometry: the resolved bounds' centers steer it —
    /// the same coordinate space the Hover path feeds the input
    /// pipeline (§9: the adapter owns any scale conversion) — so a node
    /// without bounds is the honest §10 error, not an empty drag. A
    /// failed aim synthesizes no input (the drag never starts). The
    /// applied gesture records exactly like the raw-input surface (§13
    /// over §9): the raw `mouse.drag(...)` kit line while a codegen
    /// recording runs, plus one `drag_to` trace event naming both ends.
    /// The offset form — Playwright's `{ sourcePosition, targetPosition }`
    /// options — is [`Self::drag_to_with_positions`] (m33's honest-list
    /// widening); this method keeps the centers contract byte-identical.
    pub fn drag_to(self, target: &Selector) -> Result<guiwright_driver::GestureOutcome, KitError> {
        self.drag_to_with_positions(target, None, None)
    }

    /// `locator.dragTo(target, { sourcePosition, targetPosition })` —
    /// the offset drags (§16 actions row's drag, m33's honest-list
    /// widening). A position is a `(dx, dy)` offset from the resolved
    /// node's bounds top-left (the Playwright padding-box-corner shape,
    /// the bounds' own window-relative space): `Some` aims that exact
    /// point, `None` keeps the M33 center. Playwright validates nothing,
    /// and neither does this — an offset outside the bounds lands
    /// exactly where the suite aimed (no clamp, no silent snap); a node
    /// without bounds is still the honest §10 error. The synthesis is
    /// unchanged: the same auto-waiting resolves, the same one-gesture
    /// one-window discipline, the same §9 ratchet between the aimed
    /// points, the same codegen line (the coordinates it renders are the
    /// aimed points).
    pub fn drag_to_with_positions(
        self,
        target: &Selector,
        source_position: Option<(f64, f64)>,
        target_position: Option<(f64, f64)>,
    ) -> Result<guiwright_driver::GestureOutcome, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let deadline = std::time::Instant::now() + guiwright_core::expect::DEFAULT_TIMEOUT;
        // Both ends in the source's window context: the pin scopes the
        // aim exactly like the source (one gesture, one coordinate
        // space). The gate is Attached — arrival auto-waits, the same
        // discipline the wire's Resolve arm runs; geometry is the next
        // line's contract.
        let source = app
            .run_scoped(window.as_deref(), |session| {
                session.wait_for(&selector, vec![Check::Attached], deadline)
            })?
            .into_node()?;
        let (from_x, from_y) = drag_center(source.bounds, &selector, source_position)?;
        let resolved_target = app
            .run_scoped(window.as_deref(), |session| {
                session.wait_for(target, vec![Check::Attached], deadline)
            })?
            .into_node()?;
        let (to_x, to_y) = drag_center(resolved_target.bounds, target, target_position)?;

        let gesture = Gesture::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button: guiwright_driver::PointerButton::Primary,
        };
        // Rendered before the batch moves (the §13-over-§9 rule), landed
        // after the rendezvous pump confirms it.
        let rendered = if app.codegen_active() {
            Some(codegen_call(&gesture))
        } else {
            None
        };
        let outcome = app.session.perform_gesture(&gesture, window.as_deref())?;
        app.record_event(
            "drag_to",
            format!(
                "{} → {} ({})",
                selector.render(),
                target.render(),
                outcome.detail
            ),
        );
        if let Some((surface, call, warning)) = rendered {
            let line = codegen_line(surface, &call, window.as_deref());
            app.record_input_code(line, warning);
        }
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

    /// The node-describe read (§8.1's identity facts, M36 — the m32
    /// record's standing gap): resolve the locator (strict mode, the
    /// same surface every locator action aims through) and read the
    /// match's identity facts — role, accessible name, value, bounds.
    /// The debugging read beside the bulk readers: `all_text_contents()`
    /// (M29) answers "what does the set say", `describe()` answers "what
    /// IS the node I just resolved" — the row the wire's `Describe`
    /// domain (M36) serves remote clients. A read, not an action: like
    /// [`Self::resolve`], nothing is recorded.
    pub fn describe(self) -> Result<guiwright_driver::NodeDescription, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), move |session| {
            let resolved = session.resolve(&selector)?;
            session.describe(&resolved.key, resolved.generation)
        })?)
    }

    /// The identity-state read (§8.1's state facts at handle width, M68
    /// — the m68 record's named kit gap, closed): resolve the locator
    /// (strict mode, the same surface every locator read aims through)
    /// and read the match's state three beside the identity row —
    /// focused, enabled, the checked tri-state — the
    /// [`guiwright_driver::NodeStateDescription`] row the wire's
    /// `DescribeIdentityState` domain (M68) serves remote clients. The
    /// facts are total reads, never waits (the m68 rule): the vocabulary
    /// words answer right now — the waitable face stays the kit's
    /// `to_be_checked` / `to_be_focused` / `to_be_enabled` assertions,
    /// unchanged. A read, not an action: like [`Self::describe`],
    /// nothing is recorded.
    pub fn describe_state(self) -> Result<guiwright_driver::NodeStateDescription, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), move |session| {
            let resolved = session.resolve(&selector)?;
            session.describe_state(&resolved.key, resolved.generation)
        })?)
    }

    /// `locator.inner_text()` (§16 actions row — the m19 record's
    /// remaining-gaps list's content getter, cashed last of that list:
    /// `drag_to` rode m33, `wait_for(state)` m21, `select_option` m89
    /// and `scroll_into_view_if_needed` m90): resolve the locator (the
    /// [`Self::describe`] surface — strict mode, the auto-wait, the
    /// window pin) and read the match's text — the accessible label
    /// first, then the value (the `HasText` check's source order), the
    /// same derivation [`Self::all_text_contents`] applies per set row,
    /// now at single-node width. Where [`Self::input_value`] reads what
    /// the form *carries*, this reads what the text locators *match* —
    /// the fixture's Email input reads `Email` here and
    /// `ada@example.dev` there, and the two reads can never silently
    /// agree on the wrong datum. A node with neither fact reads the
    /// empty string (the m29 totality rule at single-node width). A
    /// read, not an action: like [`Self::describe`], nothing is
    /// recorded — no codegen line, no trace action (the trace's tree
    /// snapshots carry the same facts).
    ///
    /// The a11y substrate carries no arbitrary attribute vocabulary, so
    /// the m19 list's third getter (`get_attribute`) stays the honest
    /// cap: the facts DOM attributes feed are the identity reads'
    /// name/value/description, which [`Self::describe`] already
    /// answers.
    pub fn inner_text(self) -> Result<String, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), move |session| {
            let resolved = session.resolve(&selector)?;
            let tree = session.tree();
            Ok(tree
                .node(&resolved.key)
                .and_then(|n| n.label().or_else(|| n.value()))
                .unwrap_or_default()
                .clone())
        })?)
    }

    /// `locator.input_value()` (§16 actions row — the m19 record's
    /// remaining-gaps list's content getter, beside
    /// [`Self::inner_text`]): resolve the locator (the [`Self::describe`]
    /// surface — strict mode, the auto-wait, the window pin) and read
    /// what the form *carries* — the value fact only, never the label
    /// fallback ([`Self::all_values`]'s flip is the bulk read's shape,
    /// not the single-node getter's; a button's name is not a value).
    ///
    /// The refusal is the typed one: a node that carries no value fact
    /// ([`KitError::NoValue`]) names the locator the strict resolve
    /// answered — Playwright's `inputValue()` throws for non-form
    /// elements, and a silent empty string would read as "the form
    /// carries nothing", which the tree's missing fact never claimed.
    /// A read, not an action: nothing is recorded, the
    /// [`Self::inner_text`] rule unchanged.
    pub fn input_value(self) -> Result<String, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let locator = selector.render();
        let value = app.run_scoped(window.as_deref(), move |session| {
            let resolved = session.resolve(&selector)?;
            let tree = session.tree();
            Ok(tree.node(&resolved.key).and_then(|n| n.value()))
        })?;
        value.map_or_else(|| Err(KitError::NoValue(locator)), |v| Ok(v.clone()))
    }

    /// `locator.count()` — how many nodes match right now (§16
    /// assertions row, M22): the non-strict read beside
    /// `to_have_count`'s waitable assertion (Playwright parity). A read,
    /// not an action: like [`Self::resolve`], nothing is recorded —
    /// resolution without an action is not a replayable step. A
    /// window-pinned locator counts only inside its subtree.
    pub fn count(self) -> Result<usize, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), move |session| {
            session.resolve_all(&selector).map(|all| all.len())
        })?)
    }

    /// `locator.all(visit)` (§16 assertions row — the m22/m25 records'
    /// open kit gap, closed here): enumerate the locator's current match
    /// set in document order, handing each match to `visit` as an
    /// nth-pinned [`Locator`] — Playwright's `Locator.all()`, adapted to
    /// the kit's borrow-and-act shape.
    ///
    /// The borrow shape is the design decision: a Playwright-style
    /// `Vec<Locator>` cannot exist here because every locator holds the
    /// `&mut App` it acts through — two locators would be two mutable
    /// borrows of one app. So the matches arrive **one at a time**: the
    /// visitor receives each item, acts on it (or maps it to data), and
    /// only then sees the next. The generic return collects per-item
    /// data instead (`all(|item| Ok(item.resolve()?.key))` is the
    /// whole-set read).
    ///
    /// Semantics (Playwright parity):
    ///
    /// * the set is a **snapshot read** like [`Self::count`] — the same
    ///   `resolve_all` walk, no waiting: an empty set enumerates to zero
    ///   visits, and a node that arrives later shows up on the next
    ///   enumeration;
    /// * each item is `locator.nth(i)` — the visited locator re-resolves
    ///   at act time (auto-waits like any locator), so a match removed
    ///   mid-iteration fails that item's action instead of visiting a
    ///   stale node;
    /// * a locator that already carries narrowing (`.first()`,
    ///   `.nth(2)`, `.has_text(...)`) enumerates its own narrowed set —
    ///   the same set [`Self::count`] reads;
    /// * a window-pinned locator enumerates only inside its subtree (the
    ///   pin rides every item).
    ///
    /// Enumeration itself is a read: nothing is recorded (the `count()`
    /// rule — resolution without an action is not a replayable step).
    /// Actions on the visited items record exactly like the equivalent
    /// hand-written `.nth(i)` locator actions.
    pub fn all<T>(
        self,
        mut visit: impl FnMut(Locator<'_, A>) -> Result<T, KitError>,
    ) -> Result<Vec<T>, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        // The snapshot: one scoped resolve_all walk — the `count()` read
        // generalized to identities.
        let matches =
            app.run_scoped(window.as_deref(), |session| session.resolve_all(&selector))?;
        let mut collected = Vec::with_capacity(matches.len());
        for index in 0..matches.len() {
            // Each visit reborrows the app: the previous item's borrow
            // ended with its call, so the next one re-aims cleanly.
            let item = Locator {
                app: &mut *app,
                selector: selector.clone().nth(index),
                window: window.clone(),
            };
            collected.push(visit(item)?);
        }
        Ok(collected)
    }

    /// `locator.all_text_contents()` (§16 assertions row — the m26
    /// record's open kit gap: "bulk *content* reads … a dedicated bulk
    /// reader lands when a suite asks for it"): read the text of every
    /// match in document order as one snapshot — the [`Self::all`]
    /// enumeration's set read mapped to data instead of visitors, the
    /// same non-strict `resolve_all` walk [`Self::count`] takes.
    ///
    /// Semantics (the snapshot-read family rules):
    ///
    /// * the set is a **snapshot read**: an empty set reads to an empty
    ///   `Vec`, and a node that arrives later joins the next read (no
    ///   waiting — [`Locator::wait_for`] or an assertion covers that);
    /// * each match contributes the text the text locators match — the
    ///   accessible label first, then the value (the `HasText` check's
    ///   source order), so `get_by_text(t)` matches imply `t` appears in
    ///   this read; a node with neither reads as an empty string, keeping
    ///   positions stable;
    /// * hidden nodes are included (the `all()`/`count()` set semantics —
    ///   nothing filters the walk);
    /// * a window-pinned locator reads only inside its subtree (the pin
    ///   rides the walk, §16 v0.4).
    ///
    /// Like [`Self::count`], the read is not a replayable step: nothing
    /// is recorded — no codegen line, no trace action (the trace's tree
    /// snapshots carry the same facts). Actions on a [`Self::all`] visitor
    /// remain the way to record per-item steps.
    pub fn all_text_contents(self) -> Result<Vec<String>, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let matches =
            app.run_scoped(window.as_deref(), |session| session.resolve_all(&selector))?;
        let tree = app.session.tree();
        Ok(matches
            .iter()
            .map(|m| {
                tree.node(&m.key)
                    .and_then(|n| n.label().or_else(|| n.value()))
                    .unwrap_or_default()
            })
            .collect())
    }

    /// `locator.describe_all()` (§8.1's identity facts over the set read,
    /// M39 — the m37 record's standing gap: "enumerating a set's facts
    /// would open one `Describe` round trip per row"): the
    /// [`Self::all_text_contents`] walk's rows carrying each match's
    /// identity facts — role, accessible name, value, bounds — one pass
    /// instead of one [`Self::describe`] round trip per row. The
    /// debugging read beside the bulk readers: `all_text_contents()`
    /// answers "what does the set say", `describe_all()` answers "what
    /// IS everything in the set" — the row the wire's `DescribeAll`
    /// domain (M39) serves remote clients. Snapshot semantics (the
    /// `count()`/`all()` family rule): no strict mode, no waiting, an
    /// empty set answers with an empty list, a window-pinned locator
    /// reads only inside its subtree, and a read records nothing.
    pub fn describe_all(self) -> Result<Vec<guiwright_driver::NodeDescription>, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), |session| session.describe_all(&selector))?)
    }

    /// `locator.describe_all_state()` (§8.1's state facts over the set
    /// read, M70 — the m68/m70 records' named kit gap, closed): the
    /// [`Self::describe_all`] walk's rows carrying each match's state
    /// three beside the identity row — one pass instead of one
    /// [`Self::describe_state`] round trip per row. The debugging read
    /// beside the bulk readers: `describe_all()` answers "what IS
    /// everything in the set", `describe_all_state()` answers "what is
    /// everything in the set **doing**" — the rows the wire's
    /// `DescribeAllIdentityState` domain (M70) serves remote clients.
    /// Snapshot semantics (the `describe_all` family rule): no strict
    /// mode, no waiting, an empty set answers with an empty list, a
    /// window-pinned locator reads only inside its subtree, and a read
    /// records nothing.
    pub fn describe_all_state(
        self,
    ) -> Result<Vec<guiwright_driver::NodeStateDescription>, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        Ok(app.run_scoped(window.as_deref(), |session| {
            session.describe_all_state(&selector)
        })?)
    }

    /// `locator.all_values()` (§16 assertions row — the m29 record's
    /// named follow-up: "`all_values()` (a value-first bulk read for
    /// form dumps) is a one-line variation of the same walk —
    /// deliberately not added until a suite asks for it"): read the
    /// value of every match in document order as one snapshot — the
    /// [`Self::all_text_contents`] read with the source order flipped
    /// for form semantics.
    ///
    /// The flip is the whole point: `all_text_contents` reads what the
    /// *text locators* match (label first — the `HasText` check's
    /// source order), while a form dump wants what the form *carries*.
    /// Each match contributes its value first, falling back to the
    /// label — the `getByValue` source order — so the fixture's Email
    /// input reads `ada@example.dev` here and `Email` there, and the
    /// two reads can never silently agree on the wrong datum.
    ///
    /// Everything else is the snapshot-read family rules, unchanged:
    ///
    /// * the set is the same non-strict `resolve_all` walk (no strict
    ///   mode, no waiting — [`Locator::wait_for`] or an assertion
    ///   covers arrivals);
    /// * hidden nodes are included, a window-pinned locator reads only
    ///   inside its subtree, narrowing scopes the read;
    /// * a node with neither value nor label reads as an empty string,
    ///   keeping `Vec` positions 1:1 with the document order the set
    ///   read established.
    ///
    /// Like every read, nothing is recorded (no codegen line, no trace
    /// action — the trace's tree snapshots carry the same facts).
    pub fn all_values(self) -> Result<Vec<String>, KitError> {
        let Self {
            app,
            selector,
            window,
        } = self;
        let matches =
            app.run_scoped(window.as_deref(), |session| session.resolve_all(&selector))?;
        let tree = app.session.tree();
        Ok(matches
            .iter()
            .map(|m| {
                tree.node(&m.key)
                    .and_then(|n| n.value().or_else(|| n.label()))
                    .unwrap_or_default()
            })
            .collect())
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
        // bounds are already physical; the frame is the window's buffer)
        // — the shared §10 crop helper (the retry waiter's poll rides
        // the same one, [`App::poll_element_snapshot`]).
        let ((x0, y0, x1, y1), element) = crop_element(rect, frame.size.0, frame.size.1, &pixels)?;

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
        self.start_trace(path, None);
    }

    /// The shared trace start (the window handle's scoped surface rides
    /// it with the tag set, the m6 §5/m7 §4 deferred half).
    pub(crate) fn start_trace(&mut self, path: impl Into<PathBuf>, window: Option<String>) {
        if self.trace.is_some() {
            return;
        }
        let path = path.into();
        let mut options = TraceOptions::new(
            path.clone(),
            &self.context.test_name,
            AdapterRecord {
                name: self.session.adapter().meta().name.to_string(),
                framework: self.session.adapter().meta().framework.to_string(),
                framework_version: self.session.adapter().meta().framework_version.clone(),
            },
        );
        if let Some(handle) = window {
            options = options.with_window(handle);
        }
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
        VideoHandle::scoped(self, None)
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
        let archive = guiwright_viewer::TraceArchive::open(&trace_path)?;
        let timeline = guiwright_viewer::Timeline::from_archive(&archive);
        let html = guiwright_viewer::render_html(&archive, &timeline);
        let out = out_path.into();
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", parent.display())))?;
        }
        std::fs::write(&out, html)
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", out.display())))?;
        Ok(out)
    }

    /// `app.key_of(&selector)` helper's sibling read (M32): the hit-test
    /// read (§8.3's "receives events" mechanism as a public read) — which
    /// node occupies the point. Coordinates are window-relative physical
    /// px, the same space the tree's bounds speak (the resolve rects, the
    /// dump rows' bounds), so a test can hit inside a rect it just
    /// resolved. The whole-tree surface: a hit in any window's subtree
    /// answers; aim one window with [`App::window`] +
    /// [`WindowHandle::hit_test`]. A miss answers `None` — the read is
    /// total, never an error; the hit is the same [`Resolved`] record the
    /// resolution pipeline returns (identity + generation + bounds),
    /// directly usable as a pre-resolved action handle. Nothing is
    /// recorded: like [`Locator::resolve`], a read without an action is
    /// not a replayable step.
    pub fn hit_test(&mut self, x: f64, y: f64) -> Result<Option<Resolved>, KitError> {
        Ok(self.session.hit_test(x, y)?)
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

    /// One snapshot/diff poll for the §16 screenshot retry waiter — the
    /// `Locator::screenshot` pipeline (resolve → render → crop → diff)
    /// in its waitable form: `Ok(Mismatched)` means "keep polling" (the
    /// artifacts are already written next to the baseline, §10), while
    /// `Matched`/`Blessed` satisfy the pixel budget. `Err` is fail-fast:
    /// strict-mode resolution, a bounds-less node, an adapter without a
    /// pixel path, a missing baseline without bless — no amount of
    /// waiting fixes those. The resolve rides the caller's window scope
    /// (the set/act/clear rendezvous the `expect` waiter drives), and
    /// the crop is the same [`crop_element`] helper the single-shot
    /// uses, so the two surfaces crop byte-identically.
    pub(crate) fn poll_element_snapshot(
        &mut self,
        selector: &Selector,
        name: &str,
        store: &guiwright_shot::baseline::BaselineStore,
        tolerance: guiwright_shot::config::EffectiveTolerance,
        bless: BlessMode,
    ) -> Result<SnapshotOutcome, KitError> {
        let resolved = self.session.resolve(selector)?;
        let Some(rect) = resolved.bounds else {
            return Err(KitError::Core(CoreError::other(format!(
                "{} has no bounds — nothing to crop (§10)",
                selector.render()
            ))));
        };
        let frame = self.session.adapter_mut().render_frame(None)?;
        let Some(pixels) = frame.pixels.as_deref() else {
            return Err(KitError::NoPixels(
                self.session.adapter().meta().name.to_string(),
            ));
        };
        let (_, element) = crop_element(rect, frame.size.0, frame.size.1, pixels)?;
        store
            .check_snapshot(
                name,
                self.session.adapter().meta().framework,
                &element,
                tolerance,
                bless,
            )
            .map_err(KitError::Shot)
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
                // A scope that no longer resolves must not silently drop
                // the codegen line: the recorder warns (the same honesty
                // rule the trace log-tail follows — failures surface, they
                // are not swallowed), and the unscoped line still records.
                Some(handle) => match self.session.window_scope(handle) {
                    Ok(scope) => scope,
                    Err(err) => {
                        recorder.warn(format!(
                            "codegen scope `{handle}` no longer resolves ({err}); \
                             recording the line unscoped"
                        ));
                        None
                    }
                },
            };
            let pinned = window.as_deref().zip(scope.as_ref());
            let facts = self.collect_facts(&resolved.key, resolved.bounds);
            // The same rule for the record itself: a rejected line is
            // warned about, never dropped silently.
            if let Err(err) = recorder.record(
                kind,
                argument.as_deref(),
                &facts,
                self.session.tree(),
                pinned,
            ) {
                recorder.warn(format!("codegen line for `{kind}` was not recorded: {err}"));
            }
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
        // The id and scope name the failure at this layer — the inner
        // error carries the driver's detail (the registered-ids list, the
        // hook's own message); a caller of a ten-seam suite should not
        // have to guess which call failed.
        let detail =
            self.app.session.run_seam(id, scope).map_err(|e| {
                KitError::Mock(format!("seam `{id}` failed (scope `{scope}`): {e}"))
            })?;
        self.app
            .record_event("mock", format!("call {id}({scope}) → {detail}"));
        Ok(detail)
    }

    /// `mock.reset_state(scope)` — fixture parity between tests (§14).
    pub fn reset_state(&mut self, scope: &str) -> Result<String, KitError> {
        let detail =
            self.app.session.reset_state(scope).map_err(|e| {
                KitError::Mock(format!("reset_state failed (scope `{scope}`): {e}"))
            })?;
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

/// `app.video()` — the §10 video surface (M3). Born from a window
/// handle (`app.window(h)?.video()`), the recording rides that window's
/// scope: every presented frame crops to the window subtree's bounds and
/// a change outside the scope sheds its frames (the m6 §5/m7 §4 deferred
/// half). `App::video()` stays the whole-session surface.
pub struct VideoHandle<'a, A: FrameworkAdapter> {
    app: &'a mut App<A>,
    window: Option<String>,
}

impl<A: FrameworkAdapter> std::fmt::Debug for VideoHandle<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoHandle").finish_non_exhaustive()
    }
}

impl<A: FrameworkAdapter> VideoHandle<'_, A> {
    /// The constructor surface (the window handle's scoped half rides it;
    /// fields stay private to the kit).
    pub(crate) fn scoped(app: &'_ mut App<A>, window: Option<String>) -> VideoHandle<'_, A> {
        VideoHandle { app, window }
    }

    /// `video.start_in(dir, fps)` — record presented frames into `dir`
    /// (the client/test owns the artifact dir, Playwright parity). The
    /// adapter must expose a pixel path (§6.2); `fps = 0` selects the
    /// driver default (25). A window-scoped handle records that window's
    /// stream: the scope is validated at start (a dead handle is the
    /// typed `UnknownWindow`, never a mid-stream failure).
    pub fn start_in(&mut self, dir: &std::path::Path, fps: u32) -> Result<(), KitError> {
        self.app
            .session
            .video_start_scoped(dir, fps, self.window.as_deref())?;
        let scope = self
            .window
            .as_ref()
            .map(|handle| format!(" @ window {handle}"))
            .unwrap_or_default();
        self.app.record_event(
            "video",
            format!("start(dir={}, fps={fps}){scope}", dir.display()),
        );
        Ok(())
    }

    /// `video.start(fps)` — record into the OS temp dir (the stop reply
    /// carries the finalized path). The window scope rides, same as
    /// [`Self::start_in`].
    pub fn start(&mut self, fps: u32) -> Result<(), KitError> {
        match self.window.as_deref() {
            None => self.app.session.video_start(fps)?,
            Some(handle) => self.app.session.video_start_window(fps, handle)?,
        }
        let scope = self
            .window
            .as_ref()
            .map(|handle| format!(" @ window {handle}"))
            .unwrap_or_default();
        self.app
            .record_event("video", format!("start(fps={fps}){scope}"));
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

/// The drag endpoint's aim point (M33, widened by m33's honest-list
/// offsets): the resolved bounds' midpoint, or the bounds top-left plus
/// the asked `(dx, dy)` offset when the suite aims a point (the
/// Playwright padding-box-corner shape — validated nowhere, carried as
/// asked). An unbounded node is an honest error, not an empty drag (the
/// §10 element-crop rule's shape: "no bounds — nothing to crop" reads
/// the same way here).
fn drag_center(
    bounds: Option<accesskit::Rect>,
    selector: &Selector,
    offset: Option<(f64, f64)>,
) -> Result<(f64, f64), KitError> {
    let rect = bounds.ok_or_else(|| {
        KitError::Core(CoreError::other(format!(
            "{} has no bounds — nowhere to drag (§10)",
            selector.render()
        )))
    })?;
    Ok(match offset {
        None => (rect.x0.midpoint(rect.x1), rect.y0.midpoint(rect.y1)),
        Some((dx, dy)) => (rect.x0 + dx, rect.y0 + dy),
    })
}

/// The clamped §10 crop window in physical px (`x0, y0, x1, y1`) — the
/// trace event's coordinates.
type CropWindow = (u32, u32, u32, u32);

/// The §10 element crop: the window frame's pixels clipped to the
/// resolved node's rect — never a re-render. Returns the clamped crop
/// window (the trace event's coordinates) and the element image. The
/// clamp pins every coordinate into `[0, max]` before the conversion
/// (the §10/§15 guarded-conversion pattern): the float→i64 step keeps
/// the sign domain (clamped ≥ 0), and the `u32::try_from` proves the
/// range instead of truncating — `unwrap_or(max)` is the unreachable
/// arm, kept honest by the clamp. Shared by [`Locator::screenshot`] and
/// the §16 retry waiter's poll ([`App::poll_element_snapshot`]) — the
/// two surfaces crop byte-identically by construction.
fn crop_element(
    rect: accesskit::Rect,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(CropWindow, Image), KitError> {
    let clamp = |v: f64, max: u32| {
        u32::try_from(v.round().clamp(0.0, f64::from(max)) as i64).unwrap_or(max)
    };
    let (x0, y0) = (clamp(rect.x0, width), clamp(rect.y0, height));
    let (x1, y1) = (clamp(rect.x1, width), clamp(rect.y1, height));
    let image = Image {
        width,
        height,
        rgba: pixels.to_vec(),
    };
    let element = guiwright_shot::crop(&image, x0, y0, x1, y1)?;
    Ok(((x0, y0, x1, y1), element))
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
        app,
        selector,
        window,
        timeout: guiwright_core::expect::DEFAULT_TIMEOUT,
        negated: false,
    }
}

/// Tiny namespace so the kit's expect reads like the architecture's
/// `expect(locator).to_*` while delegating to the session builder.
pub(crate) mod core_expect {
    use std::time::Duration;

    use guiwright_core::CoreError;
    use guiwright_core::selector::TextMatch;

    use super::{App, FrameworkAdapter, SnapshotOutcome};
    use guiwright_driver::session::ExpectBuilder;
    use guiwright_shot::baseline::BlessMode;

    /// The pending `expect(locator)` assertion (§16). Carries the
    /// locator's app/selector/window plus the two builder modifiers
    /// (`with_timeout`, `not`) so every terminal `to_*` builds the
    /// session's [`ExpectBuilder`] on demand — and so the pixel-domain
    /// assertion (`to_have_screenshot`) can drive the app-level §10
    /// baseline store the session builder's check-list waiter cannot
    /// reach.
    pub struct Pending<'a, A: FrameworkAdapter> {
        pub(crate) app: &'a mut App<A>,
        pub(crate) selector: super::Selector,
        pub(crate) window: Option<String>,
        pub(crate) timeout: Duration,
        pub(crate) negated: bool,
    }

    impl<A: FrameworkAdapter> std::fmt::Debug for Pending<'_, A> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Pending")
                .field("selector", &self.selector.render())
                .field("negated", &self.negated)
                .finish_non_exhaustive()
        }
    }

    // Consuming builder by design — mirrors session::ExpectBuilder and the
    // architecture's `expect(locator).to_be_visible()` shape (§16): the
    // assertion is single-shot, so `self` moves into the check (the
    // workspace lint table carries the `wrong_self_convention` carve-out).
    impl<A: FrameworkAdapter> Pending<'_, A> {
        /// The session builder for this assertion's tree-domain checks:
        /// the stored selector/window/timeout/negation replayed onto a
        /// fresh builder (the terminal method's `self` moves into it).
        fn builder(&mut self) -> ExpectBuilder<'_, A> {
            let builder = self
                .app
                .session
                .expect(self.selector.clone())
                .with_timeout(self.timeout)
                .with_window(self.window.clone());
            if self.negated { builder.not() } else { builder }
        }

        /// Override the timeout for this assertion (the session builder's
        /// deadline — default 5 s, Playwright parity).
        pub fn with_timeout(mut self, timeout: Duration) -> Self {
            self.timeout = timeout;
            self
        }
        /// The negation modifier (§16 assertions row, M24):
        /// `expect(locator).not().to_be_visible()` — the terminal
        /// assertion is inverted through the core `NotWaiter` decorator:
        /// satisfied exactly while the positive condition does not hold
        /// (absent, hidden, disabled, a count that differs — or, for the
        /// screenshot waiter, a pixel budget that does not hold),
        /// completing event-driven when the world changes. Strict-mode
        /// violations and fail-fast checks keep failing (Playwright
        /// parity), and the timeout header names the inversion. A
        /// window-pinned locator keeps aiming through the modifier.
        /// Set-once: calling it again changes nothing.
        pub fn not(mut self) -> Self {
            self.negated = true;
            self
        }
        pub fn to_be_visible(mut self) -> Result<(), CoreError> {
            self.builder().to_be_visible()
        }
        pub fn to_be_hidden(mut self) -> Result<(), CoreError> {
            self.builder().to_be_hidden()
        }
        pub fn to_be_enabled(mut self) -> Result<(), CoreError> {
            self.builder().to_be_enabled()
        }
        pub fn to_be_disabled(mut self) -> Result<(), CoreError> {
            self.builder().to_be_disabled()
        }
        pub fn to_be_focused(mut self) -> Result<(), CoreError> {
            self.builder().to_be_focused()
        }
        pub fn to_be_checked(mut self) -> Result<(), CoreError> {
            self.builder().to_be_checked()
        }
        pub fn to_be_unchecked(mut self) -> Result<(), CoreError> {
            self.builder().to_be_unchecked()
        }
        pub fn to_be_checked_mixed(mut self) -> Result<(), CoreError> {
            self.builder().to_be_checked_mixed()
        }
        /// `expect(locator).toBeEditable()` (§16 assertions row, M28): the
        /// consumer's effective read-only state reports editable — the
        /// AccessKit `read_only` flag plus the role rules the consumer
        /// applies. The mismatch is waitable (Playwright parity: an input
        /// that has not flipped yet stays Pending), the timeout error
        /// names the assertion, the locator and the state the last
        /// evaluation found. Composes with [`Self::not`].
        pub fn to_be_editable(mut self) -> Result<(), CoreError> {
            self.builder().to_be_editable()
        }
        /// `expect(locator).to_be_readonly()` (§16 assertions row, M28) —
        /// the editable check's negated pair as a direct assertion: the
        /// same effective state, so a button (no editable value) reads
        /// read-only exactly as Playwright's
        /// `expect(button).not().toBeEditable()` does. Composes with
        /// [`Self::not`].
        pub fn to_be_readonly(mut self) -> Result<(), CoreError> {
            self.builder().to_be_readonly()
        }
        pub fn to_have_text(mut self, text: impl Into<TextMatch>) -> Result<(), CoreError> {
            self.builder().to_have_text(text)
        }
        pub fn to_have_value(mut self, value: impl Into<TextMatch>) -> Result<(), CoreError> {
            self.builder().to_have_value(value)
        }
        /// `expect(locator).toBeEmpty()` (§16 assertions row, m28's
        /// honest-list row) — the label/value composite: the node's own
        /// label *and* value must both read absent-or-empty, the same
        /// node reads `to_have_text` makes (never a subtree walk — the
        /// m29/m31 reads' honest node-level scope). The mismatch is
        /// waitable (text arrives with the node's restatement, Playwright
        /// parity), the timeout error names the assertion, the locator
        /// and the facts the last evaluation found. Composes with
        /// [`Self::not`].
        pub fn to_be_empty(mut self) -> Result<(), CoreError> {
            self.builder().to_be_empty()
        }
        /// `expect(locator).toHaveRole(role)` (§16 assertions row, M38) —
        /// the identity family's role row: the node's AccessKit role
        /// equals the wanted role. The mismatch is waitable (a re-render
        /// can restate the role, Playwright parity); the timeout error
        /// names the assertion, the locator and the role the last
        /// evaluation found. Composes with [`Self::not`].
        pub fn to_have_role(mut self, role: accesskit::Role) -> Result<(), CoreError> {
            self.builder().to_have_role(role)
        }
        /// `expect(locator).toHaveAccessibleName(name)` (§16 assertions
        /// row, M38) — the identity family's name row: the node's
        /// accessible name (`label()`) matches, deliberately narrower
        /// than [`Self::to_have_text`] (label-or-value): a text input's
        /// *value* never satisfies a name assertion. The mismatch is
        /// waitable; a missing name stays Pending (labels arrive with
        /// the node's restatement). Composes with [`Self::not`].
        pub fn to_have_accessible_name(
            mut self,
            name: impl Into<TextMatch>,
        ) -> Result<(), CoreError> {
            self.builder().to_have_accessible_name(name)
        }
        /// `expect(locator).toHaveAccessibleDescription(description)` (§16
        /// assertions row, M38) — the identity family's description row:
        /// the node's accessible description (`description()`) matches.
        /// The mismatch is waitable; a missing description stays Pending
        /// (descriptions arrive with the node's restatement). Composes
        /// with [`Self::not`].
        pub fn to_have_accessible_description(
            mut self,
            description: impl Into<TextMatch>,
        ) -> Result<(), CoreError> {
            self.builder().to_have_accessible_description(description)
        }
        /// `expect(locator).toHaveCount(count)` (§16 assertions row, M22) —
        /// the selector-level assertion: how many nodes match, strict mode
        /// never consulted. The mismatch is waitable (nodes stream in and
        /// out, Playwright parity); the timeout error names the expected
        /// count, the locator and the count the last evaluation found.
        pub fn to_have_count(mut self, expected: usize) -> Result<(), CoreError> {
            self.builder().to_have_count(expected)
        }
        /// `expect(locator).to_have_count_greater_than(min)` (§16
        /// assertions row, M25) — the M22 record's reserved widening on
        /// the count waiter: satisfied while *more* than `min` nodes
        /// match, the mismatch waitable, the timeout error naming the
        /// comparator (`count > 99 — …`) and the count the last
        /// evaluation found (`found 4`). Composes with [`Self::not`].
        pub fn to_have_count_greater_than(mut self, min: usize) -> Result<(), CoreError> {
            self.builder().to_have_count_greater_than(min)
        }
        /// `expect(locator).to_have_count_less_than(max)` (§16
        /// assertions row, M25) — the mirror comparator: satisfied while
        /// *fewer* than `max` nodes match, same waiter, same waitable
        /// mismatch, same error shape. Composes with [`Self::not`].
        pub fn to_have_count_less_than(mut self, max: usize) -> Result<(), CoreError> {
            self.builder().to_have_count_less_than(max)
        }
        /// `expect(locator).to_have_count_at_least(min)` (m25's
        /// honest-list widening) — the reserved inclusive lower bound:
        /// satisfied while `min` *or more* nodes match, the bound itself
        /// included, same waiter, same waitable mismatch, same error
        /// shape (`count >= 99 — …`). Composes with [`Self::not`].
        pub fn to_have_count_at_least(mut self, min: usize) -> Result<(), CoreError> {
            self.builder().to_have_count_at_least(min)
        }
        /// `expect(locator).to_have_count_at_most(max)` (m25's
        /// honest-list widening) — the reserved inclusive upper bound:
        /// satisfied while `max` *or fewer* nodes match, the bound
        /// itself included, same waiter, same waitable mismatch, same
        /// error shape (`count <= 3 — …`). Composes with [`Self::not`].
        pub fn to_have_count_at_most(mut self, max: usize) -> Result<(), CoreError> {
            self.builder().to_have_count_at_most(max)
        }
        /// `expect(locator).to_have_count_between(min, max)` (m25's
        /// honest-list widening, the range shape the same honest list
        /// names beside the inclusive pair) — satisfied while `min`
        /// *through* `max` nodes match, both ends inclusive, same
        /// waiter, same waitable mismatch, same error shape
        /// (`count 3..=5 — …`). Composes with [`Self::not`].
        pub fn to_have_count_between(mut self, min: usize, max: usize) -> Result<(), CoreError> {
            self.builder().to_have_count_between(min, max)
        }

        /// `expect(locator).to_have_screenshot(name, bless)` — the §16
        /// retry waiter over the §10 baseline flow (Playwright
        /// `toHaveScreenshot` parity, the single-shot
        /// [`Locator::screenshot`] with the `to_have_text` retry
        /// semantics): poll resolve → render → crop → diff on every
        /// presented frame until the pixel budget is satisfied or the
        /// `expect` timeout expires. Each mismatching poll writes the
        /// §10 artifacts (`.new.png` + `.diff.png` next to the baseline),
        /// and the timeout error's header carries the LAST attempt's
        /// report — the artifact links ride the failure the same way
        /// they ride the single-shot error.
        ///
        /// Zero-polling where it can be (§15.3): the first evaluation
        /// runs before any pump — a first-frame match never pumps at all
        /// — and every later poll follows a pump, so a still app is never
        /// re-diffed into a busy loop by anything but the shared
        /// deadline. Fail-fast where waiting cannot help: strict-mode
        /// resolution, a bounds-less node, an adapter without a pixel
        /// path, a missing baseline without bless. `not()` inverts the
        /// terminal condition (the m24 semantics over pixels): satisfied
        /// exactly while the budget does NOT hold; the bless flow rides
        /// the store unchanged (a `Failing`/`Force` bless rewrites the
        /// baseline on the poll that mismatches, which short-circuits
        /// the retry — bless is a baseline-churn escape hatch, not an
        /// assertion mode).
        pub fn to_have_screenshot(self, name: &str, bless: BlessMode) -> Result<(), CoreError> {
            let Pending {
                app,
                selector,
                window,
                timeout,
                negated,
            } = self;
            let recorded = selector.clone();
            let deadline = std::time::Instant::now() + timeout;
            // The scope resolves at act time (the M17 discipline): the
            // tree is the source of truth, so a window closed between aim
            // and assert fails here instead of resolving across windows.
            let scope = match &window {
                None => None,
                Some(handle) => app.session.window_scope(handle)?,
            };
            app.session.set_resolve_scope(scope);
            let header = if negated {
                format!("not (screenshot `{name}` — {})", recorded.render())
            } else {
                format!("screenshot `{name}` — {}", recorded.render())
            };
            let tolerance = app.config.screenshots.effective();
            let root = app
                .baseline_root
                .clone()
                .unwrap_or_else(|| app.context.baseline_dir());
            let store = guiwright_shot::baseline::BaselineStore::new(root);
            // The M17 stuck-on suffix: the LAST poll's report rides the
            // timeout header (minus the prefix the header already
            // carries) so the failure deep-links the artifacts. Empty
            // until a poll leaves the budget unsatisfied (the negated
            // form's "still matching" case).
            let mut stuck;
            let result = loop {
                match app.poll_element_snapshot(&selector, name, &store, tolerance, bless) {
                    Ok(SnapshotOutcome::Mismatched { report }) => {
                        if negated {
                            // The positive form failing IS the
                            // inversion's satisfied state (the m24
                            // semantics over pixels).
                            break Ok(());
                        }
                        let prefix = format!("screenshot `{name}` mismatch: ");
                        stuck = report.strip_prefix(&prefix).unwrap_or(&report).to_string();
                    }
                    Ok(SnapshotOutcome::Matched { .. } | SnapshotOutcome::Blessed { .. }) => {
                        if negated {
                            stuck = "still matching (pixel budget satisfied)".to_string();
                        } else {
                            break Ok(());
                        }
                    }
                    Err(err) => {
                        app.session.clear_resolve_scope();
                        return Err(CoreError::other(err.to_string()));
                    }
                }
                if std::time::Instant::now() >= deadline {
                    let last = app.session.tree().summary();
                    let operation = if stuck.is_empty() {
                        header
                    } else {
                        format!("{header} ({stuck})")
                    };
                    break Err(CoreError::Timeout {
                        operation,
                        timeout,
                        last: Box::new(last),
                    });
                }
                app.session.pump();
            };
            app.session.clear_resolve_scope();
            result
        }
    }
}
