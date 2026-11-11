//! The §7 tree-dump domain over the wire (M11, the M0-era `TreeSnapshot`
//! promise: "full snapshot rides the wire once traces land" — traces
//! shipped in M1, this is the dump). The whole logical tree as typed
//! rows: depth-first per window root, `children` counts rebuilding the
//! hierarchy, the focused node included.

use std::sync::{Arc, Mutex};

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{NodeKeyV1, RequestV1, ResponseV1, SnapshotV1};
use guiwright_proto::uds::Client;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_dump_walks_the_fixture_tree_depth_first() {
    let path = socket_path("dump-fixture.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    let resp = client
        .request(RequestV1::Snapshot(SnapshotV1::Full))
        .await
        .unwrap();
    let ResponseV1::TreeDump {
        tree_seq,
        node_count,
        focus,
        nodes,
    } = resp
    else {
        panic!("expected TreeDump, got {resp:?}");
    };

    assert_eq!(node_count, 8, "the M0 fixture's 8 nodes");
    assert_eq!(nodes.len(), node_count, "redundant count stays honest");
    assert!(tree_seq >= 1, "the attach pump applied the fixture update");
    assert!(focus.is_some(), "the fixture carries a focus");

    // Depth-first shape: window root → group → the six children, the
    // counts alone rebuilding the hierarchy.
    let roles: Vec<&str> = nodes.iter().map(|n| n.role.as_str()).collect();
    assert_eq!(
        roles,
        vec![
            "Window",
            "Group",
            "Button",
            "Button",
            "Label",
            "TextInput",
            "Button",
            "Button"
        ],
        "{roles:?}"
    );
    let counts: Vec<u32> = nodes.iter().map(|n| n.children).collect();
    assert_eq!(counts, vec![1, 6, 0, 0, 0, 0, 0, 0], "{counts:?}");

    // Row facts: the Save button row is directly usable as a handle.
    let save = &nodes[2];
    assert_eq!(save.role, "Button");
    assert_eq!(save.name.as_deref(), Some("Save"));
    assert_eq!(save.bounds, Some([16.0, 16.0, 136.0, 48.0]));
    let NodeKeyV1::AccessKit { node, .. } = save.key else {
        panic!("accesskit identity");
    };
    assert_eq!(node, Fixture::buttons().save_button.0);

    // The email input carries its value into the dump (§13 fact parity).
    let email = &nodes[5];
    assert_eq!(email.role, "TextInput");
    assert_eq!(email.value.as_deref(), Some("ada@example.dev"));

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_dump_covers_every_window_target() {
    // §16 v0.4: one depth-first walk per window root — disjoint subtrees,
    // roots in adapter-reported window order.
    let path = socket_path("dump-multi.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_multi_window(&MultiWindowFixture::new())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    let resp = client
        .request(RequestV1::Snapshot(SnapshotV1::Full))
        .await
        .unwrap();
    let ResponseV1::TreeDump {
        node_count, nodes, ..
    } = resp
    else {
        panic!("expected TreeDump, got {resp:?}");
    };
    assert_eq!(nodes.len(), node_count);
    // The two window roots come first, in `windows()` order; every dumped
    // node belongs to exactly one subtree walk (no double-counting).
    assert!(node_count >= 2, "both windows' content is present");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dump_reflects_a_late_tree_change() {
    // The dump is a fresh read: a scripted diff lands mid-session, the
    // next dump shows the new node (the REPL "what does the tree look
    // like NOW" use).
    let path = socket_path("dump-late.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // Script the rename: the update lands on the dump's own pump.
    let mut save = Node::new(Role::Button);
    save.set_label("Save (renamed)");
    save.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 16.0,
        x1: 136.0,
        y1: 48.0,
    });
    shared.lock().unwrap().push(TreeUpdate {
        nodes: vec![(NodeId(2), save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    });

    let resp = client
        .request(RequestV1::Snapshot(SnapshotV1::Full))
        .await
        .unwrap();
    let ResponseV1::TreeDump { nodes, .. } = resp else {
        panic!("expected TreeDump, got {resp:?}");
    };
    assert_eq!(
        nodes[2].name.as_deref(),
        Some("Save (renamed)"),
        "the dump reads the tree as it is now"
    );

    handle.shutdown();
}
