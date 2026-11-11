//! The §7 server-push event channel over the wire (M12): a subscribed
//! connection receives `Event` frames — tree diffs and presented frames,
//! flushed between responses while the app advances with no request in
//! flight — and an unsubscribed connection keeps the byte-exact pre-M12
//! conversation.
//!
//! The driver-side scripting is the established `with_shared_scripts`
//! pattern (M7): updates pushed into the shared vec from the test thread
//! land on the session's next pump.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::{Endpoint, GwpServerHandle};
use guiwright_driver::{SeamRegistry, spawn_session_server};
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    ErrorCode, EventKindV1, EventV1, GwpEvent, RequestV1, ResponseV1, SubscribeV1,
};
use guiwright_proto::tcp::Client as TcpClient;
use guiwright_proto::uds::Client;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

/// Script a rename of the fixture's Save button (the late-change pattern
/// from `tests/tree_dump.rs`): one pump integrates the diff and presents
/// the repaint frame — both event kinds.
fn rename_save_update() -> TreeUpdate {
    let mut save = Node::new(Role::Button);
    save.set_label("Save (renamed)");
    save.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 16.0,
        x1: 136.0,
        y1: 48.0,
    });
    TreeUpdate {
        nodes: vec![(NodeId(2), save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribed_client_receives_tree_and_frame_events() {
    let path = socket_path("events-both.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    client
        .subscribe(&[EventKindV1::TreeChanged, EventKindV1::FramePresented])
        .await
        .expect("subscribe");

    // The app advances with NO request in flight: the idle tick pumps,
    // and queued diffs become events (§7 multiplexing). Harness-style
    // adapters present one pass per pump — frames (hash 0 = static pass)
    // flow continuously; the tree event proves the diff landed.
    shared.lock().unwrap().push(rename_save_update());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut saw_tree_changed = None;
    let mut saw_frame_presented = false;
    while tokio::time::Instant::now() < deadline
        && (saw_tree_changed.is_none() || !saw_frame_presented)
    {
        let event = tokio::time::timeout_at(deadline, client.next_event())
            .await
            .expect("events arrive without any request")
            .expect("event frame");
        match event {
            GwpEvent::V1(EventV1::TreeChanged { seq }) => {
                saw_tree_changed = Some(seq);
            }
            GwpEvent::V1(EventV1::FramePresented { seq, .. }) => {
                saw_frame_presented = true;
                assert!(seq >= 1);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
    let tree_seq = saw_tree_changed.expect("TreeChanged arrived");
    assert!(tree_seq >= 1, "the seq space the TreeSnapshot reports");
    assert!(
        saw_frame_presented,
        "FramePresented arrived (one pass per pump)"
    );

    // The subscription survives alongside normal traffic: a ping still
    // round-trips (the demux skips any event frames interleaved).
    client.ping().await.expect("ping round-trips");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribed_connection_keeps_the_conversation_event_free() {
    let path = socket_path("events-none.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // The tree changes while this connection is attached — it never
    // subscribed, so no event frame may arrive (the pre-M12 contract).
    shared.lock().unwrap().push(rename_save_update());
    client.ping().await.expect("ping round-trips");

    let next = tokio::time::timeout(Duration::from_millis(400), client.next_event()).await;
    assert!(
        next.is_err(),
        "an unsubscribed connection must never receive events"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_kind_filter_is_honored() {
    let path = socket_path("events-filter.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    client
        .subscribe(&[EventKindV1::FramePresented])
        .await
        .expect("subscribe to frames only");

    shared.lock().unwrap().push(rename_save_update());

    // The scripted update produces BOTH a tree change and frames; only
    // frame events may cross the filter. The pump presents one pass per
    // tick, so frames flow continuously — drain a window of events and
    // assert none of them is the filtered-out kind.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(700);
    let mut received = 0;
    while tokio::time::Instant::now() < deadline {
        let event = tokio::time::timeout_at(deadline, client.next_event()).await;
        let Ok(event) = event else { break }; // quiet window: done
        let GwpEvent::V1(inner) = event.expect("event frame");
        assert!(
            matches!(inner, EventV1::FramePresented { .. }),
            "the filter let a non-frame event through: {inner:?}"
        );
        received += 1;
    }
    assert!(received >= 1, "the frame event arrived");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn re_subscribing_replaces_the_filter() {
    let path = socket_path("events-replace.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    client
        .subscribe(&[EventKindV1::TreeChanged])
        .await
        .expect("first subscription");
    // Re-subscribe: replaces, not appends (the ack echoes the new filter).
    let ack = client
        .request(RequestV1::Subscribe(SubscribeV1::On {
            kinds: vec![EventKindV1::FramePresented],
        }))
        .await
        .expect("re-subscribe");
    assert_eq!(
        ack,
        ResponseV1::Subscribed {
            kinds: vec![EventKindV1::FramePresented]
        },
        "the ack echoes the deduplicated kinds now active"
    );

    shared.lock().unwrap().push(rename_save_update());
    let first = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the frame event arrives")
        .expect("event frame");
    assert!(
        matches!(first, GwpEvent::V1(EventV1::FramePresented { .. })),
        "the replaced filter dropped TreeChanged: {first:?}"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribing_stops_delivery() {
    let path = socket_path("events-off.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    client
        .subscribe(&[EventKindV1::TreeChanged])
        .await
        .expect("subscribe");
    client.unsubscribe().await.expect("unsubscribe ack");

    shared.lock().unwrap().push(rename_save_update());
    client.ping().await.expect("ping round-trips");

    let next = tokio::time::timeout(Duration::from_millis(400), client.next_event()).await;
    assert!(next.is_err(), "no events flow after `Off`");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_log_subscription_is_an_honest_unsupported() {
    let path = socket_path("events-log.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // All-or-nothing: naming an unsupported kind fails the whole request
    // (the M8 typed-error discipline — a capability answer, never a
    // string match or a silent no-op filter).
    let err = client
        .request(RequestV1::Subscribe(SubscribeV1::On {
            kinds: vec![EventKindV1::TreeChanged, EventKindV1::Log],
        }))
        .await
        .expect_err("the log kind is not served");
    let GwpError::Rejected { code, message } = err else {
        panic!("expected a typed rejection, got {err:?}");
    };
    assert_eq!(code, ErrorCode::Unsupported);
    assert!(
        message.contains("log"),
        "the message names the gap: {message}"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_flow_over_tcp_too() {
    // The §7 transports are byte-identical: the TCP door carries the same
    // subscription conversation (the M3 other-language-client path).
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle: GwpServerHandle = GwpServerHandle::spawn_tcp(
        "127.0.0.1:0".parse().unwrap(),
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
        SeamRegistry::new(),
    )
    .expect("server binds");
    let addr = match handle.endpoint() {
        Endpoint::Tcp(addr) => addr.to_string(),
        other => panic!("expected a TCP endpoint, got {other:?}"),
    };
    let mut client = TcpClient::connect(&addr).await.expect("client connects");
    client.handshake("@guiwright/gwp-e2e").await.unwrap();

    client
        .subscribe(&[EventKindV1::TreeChanged])
        .await
        .expect("subscribe over tcp");

    shared.lock().unwrap().push(rename_save_update());

    let first = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the event arrives over tcp")
        .expect("event frame");
    assert!(
        matches!(first, GwpEvent::V1(EventV1::TreeChanged { .. })),
        "{first:?}"
    );

    handle.shutdown();
}
