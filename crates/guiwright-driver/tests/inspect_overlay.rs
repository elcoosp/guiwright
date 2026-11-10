//! The §13 inspector overlay over the wire (M8 "inspect highlight"):
//! the session builds the [`OverlayPlan`] from the live tree facts and
//! hands it to the adapter's `show_overlay` hook at frame boundaries —
//! the "live highlight handles land with the interactive overlay session"
//! promise the `Inspect` domain doc has carried since M2.
//!
//! The mock adapter records the last plan it received, so every test
//! asserts on exactly what an adapter would composite at present time.

use accesskit::{NodeId, Role, TreeUpdate};
use guiwright_core::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_driver::Session;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, FrameDeadline, FrameworkAdapter, PumpOutcome,
    SemAction,
};
use guiwright_driver::mock::MockAdapter;

fn fixture_session() -> Session<MockAdapter> {
    // `with_fixture` already scripts the fixture's initial update (what a
    // Level-1 adapter emits on attach); two frames so the first settle
    // pumps have presented-frame stamps.
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    Session::new(adapter)
}

/// The §6.2 honest default: an adapter that implements nothing but the
/// trait's required methods has no overlay surface, so the whole Inspect
/// overlay domain reports `Unsupported` instead of silently doing nothing.
struct BareAdapter {
    meta: AdapterMeta,
}

impl FrameworkAdapter for BareAdapter {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        PumpOutcome::default()
    }

    fn tree_update(&mut self) -> Option<TreeUpdate> {
        None
    }

    fn act(&mut self, _node: NodeId, _action: SemAction) -> Result<ActionOutcome, AdapterError> {
        Err(AdapterError::Unsupported("act"))
    }
}

#[test]
fn enable_hands_a_plan_to_the_adapter() {
    let mut session = fixture_session();
    assert!(!session.overlay_active(), "off by default");
    assert!(session.adapter().overlay().is_none(), "nothing composited");

    session.inspect_enable_overlay().expect("overlay enables");
    assert!(session.overlay_active());
    let plan = session
        .adapter()
        .overlay()
        .expect("the adapter received the plan");
    assert!(plan.hover.is_none(), "no highlight yet");
    // Interactive-only badges (§13): the two fixture buttons + the email
    // input carry badges; the "Welcome, Ada" label and the window/group
    // containers do not.
    let texts: Vec<&str> = plan.badges.iter().map(|b| b.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Save")),
        "button badge in {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("Email")),
        "text-input badge in {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("Welcome, Ada")),
        "labels are not interactive: {texts:?}"
    );
}

#[test]
fn highlight_outlines_the_resolved_node_and_returns_its_facts() {
    let mut session = fixture_session();
    let save = Selector::role_named(Role::Button, "Save");
    let facts = session
        .inspect_highlight(&save)
        .expect("highlight resolves + enables the overlay");

    assert_eq!(facts.role.as_deref(), Some("Button"));
    assert_eq!(facts.name.as_deref(), Some("Save"));
    let bounds = facts.bounds.expect("the fixture Save button has bounds");
    assert_eq!(bounds, [16.0, 16.0, 136.0, 48.0]);

    let plan = session.adapter().overlay().expect("plan after highlight");
    let hover = plan.hover.expect("the Save button is outlined");
    assert_eq!(
        [hover.x0, hover.y0, hover.x1, hover.y1],
        [16.0, 16.0, 136.0, 48.0]
    );
    assert!(
        session.overlay_active(),
        "highlight implies the overlay session is on"
    );
}

#[test]
fn highlight_switches_between_nodes_and_clears() {
    let mut session = fixture_session();
    let save = Selector::role_named(Role::Button, "Save");
    let cancel = Selector::role_named(Role::Button, "Cancel");

    session.inspect_highlight(&save).expect("highlight Save");
    let first = session
        .adapter()
        .overlay()
        .and_then(|plan| plan.hover)
        .expect("Save outlined");
    session
        .inspect_highlight(&cancel)
        .expect("highlight Cancel");
    let second = session
        .adapter()
        .overlay()
        .and_then(|plan| plan.hover)
        .expect("Cancel outlined");
    assert_ne!(
        (first.x0, first.y0),
        (second.x0, second.y0),
        "the outline followed the new target"
    );

    session.inspect_clear_highlight().expect("outline cleared");
    let plan = session.adapter().overlay().expect("overlay stays on");
    assert!(plan.hover.is_none(), "no outline");
    assert!(
        session.overlay_active(),
        "clearing the highlight keeps the overlay session"
    );
}

#[test]
fn disable_clears_the_overlay_surface() {
    let mut session = fixture_session();
    session
        .inspect_highlight(&Selector::role_named(Role::Button, "Save"))
        .expect("highlight");
    assert!(session.adapter().overlay().is_some());

    session.inspect_disable_overlay().expect("overlay disables");
    assert!(!session.overlay_active());
    assert!(session.adapter().overlay().is_none(), "nothing composited");

    // A later highlight starts a fresh session (highlight implies
    // enable — the fewest-states rule): the outline is back, nothing
    // half-cleared lingers.
    session
        .inspect_highlight(&Selector::role_named(Role::Button, "Cancel"))
        .expect("fresh session");
    assert!(session.overlay_active());
    let plan = session.adapter().overlay().expect("plan again");
    assert!(plan.hover.is_some(), "outline re-aimed");
}

#[test]
fn highlight_unknown_selector_is_a_strict_mode_error() {
    let mut session = fixture_session();
    let err = session
        .inspect_highlight(&Selector::role_named(Role::Button, "Missing"))
        .expect_err("no such button");
    assert!(
        err.to_string().contains("no node matches"),
        "strict-mode wording, got: {err}"
    );
}

#[test]
fn highlight_unknown_key_is_an_unknown_node_error() {
    let mut session = fixture_session();
    let ghost = guiwright_core::NodeKey::AccessKit {
        tree: accesskit::TreeId::ROOT,
        node: NodeId(4242),
    };
    let err = session
        .inspect_highlight_key(Some(ghost))
        .expect_err("no such node");
    assert!(
        err.to_string().contains("no node"),
        "unknown-node wording, got: {err}"
    );
    assert!(
        session.adapter().overlay().is_none(),
        "a failed highlight composites nothing"
    );
}

#[test]
fn overlay_plan_follows_tree_changes_on_pump() {
    let mut session = fixture_session();
    session.inspect_enable_overlay().expect("overlay enables");
    let before = session.adapter().overlay().expect("plan");
    let count_before = before.badges.len();

    // The app adds a button (§15.2 incremental update); the next pump
    // refreshes the plan from the new tree — badges are never stale.
    let fixture = Fixture::buttons();
    session
        .adapter_mut()
        .push_update(fixture.add_button_update("Export"));
    session.pump();
    let after = session.adapter().overlay().expect("plan after pump");
    let texts: Vec<&str> = after.badges.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        after.badges.len(),
        count_before + 1,
        "the Export button gained a badge: {texts:?}"
    );
    assert!(texts.iter().any(|t| t.contains("Export")), "{texts:?}");
}

#[test]
fn adapters_without_an_overlay_surface_report_unsupported() {
    let mut session = Session::new(BareAdapter {
        meta: AdapterMeta {
            name: "bare",
            framework: "bare",
            framework_version: "0".into(),
            caps: guiwright_driver::Caps::default(),
        },
    });
    let err = session
        .inspect_enable_overlay()
        .expect_err("no overlay surface");
    assert!(
        matches!(err, guiwright_core::CoreError::Unsupported(_)),
        "honest §6.2 Unsupported, got: {err}"
    );
    assert!(!session.overlay_active(), "the session stays off");
}
