//! The §7 resume vocabulary over the wire (the m18 record's remaining-gaps
//! list: *"The feed is one-shot per connection. After `Bye` the connection
//! is done; a reconnecting viewer re-attaches (and re-dumps) — the
//! session-side resume vocabulary (a `Subscribe` replay cursor) stays a
//! §7 follow-on"*): a re-attaching client sends `SubscribeV1::Resume`
//! carrying the seqs it last saw, and the session replays the *position*
//! past the cursor — the coalesced current-seq notification, the honest
//! replay shape (the events are "the tree/frame is at seq N now", so the
//! replay is where the session stands, not the missed history; the seq
//! gap says exactly how far behind the client is, §15.6). A cursor that
//! already names the current position replays nothing — the reconnecting
//! viewer skips the re-dump it used to pay unconditionally.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{
    EventKindV1, EventV1, GwpEvent, RequestV1, ResponseV1, ResumeCursorV1, SubscribeV1,
};
use guiwright_proto::uds::Client;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

/// Script a rename of the fixture's Save button (the late-change pattern
/// from `tests/events.rs`): one pump integrates the diff.
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

async fn connect(path: &std::path::Path) -> Client {
    let mut client = Client::connect(path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();
    client
}

/// The first feed's view: subscribe, ride one real change, remember the
/// seqs it saw (the cursor a re-attach carries).
async fn first_feed_records_cursor(
    path: &std::path::Path,
    shared: &Arc<Mutex<Vec<TreeUpdate>>>,
) -> ResumeCursorV1 {
    let mut client = connect(path).await;
    client
        .subscribe(&[EventKindV1::TreeChanged])
        .await
        .expect("subscribe");
    shared.lock().unwrap().push(rename_save_update());
    let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the change arrives")
        .expect("event frame");
    let GwpEvent::V1(EventV1::TreeChanged { seq }) = event else {
        panic!("expected a tree event, got {event:?}");
    };
    // The connection ends the m18 way: dropped (or closed) — the feed is
    // one-shot per connection. The cursor is what it left behind.
    let cursor = ResumeCursorV1 {
        tree_seq: seq,
        frame_seq: 0,
    };
    drop(client);
    cursor
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_replays_the_position_past_the_cursor() {
    let path = socket_path("events-resume.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let cursor = first_feed_records_cursor(&path, &shared).await;
    let last_seen = cursor.tree_seq;

    // The app moves on while nobody is attached (the mid-disconnect
    // change the reconnecting viewer missed).
    shared.lock().unwrap().push(rename_save_update());

    // The re-attach: resume from the cursor — the session answers with
    // the position replay before the ack, so `next_event` yields it from
    // the interleaved buffer.
    let mut client = connect(&path).await;
    let ack = client
        .request(RequestV1::Subscribe(SubscribeV1::Resume {
            kinds: vec![EventKindV1::TreeChanged],
            cursor,
        }))
        .await
        .expect("resume subscribes");
    assert_eq!(
        ack,
        ResponseV1::Subscribed {
            kinds: vec![EventKindV1::TreeChanged]
        },
        "the ack echoes the honored kinds"
    );

    // The replay: the current position, past the cursor — the client
    // learns it is behind without waiting for the next change.
    let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the replayed position arrives")
        .expect("event frame");
    let GwpEvent::V1(EventV1::TreeChanged { seq }) = event else {
        panic!("expected the replayed tree position, got {event:?}");
    };
    assert!(
        seq > last_seen,
        "the replay names a position past the cursor: {seq} > {last_seen}"
    );

    // The live stream continues after the replay: the next change flows.
    shared.lock().unwrap().push(rename_save_update());
    let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the live event arrives")
        .expect("event frame");
    let GwpEvent::V1(EventV1::TreeChanged { seq: live }) = event else {
        panic!("expected a live tree event, got {event:?}");
    };
    assert!(
        live >= seq,
        "live events only move forward: {live} >= {seq}"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_current_cursor_replays_nothing() {
    // The quiet re-attach — the capability the re-dump used to bill
    // unconditionally: the cursor names the current position, so no
    // replay fires and the reconnecting viewer skips the dump.
    let path = socket_path("events-resume-quiet.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let cursor = first_feed_records_cursor(&path, &shared).await;

    // Nothing changed since the cursor — it names the current position.
    let mut client = connect(&path).await;
    let ack = client
        .request(RequestV1::Subscribe(SubscribeV1::Resume {
            kinds: vec![EventKindV1::TreeChanged],
            cursor,
        }))
        .await
        .expect("resume subscribes");
    assert_eq!(
        ack,
        ResponseV1::Subscribed {
            kinds: vec![EventKindV1::TreeChanged]
        }
    );

    // A quiet window: no tree event may arrive (frames are filtered out;
    // the tree stands still, so the replay has nothing to say).
    shared.lock().unwrap().clear();
    let next = tokio::time::timeout(Duration::from_millis(400), client.next_event()).await;
    assert!(
        next.is_err(),
        "a current cursor must replay nothing — got {next:?}"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_resume_honors_the_kind_filter() {
    // The replay rides the same filter the live stream does: a cursor
    // behind on the tree but resuming with a frame-only filter must not
    // leak the tree position.
    let path = socket_path("events-resume-filter.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let cursor = first_feed_records_cursor(&path, &shared).await;
    shared.lock().unwrap().push(rename_save_update());

    let mut client = connect(&path).await;
    client
        .request(RequestV1::Subscribe(SubscribeV1::Resume {
            kinds: vec![EventKindV1::FramePresented],
            cursor,
        }))
        .await
        .expect("resume subscribes");

    // Drain a window: every event is a frame — the stale tree cursor
    // never crosses the filter.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut received = 0;
    while tokio::time::Instant::now() < deadline {
        let event = tokio::time::timeout_at(deadline, client.next_event()).await;
        let Ok(event) = event else { break }; // quiet window: done
        let GwpEvent::V1(inner) = event.expect("event frame");
        assert!(
            matches!(inner, EventV1::FramePresented { .. }),
            "the resume filter let a tree event through: {inner:?}"
        );
        received += 1;
    }
    assert!(received >= 1, "the frame replay arrived");
    handle.shutdown();
}
