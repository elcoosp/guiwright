//! The client-side wait over the wire (m42 — the m21 record's named
//! follow-up, closed): [`ClientConn::wait_for`] composes the §7 event
//! channel with the M39 describe-all re-reads — a remote suite waits
//! attached/visible/hidden/stable exactly like the kit's `wait_for`, no
//! wire change (the wait rides existing domains byte-for-byte). Driven
//! here over the in-process door with the four-button fixture and the
//! shared-script channel standing in for app-side UI changes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{DescribedV1, NodeKeyV1, SelectorV1, TextMatchV1};
use guiwright_proto::wait::WaitState;
use guiwright_proto::{DuplexClient, GwpError};

fn named_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-wait-test/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn_fixture() -> (GwpServerHandle, DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

/// The buttons fixture behind a shared script channel — updates pushed
/// from another thread land on the session's next pump, exactly like
/// app-side UI changes riding a real adapter.
fn spawn_scripted() -> (GwpServerHandle, DuplexConnect, Arc<Mutex<Vec<TreeUpdate>>>) {
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_app = Arc::clone(&scripts);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts_for_app),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, scripts)
}

/// A late "Late" button (node 8): the group restates its children so the
/// consumer tree grafts the new node — the same update shape a real
/// toolkit emits when a widget joins the layout.
fn late_button_update() -> TreeUpdate {
    let mut late = Node::new(Role::Button);
    late.set_label("Late");
    late.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 228.0,
        x1: 136.0,
        y1: 260.0,
    });
    let mut group = Node::new(Role::Group);
    group.set_children([
        NodeId(2),
        NodeId(3),
        NodeId(4),
        NodeId(5),
        NodeId(6),
        NodeId(7),
        NodeId(8),
    ]);
    TreeUpdate {
        nodes: vec![(NodeId(1), group), (NodeId(8), late)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    }
}

fn node_id_of(row: &DescribedV1) -> u64 {
    match &row.node {
        NodeKeyV1::AccessKit { node, .. } => *node,
        other => panic!("expected AccessKit keys, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_visible_is_satisfied_on_entry() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let row = client
        .wait_for(
            named_button("Save"),
            WaitState::Visible,
            Duration::from_secs(1),
        )
        .await
        .expect("Save is visible from the start");
    let row = row.expect("a positive state answers its satisfying row");
    assert_eq!(
        node_id_of(&row),
        2,
        "the Save row, usable as an Action handle"
    );
    assert_eq!(row.bounds, Some([16.0, 16.0, 136.0, 48.0]));

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_attached_waits_for_the_tree_to_grow() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    // The app "renders" the Late button a moment after attach: the wait
    // subscribes, evaluates (no match yet), and wakes on the pushed
    // TreeChanged — event-driven, no polling (§15.3 on both wire ends).
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(late_button_update());
    });

    let row = client
        .wait_for(
            named_button("Late"),
            WaitState::Attached,
            Duration::from_secs(2),
        )
        .await
        .expect("the Late button arrives");
    let row = row.expect("a positive state answers its satisfying row");
    assert_eq!(node_id_of(&row), 8, "the freshly grafted node");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_hidden_is_satisfied_by_absence() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The fixture's "Invisible" button is attached but carries no bounds:
    // hidden is satisfied by absence of any visible row (Playwright
    // parity), and the answer is `None` — there is no row to hand back.
    let satisfied = client
        .wait_for(
            named_button("Invisible"),
            WaitState::Hidden,
            Duration::from_secs(1),
        )
        .await
        .expect("hidden is satisfied from the start");
    assert!(satisfied.is_none(), "absence answers None: {satisfied:?}");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_visible_times_out_naming_the_state() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // Waiting for an invisible node to become visible expires: the error
    // names the state, the locator and the last evaluation (the "failures
    // name names" discipline over the wire).
    let err = client
        .wait_for(
            named_button("Invisible"),
            WaitState::Visible,
            Duration::from_millis(150),
        )
        .await
        .expect_err("an invisible node never satisfies a visible wait");
    match &err {
        GwpError::WaitTimeout {
            state,
            selector,
            last,
            ..
        } => {
            assert_eq!(*state, "visible");
            assert!(selector.contains("Invisible"), "{selector:?}");
            assert!(last.contains("no bounds"), "{last:?}");
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }
    assert!(err.to_string().contains("150 ms"), "{}", err);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spent_wait_leaves_the_connection_healthy() {
    // The m44 read-half contract at the consumer level: a wait that
    // expires — its deadline dropping `next_event` mid-conversation, and
    // byte-level possibly mid-frame (the conn-level tests stage that
    // hazard directly) — leaves the SAME connection serving the rest of
    // the session. The expire path's unsubscribe and the follow-up round
    // trips ride the buffered read half.
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let err = client
        .wait_for(
            named_button("Invisible"),
            WaitState::Visible,
            Duration::from_millis(120),
        )
        .await
        .expect_err("an invisible node never satisfies a visible wait");
    assert!(matches!(err, GwpError::WaitTimeout { .. }), "{err:?}");

    // The spent wait dropped its subscription and the connection keeps
    // answering: a liveness probe, then a full set read on the same door.
    client.ping().await.expect("the connection still answers");
    let rows = client
        .describe_all(named_button("Save"))
        .await
        .expect("the set read rides the same door");
    assert_eq!(rows.len(), 1, "the Save row answers after the spent wait");

    // A second wait starts clean (no stale subscription, no stale
    // events) and expires the same honest way.
    let err = client
        .wait_for(
            named_button("Invisible"),
            WaitState::Visible,
            Duration::from_millis(120),
        )
        .await
        .expect_err("the second wait expires the same way");
    assert!(matches!(err, GwpError::WaitTimeout { .. }), "{err:?}");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_stable_satisfied_across_presented_frames() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The mock presents a frame every pump, and the session pumps at
    // frame cadence while anyone is subscribed (§7, M12) — two identical
    // observations of the Save rect land well inside the timeout.
    let row = client
        .wait_for(
            named_button("Save"),
            WaitState::Stable,
            Duration::from_secs(3),
        )
        .await
        .expect("a static node stabilizes");
    let row = row.expect("a positive state answers its satisfying row");
    assert_eq!(node_id_of(&row), 2);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_for_leaves_no_subscription_behind() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    client
        .wait_for(
            named_button("Save"),
            WaitState::Visible,
            Duration::from_secs(1),
        )
        .await
        .expect("satisfied on entry");

    // The wait owned the event filter for its duration and dropped it on
    // return (a consumer that never drains its events must not stay
    // subscribed — the driver only pumps while subscribers exist). Any
    // events that interleaved the short window are drained first; after
    // that the wire is quiet — no wake-ups flow.
    let deadline = std::time::Instant::now() + Duration::from_millis(120);
    while tokio::time::timeout(Duration::from_millis(30), client.next_event())
        .await
        .is_ok()
    {
        assert!(std::time::Instant::now() < deadline, "drain took too long");
    }
    // Nothing flows anymore: the subscription the wait installed is off.
    assert!(
        tokio::time::timeout(Duration::from_millis(150), client.next_event())
            .await
            .is_err(),
        "a post-wait connection must be quiet — the wait unsubscribed"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_scope_rides_the_window_target() {
    let fx = MultiWindowFixture::default();
    let w2 = fx.w2.0.to_string();
    let w2_bytes = *fx.w2.0.as_bytes();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_multi_window(&fx),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    client.window_use(&w2).await.expect("Preview aimed");

    // Both windows carry a "Save" — the connection's target narrows the
    // wait's describe-all re-reads exactly like every resolve path (M6).
    let row = client
        .wait_for(
            named_button("Save"),
            WaitState::Visible,
            Duration::from_secs(1),
        )
        .await
        .expect("Preview's Save is visible");
    let row = row.expect("a positive state answers its satisfying row");
    match &row.node {
        NodeKeyV1::AccessKit { tree, node } => {
            assert_eq!(*node, 2);
            assert_eq!(
                tree.as_slice(),
                w2_bytes.as_slice(),
                "the Preview subtree, not the Editor's Save"
            );
        }
        other => panic!("expected AccessKit keys, got {other:?}"),
    }

    // And a selector that only matches inside the Editor never satisfies
    // under the Preview target — the wait expires pending.
    let editor_only = SelectorV1::Label(TextMatchV1 {
        text: "editor content".into(),
        exact: true,
        ignore_case: false,
    });
    let err = client
        .wait_for(editor_only, WaitState::Attached, Duration::from_millis(150))
        .await
        .expect_err("the Editor's content is outside the Preview target");
    assert!(matches!(err, GwpError::WaitTimeout { .. }), "{err:?}");

    handle.shutdown();
}
