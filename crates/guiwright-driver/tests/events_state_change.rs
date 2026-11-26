//! The §7 state-change events over the wire (the m20 record's
//! remaining-gaps list: *"`setChecked`-state change events (assert via
//! subscription instead of polling the tree) would ride the M12 event
//! channel with a new `EventKindV1` — same schema scope, same
//! reasoning"*): a check-vocabulary action that moves a node's checked
//! fact publishes [`EventV1::StateChange`] — the node and the tri-state
//! it now carries — so a client asserts on the event instead of polling
//! the tree. The event rides the same kind filter every M12 kind does,
//! and an action that lands without moving the fact publishes nothing.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{NodeId, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{
    CheckedStateV1, EventKindV1, EventV1, GwpEvent, NodeKeyV1, RequestV1, ResponseV1, SelectorV1,
    SemActionV1,
};
use guiwright_proto::uds::Client;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

/// Script the checked-fact move of the fixture's Dark-mode checkbox
/// (`NodeId` 10, `Toggled::False` in `Fixture::with_checkboxes`) — the
/// app-side state change a real adapter's widget mutation carries. The
/// fixture's own toggle helper re-states the row bounded (the
/// actionability gate reads bounds).
fn check_dark_mode() -> TreeUpdate {
    Fixture::with_checkboxes()
        .checkbox_toggle_update(NodeId(10), accesskit::Toggled::True)
        .expect("Dark mode is a trio node")
}

async fn connect(path: &std::path::Path) -> Client {
    let mut client = Client::connect(path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();
    client
}

/// Resolve the Dark-mode checkbox over the wire: the handle + generation
/// the `Action` request needs.
async fn resolve_dark_mode(client: &mut Client) -> (NodeKeyV1, u32) {
    let response = client
        .request(RequestV1::Resolve {
            selector: SelectorV1::Role {
                role: "CheckBox".into(),
                name: Some(guiwright_proto::schema::TextMatchV1 {
                    text: "Dark mode".into(),
                    exact: false,
                    ignore_case: false,
                }),
            },
            timeout_ms: 0,
        })
        .await
        .expect("resolve");
    let ResponseV1::Resolved {
        node, generation, ..
    } = response
    else {
        panic!("expected a Resolved reply, got {response:?}");
    };
    (node, generation)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_check_action_that_moves_the_fact_publishes_the_state_change() {
    let path = socket_path("events-state-change.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::with_checkboxes()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = connect(&path).await;
    let (node, generation) = resolve_dark_mode(&mut client).await;

    client
        .subscribe(&[EventKindV1::StateChange, EventKindV1::TreeChanged])
        .await
        .expect("subscribe");

    // The app-side checked-fact move lands with the action's pump (the
    // same shape a real adapter's widget mutation carries), then the
    // check action applies over the wire.
    shared.lock().expect("scripts lock").push(check_dark_mode());
    let ack = client
        .request(RequestV1::Action {
            node: node.clone(),
            generation,
            action: SemActionV1::Check,
            force: false,
            timeout_ms: 5000,
        })
        .await
        .expect("the check lands");
    assert!(
        matches!(ack, ResponseV1::ActionDone { .. }),
        "the action answer is unchanged: {ack:?}"
    );

    // The subscription answers instead of a tree poll: the StateChange
    // names the node and the tri-state it now carries.
    let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the state change arrives")
        .expect("event frame");
    let GwpEvent::V1(EventV1::StateChange {
        node: changed,
        checked,
    }) = event
    else {
        panic!("expected a state-change event, got {event:?}");
    };
    assert_eq!(changed, node, "the event names the acted-on node");
    assert_eq!(
        checked,
        CheckedStateV1::Checked,
        "the event carries the post-action tri-state"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_action_that_moves_nothing_publishes_nothing() {
    let path = socket_path("events-state-quiet.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::with_checkboxes())
    })
    .expect("server binds");
    let mut client = connect(&path).await;
    let (node, generation) = resolve_dark_mode(&mut client).await;

    client
        .subscribe(&[EventKindV1::StateChange])
        .await
        .expect("subscribe");

    // The check lands, but the checked fact stands still (no scripted
    // move): the honest contract publishes nothing — a state-change
    // event without a state change would be a lie.
    let ack = client
        .request(RequestV1::Action {
            node,
            generation,
            action: SemActionV1::Check,
            force: false,
            timeout_ms: 5000,
        })
        .await
        .expect("the check lands");
    assert!(matches!(ack, ResponseV1::ActionDone { .. }));

    let next = tokio::time::timeout(Duration::from_millis(400), client.next_event()).await;
    assert!(
        next.is_err(),
        "a moved-nothing action must publish no event — got {next:?}"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_state_change_rides_the_kind_filter() {
    // A subscriber that never named StateChange never sees one — the
    // same filter contract the M12 kinds ride.
    let path = socket_path("events-state-filter.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::with_checkboxes()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = connect(&path).await;
    let (node, generation) = resolve_dark_mode(&mut client).await;

    client
        .subscribe(&[EventKindV1::TreeChanged])
        .await
        .expect("subscribe to tree events only");

    shared.lock().expect("scripts lock").push(check_dark_mode());
    client
        .request(RequestV1::Action {
            node,
            generation,
            action: SemActionV1::Check,
            force: false,
            timeout_ms: 5000,
        })
        .await
        .expect("the check lands");

    // The tree event flows (the scripted move integrated); a state
    // change must never cross this filter.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(700);
    let mut received = 0;
    while tokio::time::Instant::now() < deadline {
        let event = tokio::time::timeout_at(deadline, client.next_event()).await;
        let Ok(event) = event else { break }; // quiet window: done
        let GwpEvent::V1(inner) = event.expect("event frame");
        assert!(
            matches!(inner, EventV1::TreeChanged { .. }),
            "the filter let a state change through: {inner:?}"
        );
        received += 1;
    }
    assert!(received >= 1, "the tree event arrived");

    handle.shutdown();
}
