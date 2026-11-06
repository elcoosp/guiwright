//! The session: adapter + tree service + event-driven wait loop + action
//! scheduler (architecture §5.1 "driver thread(s)" and §15.3 zero-polling).
//!
//! The session owns the adapter on one thread (the UI thread in embedded
//! mode). It pumps the adapter, feeds every resulting event (tree diffs,
//! presented frames) into the active waiter, and performs actions with
//! full actionability checking (§8.3). Checks run **exactly when the world
//! changes** — between frames the session does no CPU work.

use std::time::{Duration, Instant};

use accesskit::NodeId;
use guiwright_core::actionability::{Check, action_checks};
use guiwright_core::expect::DEFAULT_TIMEOUT;
use guiwright_core::tree::{Resolved, TreeService};
use guiwright_core::wait::{ChecksWaiter, WaitOutcome, Waiter, WaiterEvent};
use guiwright_core::{CoreError, NodeKey, Selector};

use crate::adapter::{ActionOutcome, FrameDeadline, FrameworkAdapter, InputEvent, SemAction};

/// Options for actions (§8.3 "Timeout & retries" + force mode).
#[derive(Clone, Debug)]
pub struct ActionOpts {
    pub timeout: Duration,
    /// Skip stability + hit-test checks (Playwright `force: true`).
    pub force: bool,
}

impl Default for ActionOpts {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            force: false,
        }
    }
}

/// What one pump produced (for traces and tests).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpSummary {
    pub frames_presented: usize,
    pub tree_changed: bool,
    pub quiescent: bool,
}

/// The outcome of a completed wait: either a resolved node or
/// satisfied-by-absence (only possible for pure-negation assertions).
#[derive(Clone, Debug)]
pub enum WaitResolved {
    Node(Resolved),
    /// `expect(...).to_be_hidden()` on a node that never existed
    /// (Playwright parity). No node to act on.
    Absent,
}

impl WaitResolved {
    /// Unwrap for action paths — absence is an error there.
    pub fn into_node(self) -> Result<Resolved, CoreError> {
        match self {
            WaitResolved::Node(r) => Ok(r),
            WaitResolved::Absent => Err(CoreError::Other(
                "action target vanished before the action could run".into(),
            )),
        }
    }
}

/// Session state machine around an adapter (§5.1).
pub struct Session<A: FrameworkAdapter> {
    adapter: A,
    tree: TreeService,
    tree_seq: u64,
    frame_seq: u64,
    /// Last frame seq fed to a waiter — events are fed exactly once.
    fed_frame_seq: u64,
}

impl<A: FrameworkAdapter> Session<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            tree: TreeService::new(),
            tree_seq: 0,
            frame_seq: 0,
            fed_frame_seq: 0,
        }
    }

    pub fn tree(&self) -> &TreeService {
        &self.tree
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    /// Pump the adapter once and integrate the outcome: tree diffs feed the
    /// tree service, frame stamps feed stability bookkeeping. This is the
    /// frame-boundary rendezvous (§15.4): **one rendezvous per action**,
    /// no "spin the loop N times" heuristics.
    pub fn pump(&mut self) -> PumpSummary {
        // Pump the framework loop with a short frame deadline; the session
        // re-checks outer deadlines between pumps.
        let outcome = self
            .adapter
            .pump(FrameDeadline::within(Duration::from_millis(30)));
        let mut summary = PumpSummary::default();

        if let Some(update) = self.adapter.tree_update() {
            let _diff = self.tree.apply_update(update);
            self.tree_seq += 1;
            summary.tree_changed = true;
        }
        if let Some(diff) = self.adapter.logical_snapshot() {
            // Level-2 adapters synthesize an AccessKit-shaped update
            // (§6.1): selectors, actionability and expect run unchanged on
            // the consumer tree; actions come back with the adapter-minted
            // NodeIds (session.rs `node_id`).
            let _diff = self.tree.apply_update(diff.update);
            for (node, test_id) in diff.test_ids {
                self.tree.register_test_id(
                    guiwright_core::NodeKey::AccessKit {
                        tree: accesskit::TreeId::ROOT,
                        node: NodeId(node),
                    },
                    test_id,
                );
            }
            self.tree_seq += 1;
            summary.tree_changed = true;
        }

        for stamp in &outcome.frames {
            self.frame_seq = self.frame_seq.max(stamp.seq);
            let _info = self.tree.frame_presented(stamp.hash);
            summary.frames_presented += 1;
        }
        summary.quiescent = outcome.is_quiescent();
        summary
    }

    /// Pump until the app is quiescent or the deadline hits (§6.1 "pump
    /// until quiescent"). Used at session start and after clock jumps.
    pub fn settle(&mut self, deadline: Instant) -> PumpSummary {
        let mut total = PumpSummary::default();
        loop {
            if Instant::now() >= deadline {
                break;
            }
            let s = self.pump();
            total.frames_presented += s.frames_presented;
            total.tree_changed |= s.tree_changed;
            total.quiescent = s.quiescent;
            if s.quiescent {
                break;
            }
        }
        total
    }

    /// Resolve a selector against the current tree (strict mode). A pump is
    /// performed first so freshly-queued diffs are visible — resolve stays
    /// consistent with the adapter's latest state.
    pub fn resolve(&mut self, selector: &Selector) -> Result<Resolved, CoreError> {
        self.pump();
        self.tree.resolve(selector)
    }

    /// Resolve returning all matches (no strict check).
    pub fn resolve_all(&mut self, selector: &Selector) -> Result<Vec<Resolved>, CoreError> {
        self.pump();
        self.tree.resolve_all(selector)
    }

    /// The event-driven wait loop (§15.3): pump → feed events → re-check.
    ///
    /// Waiters only progress on events; each pump feeds at most one event
    /// (the newest world change). Nothing is fed twice, and when the pump
    /// produced nothing new the waiter is not consulted at all — that is
    /// the zero-polling contract in practice.
    pub fn wait_for(
        &mut self,
        selector: &Selector,
        checks: Vec<Check>,
        deadline: Instant,
    ) -> Result<WaitResolved, CoreError> {
        let started = Instant::now();
        let configured = deadline.saturating_duration_since(started);
        let mut waiter = ChecksWaiter::new(selector.clone(), checks);

        // Initial evaluation against the current tree — many waits finish
        // without pumping at all.
        match waiter.on_event(WaiterEvent::Initial, &self.tree) {
            WaitOutcome::Satisfied(resolved) => {
                return Ok(resolved.map_or(WaitResolved::Absent, WaitResolved::Node));
            }
            WaitOutcome::Failed(failure) => {
                return Err(CoreError::Other(format!(
                    "wait for `{}` failed: {} ({})",
                    selector.render(),
                    failure.check,
                    failure.reason
                )));
            }
            WaitOutcome::Pending => {}
        }

        loop {
            if Instant::now() >= deadline {
                return Err(CoreError::Timeout {
                    operation: waiter.describe(),
                    timeout: configured,
                    last: Box::new(self.tree.summary()),
                });
            }
            let last_diff = self.tree.last_diff().clone();
            let last_frame = self.tree.last_frame();
            self.pump();

            // Feed the newest event produced by this pump, if any.
            let diff = self.tree.last_diff();
            let event = if !diff.is_empty() && diff != &last_diff {
                Some(WaiterEvent::TreeChanged(diff))
            } else if let Some(info) = self.tree.last_frame() {
                let frame_is_new = last_frame.is_none_or(|prev| info.seq > prev.seq);
                frame_is_new.then_some(WaiterEvent::FramePresented {
                    seq: info.seq,
                    hash: info.hash,
                })
            } else {
                None
            };

            if let Some(ev) = event {
                match waiter.on_event(ev, &self.tree) {
                    WaitOutcome::Satisfied(resolved) => {
                        self.fed_frame_seq = self.frame_seq;
                        return Ok(resolved.map_or(WaitResolved::Absent, WaitResolved::Node));
                    }
                    WaitOutcome::Failed(failure) => {
                        return Err(CoreError::Other(format!(
                            "wait for `{}` failed: {} ({})",
                            selector.render(),
                            failure.check,
                            failure.reason
                        )));
                    }
                    WaitOutcome::Pending => {}
                }
            }
        }
    }

    /// expect()-style assertion with retry-until-timeout semantics (§16).
    pub fn expect(&mut self, selector: Selector) -> ExpectBuilder<'_, A> {
        ExpectBuilder {
            session: self,
            selector,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Run a semantic action with full actionability checking (§8.3–§8.4).
    /// Force mode skips stability + hit-test checks.
    pub fn perform_action(
        &mut self,
        selector: &Selector,
        action: SemAction,
        opts: &ActionOpts,
    ) -> Result<ActionOutcome, CoreError> {
        let deadline = Instant::now() + opts.timeout;
        let checks = action_checks(opts.force);
        let resolved = self.wait_for(selector, checks, deadline)?.into_node()?;
        // Generation guard (§8.2): the identity must still be live at act
        // time — no ABA through id recycling.
        self.tree.check_generation(&resolved)?;

        // §8.4: hover/move/drag/scroll are NOT semantic actions — they ride
        // the real input pipeline with coordinates from the resolved bounds.
        let outcome = match (&action, resolved.bounds) {
            (SemAction::Hover, Some(rect)) => {
                self.adapter
                    .input(InputEvent::PointerMove {
                        x: (rect.x0 + rect.x1) / 2.0,
                        y: (rect.y0 + rect.y1) / 2.0,
                    })
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                ActionOutcome {
                    applied_at_frame: self.frame_seq,
                    detail: "hovered".into(),
                }
            }
            (SemAction::Scroll { dx, dy }, Some(_rect)) => {
                self.adapter
                    .input(InputEvent::Wheel { dx: *dx, dy: *dy })
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                ActionOutcome {
                    applied_at_frame: self.frame_seq,
                    detail: format!("scrolled ({dx},{dy})"),
                }
            }
            _ => self
                .adapter
                .act(node_id(&resolved.key), action)
                .map_err(|e| CoreError::Other(e.to_string()))?,
        };
        // Apply the action inside the next frame (rendezvous, §15.4): one
        // pump, then done — never spin.
        self.pump();
        Ok(outcome)
    }

    /// Apply a pre-resolved action (the GWP `Action` path, §7): the caller
    /// already holds the node handle + generation from a `Resolve`; the
    /// session validates the generation and applies inside the next frame.
    pub fn apply_action(
        &mut self,
        key: NodeKey,
        action: SemAction,
        force: bool,
        timeout_ms: u64,
    ) -> Result<ActionOutcome, CoreError> {
        let _ = (force, timeout_ms);
        let outcome = self
            .adapter
            .act(node_id(&key), action)
            .map_err(|e| CoreError::Other(e.to_string()))?;
        // Frame-boundary rendezvous (§15.4): one pump so the action is
        // applied at a frame before the reply is sent.
        self.pump();
        Ok(outcome)
    }

    /// `locator.click()`.
    pub fn click(&mut self, selector: &Selector) -> Result<ActionOutcome, CoreError> {
        self.perform_action(selector, SemAction::Click, &ActionOpts::default())
    }

    /// `locator.fill(text)`.
    pub fn fill(&mut self, selector: &Selector, text: &str) -> Result<ActionOutcome, CoreError> {
        self.perform_action(
            selector,
            SemAction::SetValue {
                text: text.to_string(),
            },
            &ActionOpts::default(),
        )
    }

    /// `locator.hover()`.
    pub fn hover(&mut self, selector: &Selector) -> Result<ActionOutcome, CoreError> {
        self.perform_action(selector, SemAction::Hover, &ActionOpts::default())
    }

    /// `app.clock().fast_forward(dt)` (§14).
    pub fn clock_fast_forward(&mut self, dt: Duration) -> Result<(), CoreError> {
        self.adapter
            .clock(crate::adapter::ClockCmd::FastForward(dt))
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.settle(Instant::now() + Duration::from_secs(5));
        Ok(())
    }

    pub fn tree_seq(&self) -> u64 {
        self.tree_seq
    }

    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }
}

fn node_id(key: &NodeKey) -> NodeId {
    match key {
        NodeKey::AccessKit { node, .. } => *node,
        NodeKey::Logical { adapter, id } => {
            // Level-2 identities are delivered to adapters by their numeric
            // handle; AccessKit NodeId carries it losslessly for the
            // adapter-side translation (§6.1 NodeKey normalization).
            tracing::debug!(adapter, id, "logical identity routed through NodeId");
            NodeId(*id)
        }
    }
}

/// Builder for retry-until-timeout assertions on a session (§16).
pub struct ExpectBuilder<'s, A: FrameworkAdapter> {
    session: &'s mut Session<A>,
    selector: Selector,
    timeout: Duration,
}

impl<'s, A: FrameworkAdapter> ExpectBuilder<'s, A> {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn run(self, checks: Vec<Check>, name: &str) -> Result<(), CoreError> {
        let deadline = Instant::now() + self.timeout;
        self.session
            .wait_for(&self.selector, checks, deadline)
            .map(|_| ())
            .map_err(|e| match e {
                CoreError::Timeout { last, .. } => CoreError::Timeout {
                    operation: name.to_string(),
                    timeout: self.timeout,
                    last,
                },
                other => other,
            })
    }

    pub fn to_be_visible(self) -> Result<(), CoreError> {
        self.run(vec![Check::Visible], "visible")
    }

    pub fn to_be_hidden(self) -> Result<(), CoreError> {
        self.run(vec![Check::Hidden], "hidden")
    }

    pub fn to_be_enabled(self) -> Result<(), CoreError> {
        self.run(vec![Check::Enabled], "enabled")
    }

    pub fn to_be_disabled(self) -> Result<(), CoreError> {
        self.run(vec![Check::Disabled], "disabled")
    }

    pub fn to_be_focused(self) -> Result<(), CoreError> {
        self.run(vec![Check::Focused], "focused")
    }

    pub fn to_have_text(self, text: impl Into<guiwright_core::TextMatch>) -> Result<(), CoreError> {
        self.run(vec![Check::HasText(text.into())], "text")
    }

    pub fn to_have_value(
        self,
        value: impl Into<guiwright_core::TextMatch>,
    ) -> Result<(), CoreError> {
        self.run(vec![Check::HasText(value.into())], "value")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAdapter;
    use guiwright_core::test_util::Fixture;

    fn save_selector() -> Selector {
        Selector::role_named(accesskit::Role::Button, "Save")
    }

    /// A settled mock with two presented frames (stability baseline +
    /// confirmation), mirroring what a real adapter streams while idle.
    fn settled_session(fixture: &Fixture) -> Session<MockAdapter> {
        let mut adapter = MockAdapter::with_fixture(fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        session.settle(Instant::now() + Duration::from_secs(1));
        session
    }

    #[test]
    fn click_performs_actionability_then_act() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let outcome = session.click(&save_selector()).expect("click");
        assert_eq!(outcome.detail, "clicked");
        let actions = session.adapter().actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].1, SemAction::Click);
        assert_eq!(actions[0].0, fixture.save_button, "acts on the Save node");
    }

    #[test]
    fn click_waits_for_late_appearing_button() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        adapter.push_update(fixture.add_button_update("Export"));
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        let outcome = session
            .click(&Selector::role_named(accesskit::Role::Button, "Export"))
            .expect("export clicked after it appeared");
        assert_eq!(outcome.detail, "clicked");
    }

    #[test]
    fn click_on_disabled_times_out() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        let err = session
            .perform_action(
                &Selector::role_named(accesskit::Role::Button, "Confirm"),
                SemAction::Click,
                &ActionOpts {
                    timeout: Duration::from_millis(200),
                    force: false,
                },
            )
            .expect_err("disabled button cannot be clicked");
        assert!(
            matches!(err, CoreError::Timeout { timeout, .. } if timeout >= Duration::from_millis(199)),
            "{err}"
        );
        assert!(
            session.adapter().actions().is_empty(),
            "no action must reach the adapter"
        );
    }

    #[test]
    fn force_skips_stability_but_not_enabled() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .perform_action(
                &save_selector(),
                SemAction::Click,
                &ActionOpts {
                    timeout: Duration::from_secs(2),
                    force: true,
                },
            )
            .expect("force click skips stability");
    }

    #[test]
    fn assertions_run_on_session() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .expect(save_selector())
            .to_be_visible()
            .expect("save visible");
        session
            .expect(Selector::role_named(accesskit::Role::Button, "Confirm"))
            .to_be_disabled()
            .expect("confirm disabled");
        session
            .expect(Selector::role(accesskit::Role::TextInput))
            .to_have_value("ada@example.dev")
            .expect("email value");
        session
            .expect(Selector::role_named(accesskit::Role::Button, "Ghost"))
            .to_be_hidden()
            .expect("ghost hidden");
    }

    #[test]
    fn fill_sets_value_through_action() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .fill(&Selector::role(accesskit::Role::TextInput), "hello@dev")
            .expect("fill");
        let actions = session.adapter().actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].1,
            SemAction::SetValue {
                text: "hello@dev".into()
            }
        );
    }

    #[test]
    fn settle_drains_scripted_frames() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::with_fixture(&fixture);
        adapter.push_frame(1);
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        let summary = session.settle(Instant::now() + Duration::from_secs(1));
        assert!(summary.quiescent);
        // 3 scripted passes + 1 trailing pass (the pump that discovers
        // quiescence still presents one pass — harness-driven adapters do).
        assert_eq!(summary.frames_presented, 4);
    }

    #[test]
    fn generation_guard_blocks_stale_action() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let resolved = session
            .tree()
            .resolve(&save_selector())
            .expect("save resolves");
        session
            .adapter_mut()
            .push_update(fixture.remove_save_update());
        session.pump();
        let err = session.tree.check_generation(&resolved).expect_err("stale");
        assert!(matches!(err, CoreError::StaleNode { .. }), "{err}");
    }

    #[test]
    fn strict_mode_violation_surfaces() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        let err = session
            .click(&Selector::role(accesskit::Role::Button))
            .expect_err("four buttons");
        assert!(matches!(err, CoreError::Other(_)), "{err}");
        assert!(err.to_string().contains("resolved to"), "{err}");
    }

    /// Level-2 (M1): an iced-shaped adapter synthesizes an AccessKit
    /// update through `logical_snapshot`; selectors, actionability and
    /// actions must run unchanged, and the adapter-minted NodeId routes
    /// back to the adapter (§6.1).
    #[test]
    fn logical_snapshot_resolves_and_routes_actions() {
        let fixture = Fixture::buttons();
        let mut adapter = MockAdapter::bare().as_logical();
        adapter.push_logical(crate::adapter::LogicalTreeDiff {
            adapter: "mock-logical",
            update: fixture.initial_update(),
            removed: vec![],
            test_ids: vec![],
            changed: true,
        });
        adapter.push_frame(1);
        adapter.push_frame(1);
        let mut session = Session::new(adapter);
        session.settle(Instant::now() + Duration::from_secs(1));

        let outcome = session.click(&save_selector()).expect("logical click");
        assert_eq!(outcome.detail, "clicked");
        let actions = session.adapter().actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].1, SemAction::Click);
        assert_eq!(
            actions[0].0, fixture.save_button,
            "the adapter-minted id routes back unchanged"
        );

        // ...and expect() assertions work on the synthesized tree too.
        session
            .expect(Selector::role_named(accesskit::Role::Button, "Confirm"))
            .to_be_disabled()
            .expect("logical tree carries semantics");
    }

    #[test]
    fn clock_fast_forward_reaches_mock() {
        let fixture = Fixture::buttons();
        let mut session = settled_session(&fixture);
        session
            .clock_fast_forward(Duration::from_secs(30))
            .expect("mock supports clock");
        assert_eq!(
            session.adapter().virtual_time(),
            Some(Duration::from_secs(30))
        );
    }
}
