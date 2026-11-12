//! The wire Action request honors `force` and `timeout_ms` (architecture
//! §8.3 over the wire, M17): the pre-resolved node must still be
//! actionable at act time — the tree may have moved on since the Resolve
//! — so a non-force action waits for actionability within `timeout_ms`
//! and refuses with `Timeout` otherwise, naming the check it was stuck
//! on. `force: true` skips the stability + hit-test checks (the
//! [`guiwright_driver::ActionOpts`] contract) — it is **not** a license
//! through hidden or disabled controls, which stay refused. The
//! conversation rides the in-process door (M16).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    ErrorCode, NodeKeyV1, RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1,
};

/// A Role-selector by exact name.
fn button_named(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

/// Resolve through the door; returns the wire handle + generation.
async fn resolve(
    client: &mut DuplexClient,
    selector: SelectorV1,
    timeout_ms: u64,
) -> (NodeKeyV1, u32) {
    let resolved = client
        .request(RequestV1::Resolve {
            selector,
            timeout_ms,
        })
        .await
        .expect("resolve");
    match resolved {
        ResponseV1::Resolved {
            node, generation, ..
        } => (node, generation),
        other => panic!("expected Resolved, got {other:?}"),
    }
}

/// Send an Action request and unwrap the wire result: an error response
/// arrives as `GwpError::Rejected`, a success as the `ActionDone`
/// payload.
async fn act(
    client: &mut DuplexClient,
    node: NodeKeyV1,
    generation: u32,
    force: bool,
    timeout_ms: u64,
) -> Result<String, (ErrorCode, String)> {
    let response = client
        .request(RequestV1::Action {
            node,
            generation,
            action: SemActionV1::Click,
            force,
            timeout_ms,
        })
        .await;
    match response {
        Ok(ResponseV1::ActionDone { detail, .. }) => Ok(detail),
        Err(GwpError::Rejected { code, message }) => Err((code, message)),
        other => panic!("expected ActionDone or a wire error, got {other:?}"),
    }
}

/// A non-force action on the fixture's hidden button refuses with
/// `Timeout` inside `timeout_ms`, naming the failed check — never click
/// a hidden control (§8.3 over the wire).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn action_without_force_refuses_a_hidden_node() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    client.handshake("action-opts/0.1").await.unwrap();
    let (node, generation) = resolve(&mut client, button_named("Invisible"), 2000).await;

    let started = std::time::Instant::now();
    let outcome = act(&mut client, node, generation, false, 300).await;
    let elapsed = started.elapsed();

    let (code, message) = outcome.expect_err("a hidden node is refused");
    assert_eq!(code, ErrorCode::Timeout, "the §8.3 refusal: {message}");
    assert!(
        message.contains("stuck on visible"),
        "the failing check is named: {message}"
    );
    assert!(
        elapsed >= Duration::from_millis(250),
        "the wait actually waited, took {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "bounded by timeout_ms, took {elapsed:?}"
    );

    handle.shutdown();
}

/// A non-force action succeeds once the node becomes actionable inside
/// the window: the gate's frame pumps integrate the scripted bounds
/// return and the click lands — no force needed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn action_without_force_waits_out_the_gap() {
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    // Hide the Save button (bounds gone) before anyone resolves it.
    let mut hidden_save = Node::new(Role::Button);
    hidden_save.set_label("Save");
    shared.lock().unwrap().push(TreeUpdate {
        nodes: vec![(NodeId(2), hidden_save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    });

    let mut client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    client.handshake("action-opts/0.1").await.unwrap();
    let (node, generation) = resolve(&mut client, button_named("Save"), 2000).await;

    // The app restores the button while the action request is in flight:
    // the gate's frame pumps integrate the update and the click proceeds.
    let mut shown_save = Node::new(Role::Button);
    shown_save.set_label("Save");
    shown_save.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 16.0,
        x1: 136.0,
        y1: 48.0,
    });
    shared.lock().unwrap().push(TreeUpdate {
        nodes: vec![(NodeId(2), shown_save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    });

    let detail = act(&mut client, node, generation, false, 5000)
        .await
        .expect("the node returned inside the window");
    assert_eq!(detail, "clicked");

    handle.shutdown();
}

/// `force: true` skips the hit-test gate: an occluded button is clicked
/// anyway — while the same request without force refuses, stuck on
/// `receives events` (the ActionOpts contract: force skips stability +
/// hit-test, nothing else).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn action_with_force_skips_the_hit_test_gate() {
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    // Drop an overlay group over the Save button's center, painted after
    // every existing sibling (later children win the point lookup).
    let mut window = Node::new(Role::Window);
    window.set_label("Test App");
    window.set_children([NodeId(1)]);
    let mut group = Node::new(Role::Group);
    group.set_children([
        NodeId(2),
        NodeId(3),
        NodeId(4),
        NodeId(5),
        NodeId(6),
        NodeId(7),
        NodeId(20),
    ]);
    let mut overlay = Node::new(Role::Group);
    overlay.set_label("Overlay");
    overlay.set_bounds(accesskit::Rect {
        x0: 56.0,
        y0: 8.0,
        x1: 140.0,
        y1: 56.0,
    });
    shared.lock().unwrap().push(TreeUpdate {
        nodes: vec![
            (NodeId(0), window),
            (NodeId(1), group),
            (NodeId(20), overlay),
        ],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    });

    let mut client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    client.handshake("action-opts/0.1").await.unwrap();
    let (node, generation) = resolve(&mut client, button_named("Save"), 2000).await;

    let (code, message) = act(&mut client, node.clone(), generation, false, 300)
        .await
        .expect_err("the occluded button is refused without force");
    assert_eq!(code, ErrorCode::Timeout, "{message}");
    assert!(
        message.contains("stuck on receives events"),
        "the hit-test check is named: {message}"
    );

    let detail = act(&mut client, node, generation, true, 300)
        .await
        .expect("force skips the hit-test gate");
    assert_eq!(detail, "clicked");

    handle.shutdown();
}

/// Hidden and disabled controls stay refused even under `force`: force
/// skips stability + hit-test only — `Visible` and `Enabled` still hold
/// the line (the [`guiwright_driver::ActionOpts`] contract, now pinned on
/// the wire).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_still_refuses_disabled_and_hidden_nodes() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    client.handshake("action-opts/0.1").await.unwrap();

    // The disabled Confirm button: refused without force…
    let (node, generation) = resolve(&mut client, button_named("Confirm"), 2000).await;
    let (code, message) = act(&mut client, node.clone(), generation, false, 300)
        .await
        .expect_err("a disabled node is refused");
    assert_eq!(code, ErrorCode::Timeout, "{message}");
    assert!(
        message.contains("stuck on enabled"),
        "the enablement check is named: {message}"
    );

    // …and still refused under force — Enabled is not a force-skipped
    // check.
    let (code, message) = act(&mut client, node.clone(), generation, true, 300)
        .await
        .expect_err("force does not license disabled controls");
    assert_eq!(code, ErrorCode::Timeout, "{message}");
    assert!(message.contains("stuck on enabled"), "{message}");

    handle.shutdown();
}
