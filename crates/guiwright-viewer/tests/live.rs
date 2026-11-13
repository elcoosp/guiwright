//! The viewer live mode end to end (architecture §12, M18): a
//! [`LiveFeed`] attached to a running GWP session builds the same
//! scrubber timeline the archive path does — one tick per presented
//! frame, the UI tree resolved from the snapshots the feed fetches as
//! the session changes. The doors are the M15/M16 transports; the
//! fixture-backed tests run over the in-process duplex door with the
//! driver's mock adapter, plus a session-less scripted door that pins
//! the wire dance (attach → subscribe → dump fetch → quiet pump) without
//! a session thread.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, NodeKeyV1, PROTO_VERSION, RequestV1, ResponseV1, SnapshotNodeV1,
    SnapshotV1,
};
use guiwright_proto::{Handler, duplex};
use guiwright_viewer::ViewerError;
use guiwright_viewer::live::{LiveChange, LiveFeed, LiveTimeline};

/// A scripted door (no session thread): Hello → Welcome, Subscribe →
/// Subscribed, `Snapshot::Full` → a fixed three-row dump, Ping → Pong, and
/// never an event — the deterministic wire dance behind the feed.
struct ScriptedDoor {
    dumps: Arc<AtomicUsize>,
}

impl Handler for ScriptedDoor {
    fn handle(
        &mut self,
        request: RequestV1,
    ) -> impl std::future::Future<Output = ResponseV1> + Send {
        let dumps = Arc::clone(&self.dumps);
        let response = match request {
            RequestV1::Hello { .. } => ResponseV1::Welcome {
                proto_version: PROTO_VERSION,
                session_id: "scripted".into(),
                tree_seq: 7,
                frame_seq: 3,
                adapter: AdapterInfoV1 {
                    name: "scripted-adapter".into(),
                    framework: "fixture".into(),
                    framework_version: "0".into(),
                    caps: CapsV1::default(),
                },
            },
            RequestV1::Subscribe(_) => ResponseV1::Subscribed {
                kinds: vec![guiwright_proto::schema::EventKindV1::TreeChanged],
            },
            RequestV1::Snapshot(SnapshotV1::Full) => {
                dumps.fetch_add(1, Ordering::SeqCst);
                ResponseV1::TreeDump {
                    tree_seq: 7,
                    node_count: 3,
                    focus: Some(NodeKeyV1::AccessKit {
                        tree: [0u8; 16],
                        node: 2,
                    }),
                    nodes: vec![
                        SnapshotNodeV1 {
                            key: NodeKeyV1::AccessKit {
                                tree: [0u8; 16],
                                node: 1,
                            },
                            role: "Window".into(),
                            name: Some("Scripted App".into()),
                            value: None,
                            bounds: Some([0.0, 0.0, 480.0, 240.0]),
                            children: 2,
                        },
                        SnapshotNodeV1 {
                            key: NodeKeyV1::AccessKit {
                                tree: [0u8; 16],
                                node: 2,
                            },
                            role: "Button".into(),
                            name: Some("Save".into()),
                            value: None,
                            bounds: Some([10.0, 10.0, 80.0, 24.0]),
                            children: 0,
                        },
                        SnapshotNodeV1 {
                            key: NodeKeyV1::AccessKit {
                                tree: [0u8; 16],
                                node: 3,
                            },
                            role: "Button".into(),
                            name: Some("Cancel".into()),
                            value: None,
                            bounds: Some([100.0, 10.0, 80.0, 24.0]),
                            children: 0,
                        },
                    ],
                }
            }
            RequestV1::Ping => ResponseV1::Pong,
            _ => ResponseV1::Error {
                code: guiwright_proto::schema::ErrorCode::Unsupported,
                message: "not implemented in the scripted door".into(),
            },
        };
        async move { response }
    }
}

/// The fixture-backed session: the feed attaches over the embedded door,
/// syncs the tree, and the resolved view carries the fixture's buttons.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_feed_syncs_the_fixture_tree() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    let mut feed = LiveFeed::new(client);
    let info = feed.attach("viewer-live-test/0.1").await.expect("attach");
    assert_eq!(info.adapter.name, "mock");
    assert_eq!(info.adapter.framework, "fixture");

    let frame = feed.sync_tree().await.expect("sync");
    assert_eq!(frame, feed.current_frame());

    let view = feed.tree_at(frame).expect("the synced frame resolves");
    assert_eq!(view.nodes.len(), 8, "the fixture's full tree");
    let save = view
        .nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("Save"))
        .expect("the Save button row");
    assert_eq!(save.role, "Button");

    feed.bye().await.expect("bye");
    handle.shutdown();
}

/// A mid-session rename through the mock's shared-script channel: the
/// feed pumps the pushed `TreeChanged`, auto-syncs, and the resolved
/// tree at the change frame carries the new label.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_feed_tracks_a_mid_session_rename() {
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    let mut feed = LiveFeed::new(client);
    let info = feed.attach("viewer-live-rename/0.1").await.expect("attach");
    assert_eq!(info.adapter.name, "mock");
    let before = feed
        .tree_at(feed.current_frame())
        .expect("the initial dump resolves a view");
    assert!(
        !before
            .nodes
            .iter()
            .any(|n| n.label.as_deref() == Some("Save (renamed)")),
        "the rename has not landed yet"
    );

    // App-side change mid-session: the rename lands on the session's
    // next pump and arrives as a pushed TreeChanged.
    let mut save = Node::new(Role::Button);
    save.set_label("Save (renamed)");
    shared.lock().expect("scripts lock").push(TreeUpdate {
        nodes: vec![(NodeId(2), save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    });

    let mut change_frame = None;
    for _ in 0..64 {
        match feed.pump(Duration::from_secs(5)).await.expect("pump") {
            Some(LiveChange::TreeChanged { frame }) => {
                change_frame = Some(frame);
                break;
            }
            Some(_) => {}
            None => panic!("quiet window while a rename was pending"),
        }
    }
    let frame = change_frame.expect("the TreeChanged event arrives");

    let view = feed.tree_at(frame).expect("the change frame resolves");
    let renamed = view
        .nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("Save (renamed)"))
        .expect("the renamed row after the auto-sync");
    assert_eq!(renamed.role, "Button");

    feed.bye().await.expect("bye");
    handle.shutdown();
}

/// The session-less wire dance over any door: attach handshakes and
/// subscribes, `sync_tree` fetches the dump once, the timeline resolves
/// the scripted rows with rebuilt parent links, and a pump with nothing
/// coming answers honestly quiet instead of hanging.
#[tokio::test]
async fn live_feed_records_a_scripted_dump_without_a_session() {
    let (listener, connect) = duplex::channel();
    let dumps = Arc::new(AtomicUsize::new(0));
    let d2 = Arc::clone(&dumps);
    tokio::spawn(async move {
        duplex::serve(listener, move || ScriptedDoor {
            dumps: Arc::clone(&d2),
        })
        .await;
    });

    let client = DuplexClient::connect(&connect).await.expect("connect");
    let mut feed = LiveFeed::new(client);
    let info = feed.attach("scripted-viewer/0.1").await.expect("attach");
    assert_eq!(info.adapter.name, "scripted-adapter");
    assert_eq!(
        dumps.load(Ordering::SeqCst),
        1,
        "attach takes the initial dump"
    );

    let frame = feed.sync_tree().await.expect("sync");
    assert_eq!(
        frame, 0,
        "the axis starts at the feed's origin until frames arrive"
    );
    assert_eq!(dumps.load(Ordering::SeqCst), 2, "sync takes one more");

    let view = feed.tree_at(frame).expect("the scripted view");
    assert_eq!(view.nodes.len(), 3);
    assert_eq!(view.focus, Some(2));
    let save = view.nodes.iter().find(|n| n.id == 2).expect("the Save row");
    assert_eq!(
        save.parent,
        Some(1),
        "the DFS children counts rebuild parents"
    );
    let cancel = view
        .nodes
        .iter()
        .find(|n| n.id == 3)
        .expect("the Cancel row");
    assert_eq!(cancel.parent, Some(1), "second child of the same root");

    let quiet = feed
        .pump(Duration::from_millis(80))
        .await
        .expect("pump never errors on a quiet door");
    assert!(quiet.is_none(), "no events are coming: {quiet:?}");

    feed.bye().await.expect("bye");
}

/// The sync accumulation model (no tokio): frames build ticks, events
/// stamp against the current frame, and `timeline()` hands the same
/// scrubber model the archive path builds.
#[test]
fn live_timeline_resolves_frames_from_recorded_parts() {
    let mut timeline = LiveTimeline::new("unit-demo");
    timeline.record_frame(1, 0xAA);
    let rows = vec![
        SnapshotNodeV1 {
            key: NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: 1,
            },
            role: "Window".into(),
            name: Some("App".into()),
            value: None,
            bounds: None,
            children: 1,
        },
        SnapshotNodeV1 {
            key: NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: 2,
            },
            role: "Button".into(),
            name: Some("Only".into()),
            value: None,
            bounds: Some([0.0, 0.0, 10.0, 10.0]),
            children: 0,
        },
    ];
    let frame = timeline.record_snapshot(
        &rows,
        Some(&NodeKeyV1::AccessKit {
            tree: [0u8; 16],
            node: 2,
        }),
    );
    assert_eq!(frame, 1);

    timeline.record_frame(2, 0xBB);
    timeline.record_event("click", "click at (12, 14)");
    timeline.record_log("{\"message\":\"renamed\"}");

    let scrubber = timeline.timeline();
    let seqs: Vec<u64> = scrubber.ticks.iter().map(|t| t.frame_seq).collect();
    assert_eq!(seqs, vec![1, 2], "the recorded union, sorted");
    assert_eq!(scrubber.ticks[0].hash, Some(0xAA));
    assert_eq!(scrubber.ticks[1].events.len(), 1, "the event rides frame 2");

    let view = scrubber.tree_at(2).expect("resolved from the snapshot");
    assert_eq!(view.snapshot_frame, Some(1));
    assert_eq!(view.nodes.len(), 2);
    assert_eq!(view.nodes[1].parent, Some(1));
    assert_eq!(view.focus, Some(2));
    assert_eq!(timeline.snapshot_count(), 1);
}

/// A live feed whose door closes behind it fails honestly at the next
/// pump — never a silent, frozen scrubber. `Bye` ends the connection
/// loop server-side, so the pipe reads EOF and the feed reports it.
#[tokio::test]
async fn pump_fails_honestly_when_the_door_closes() {
    let (listener, connect) = duplex::channel();
    let dumps = Arc::new(AtomicUsize::new(0));
    let d2 = Arc::clone(&dumps);
    tokio::spawn(async move {
        duplex::serve(listener, move || ScriptedDoor {
            dumps: Arc::clone(&d2),
        })
        .await;
    });

    let client = DuplexClient::connect(&connect).await.expect("connect");
    let mut feed = LiveFeed::new(client);
    feed.attach("closing-viewer/0.1").await.expect("attach");
    feed.bye().await.expect("bye ends the conversation");

    let err = feed
        .pump(Duration::from_secs(5))
        .await
        .expect_err("a closed door is a pump error, not a hang");
    assert!(
        matches!(err, ViewerError::Wire(_)),
        "the wire error rides the viewer's Wire family, got {err:?}"
    );
}

/// The §12 follow-on (M23): the emitter is on the live surface — attach
/// records the adapter facts the header names, and the accumulated model
/// renders the same self-contained scrubber the archive path emits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_records_the_adapter_and_the_model_renders_html() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let client = DuplexClient::connect(&connect)
        .await
        .expect("client connects");
    let mut feed = LiveFeed::new(client);
    feed.attach("viewer-live-html/0.1").await.expect("attach");
    assert_eq!(
        feed.live_timeline().adapter(),
        Some("mock"),
        "the attach report feeds the emitter's header"
    );

    let html = feed.render_html();
    assert!(html.starts_with("<!doctype html>"), "the scrubber shape");
    assert!(html.contains("viewer-live-html/0.1"), "the test label");
    assert!(html.contains(r#""adapter":"mock""#), "the adapter name");
    assert!(html.contains("(live)"), "the live schema stamp");

    feed.bye().await.expect("bye");
    handle.shutdown();
}
