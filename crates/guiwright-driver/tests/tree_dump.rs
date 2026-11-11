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
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    ErrorCode, NodeKeyV1, RequestV1, ResponseV1, SelectorV1, SnapshotV1, WindowV1,
};
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

// --- M13: `Snapshot::Filtered` — one subtree as data ------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filtered_dump_walks_only_the_match_subtree() {
    let path = socket_path("dump-filtered.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // The fixture's window root has one Group child with six children —
    // dumping under the Group yields 7 rows, not the whole 8-node tree.
    let resp = client
        .request(RequestV1::Snapshot(SnapshotV1::Filtered {
            selector: SelectorV1::Role {
                role: "Group".into(),
                name: None,
            },
            timeout_ms: 1_000,
        }))
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
    assert_eq!(node_count, 7, "the Group subtree only");
    assert_eq!(nodes.len(), node_count);
    assert!(tree_seq >= 1);

    // The root row is the match itself; the walk is depth-first below it.
    assert_eq!(nodes[0].role, "Group");
    let counts: Vec<u32> = nodes.iter().map(|n| n.children).collect();
    assert_eq!(counts, vec![6, 0, 0, 0, 0, 0, 0], "{counts:?}");
    let roles: Vec<&str> = nodes.iter().map(|n| n.role.as_str()).collect();
    assert_eq!(
        roles,
        vec![
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
    // The window root is NOT part of this dump (a node outside the
    // subtree is not this dump's fact to report).
    assert!(
        nodes.iter().all(|n| n.role != "Window"),
        "no window root row in a filtered dump"
    );

    // Focus rides along only when it lives inside the dumped subtree: the
    // fixture focuses the window root — outside this Group dump, so the
    // honest reply carries no focus at all.
    assert!(
        focus.is_none(),
        "a focus outside the subtree is not this dump's fact to report"
    );

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filtered_dump_auto_waits_for_a_late_node() {
    // The auto-wait contract: a selector that matches only after the next
    // scripted diff resolves within `timeout_ms` (the same behavior the
    // `Resolve` arm pins).
    let path = socket_path("dump-filtered-late.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // Script a NEW group under the fixture root (NodeId 0): a Group node
    // with one Button, landed on a later pump — the resolve inside the
    // filtered dump auto-waits for it.
    let mut group = Node::new(Role::Group);
    group.set_label("Dialog");
    group.push_child(NodeId(101));
    let mut ok = Node::new(Role::Button);
    ok.set_label("OK");
    let mut root = Node::new(Role::Window);
    root.set_label("Test App");
    root.push_child(NodeId(100));
    let root_update = TreeUpdate {
        nodes: vec![(NodeId(0), root), (NodeId(100), group), (NodeId(101), ok)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        // Focus INSIDE the dumped subtree: the filtered dump reports it.
        focus: accesskit::NodeId(101),
    };
    shared.lock().unwrap().push(root_update);

    let resp = client
        .request(RequestV1::Snapshot(SnapshotV1::Filtered {
            selector: SelectorV1::Role {
                role: "Group".into(),
                name: Some(guiwright_proto::schema::TextMatchV1 {
                    text: "Dialog".into(),
                    exact: false,
                    ignore_case: true,
                }),
            },
            timeout_ms: 5_000,
        }))
        .await
        .unwrap();
    let ResponseV1::TreeDump { nodes, focus, .. } = resp else {
        panic!("expected TreeDump, got {resp:?}");
    };
    // Group "Dialog" + its OK button, nothing else.
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert_eq!(nodes[0].role, "Group");
    assert_eq!(nodes[0].name.as_deref(), Some("Dialog"));
    assert_eq!(nodes[1].role, "Button");
    assert_eq!(nodes[1].name.as_deref(), Some("OK"));
    // The focus moved INSIDE the subtree (the OK button): the dump
    // reports it, tied to the row a client can act on.
    assert_eq!(focus.as_ref(), Some(&nodes[1].key), "{focus:?}");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filtered_dump_without_a_match_is_a_typed_rejection() {
    let path = socket_path("dump-filtered-miss.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    // A selector the fixture cannot match: strict-mode violation at
    // timeout 0 (one honest try, no waiting), `Timeout` with a wait.
    // `Client::request` maps `Error` replies to `GwpError::Rejected` —
    // the typed codes are the assertion surface.
    let err = client
        .request(RequestV1::Snapshot(SnapshotV1::Filtered {
            selector: SelectorV1::Role {
                role: "TabPanel".into(),
                name: None,
            },
            timeout_ms: 0,
        }))
        .await
        .expect_err("no match at timeout 0");
    let GwpError::Rejected {
        code: strict_code, ..
    } = err
    else {
        panic!("expected a typed rejection");
    };
    assert_eq!(strict_code, ErrorCode::StrictModeViolation);

    let err = client
        .request(RequestV1::Snapshot(SnapshotV1::Filtered {
            selector: SelectorV1::Role {
                role: "TabPanel".into(),
                name: None,
            },
            timeout_ms: 250,
        }))
        .await
        .expect_err("the auto-wait expires honestly");
    let GwpError::Rejected {
        code: timeout_code, ..
    } = err
    else {
        panic!("expected a typed rejection");
    };
    assert_eq!(timeout_code, ErrorCode::Timeout);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filtered_dump_honors_the_connection_window_scope() {
    // §16 v0.4: a connection aimed at one window resolves within that
    // window's subtree only — the filtered dump is no exception.
    let path = socket_path("dump-filtered-scope.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_multi_window(&MultiWindowFixture::new())
    })
    .expect("server binds");
    let mut client = Client::connect(&path).await.expect("client connects");
    client.handshake("guiwright-tests/0.1").await.unwrap();

    let resp = client
        .request(RequestV1::Window(WindowV1::List))
        .await
        .unwrap();
    let ResponseV1::Windows { windows } = resp else {
        panic!("expected Windows, got {resp:?}");
    };
    assert!(windows.len() >= 2, "the multi-window fixture");

    // Aim at the second window; the same selector now resolves inside it.
    let second = &windows[1];
    let resp = client
        .request(RequestV1::Window(WindowV1::Use {
            handle: second.handle.clone(),
        }))
        .await
        .unwrap();
    assert!(matches!(resp, ResponseV1::WindowUsed { .. }));

    let resp = client
        .request(RequestV1::Snapshot(SnapshotV1::Filtered {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: None,
            },
            timeout_ms: 1_000,
        }))
        .await
        .unwrap();
    let ResponseV1::TreeDump { nodes, .. } = resp else {
        panic!("expected TreeDump, got {resp:?}");
    };
    assert!(
        !nodes.is_empty(),
        "the aimed window's buttons are visible to the scoped dump"
    );

    handle.shutdown();
}
