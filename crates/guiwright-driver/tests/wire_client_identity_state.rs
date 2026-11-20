//! The identity read's state facts at handle width (m68): the m67
//! record's named standing gap — "no state flags on the identity row:
//! the checked/focused/enabled facts ride the dump rows and the kit
//! assertion vocabulary" — closed by the appended
//! `DescribeIdentityState` / `DescribedIdentityState` domain. The M36
//! handle pair rides in and the M68 row rides out (the M64 identity
//! row's fields plus the state three), so one handle-addressed round
//! trip answers "what does this node say about itself *and its state*"
//! where the assertion vocabulary answers one question at a time. The
//! handle guards are the [`RequestV1::Action`] boundary's exact typed
//! errors. Driven here over the in-process door with the state fixture.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{CheckedStateV1, ErrorCode, SelectorV1, TextMatchV1};

/// One named row of one role — a narrowed set whose head is that row
/// alone (the m66 test's `named_button`, generalized over the role so
/// the checkbox trio resolves the same way).
fn named_row(role: &str, name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-identity-state-test/0.1")
        .await
        .expect("handshake");
    client
}

/// The state fixture — the buttons base plus the checkbox trio (the M20
/// tri-state rows) and the described Install button (the M38 extension
/// band): every state fact the read answers has a node that carries it.
fn spawn_state() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::with_state()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

/// The checkbox trio answers the full tri-state beside the identity row:
/// `Checked` / `Unchecked` / `Mixed` — the vocabulary word, never an
/// `Option<bool>` (a mixed checkbox is not an unchecked one).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_checkbox_trio_answers_the_tri_state() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let rows = client
        .resolve_all(named_row("CheckBox", "Notifications"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the checked checkbox resolves");
    let described = client
        .describe_identity_state(row.node.clone(), row.generation)
        .await
        .expect("the state read answers the handle");
    assert_eq!(described.node, row.node, "the handle echoes the request's");
    assert_eq!(described.generation, row.generation);
    assert_eq!(described.role, "CheckBox");
    assert_eq!(described.name.as_deref(), Some("Notifications"));
    assert_eq!(
        described.checked,
        CheckedStateV1::Checked,
        "the tri-state rides beside the identity four"
    );
    assert!(described.enabled, "a live checkbox is enabled");
    assert!(!described.focused, "the tree's focus is the window root");

    let rows = client
        .resolve_all(named_row("CheckBox", "Dark mode"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the unchecked checkbox resolves");
    let described = client
        .describe_identity_state(row.node.clone(), row.generation)
        .await
        .expect("the state read answers the handle");
    assert_eq!(described.checked, CheckedStateV1::Unchecked);

    let rows = client
        .resolve_all(named_row("CheckBox", "Partially installed"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the mixed checkbox resolves");
    let described = client
        .describe_identity_state(row.node.clone(), row.generation)
        .await
        .expect("the state read answers the handle");
    assert_eq!(
        described.checked,
        CheckedStateV1::Mixed,
        "the mixed state is its own vocabulary word"
    );
    assert_eq!(
        described.description, None,
        "the absent fact is None, never the name or the value"
    );

    handle.shutdown();
}

/// The disabled node answers `enabled: false`, and a node that exposes
/// no checked state answers `NotCheckable` — the total read, never an
/// error (the m38 rule at state width).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disabled_node_answers_enabled_false_and_not_checkable() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let rows = client
        .resolve_all(named_row("Button", "Confirm"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the disabled button resolves");
    let described = client
        .describe_identity_state(row.node.clone(), row.generation)
        .await
        .expect("the state read answers the handle");
    assert!(
        !described.enabled,
        "the AccessKit Disabled flag reads as enabled: false"
    );
    assert_eq!(
        described.checked,
        CheckedStateV1::NotCheckable,
        "a button exposes no checked state — the fact, not an error"
    );

    handle.shutdown();
}

/// The focused fact names the tree's focus: the window root (the
/// fixture's seeded focus) reads `focused: true`, everything else
/// `focused: false` — the core `Check::Focused` read's shape
/// (`tree.focus()` equality).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_focus_fact_names_the_tree_focus() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let rows = client
        .resolve_all(SelectorV1::Role {
            role: "Window".into(),
            name: None,
        })
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the window root resolves");
    let described = client
        .describe_identity_state(row.node.clone(), row.generation)
        .await
        .expect("the state read answers the handle");
    assert!(described.focused, "the fixture seeds the focus at the root");

    let rows = client
        .resolve_all(named_row("Button", "Save"))
        .await
        .expect("the set read enumerates");
    let row = rows.first().expect("the Save button resolves");
    let described = client
        .describe_identity_state(row.node.clone(), row.generation)
        .await
        .expect("the state read answers the handle");
    assert!(
        !described.focused,
        "an unfocused node answers false, never an error"
    );

    handle.shutdown();
}

/// A handle the tree does not know rides the typed
/// `ErrorCode::UnknownNode` — the [`RequestV1::Action`] boundary's
/// honesty, the same error an action on the ghost row would meet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ghost_handle_rides_the_typed_unknown_node() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let ghost = guiwright_proto::schema::NodeKeyV1::AccessKit {
        tree: [0xa5; 16],
        node: 4242,
    };
    let err = client
        .describe_identity_state(ghost, 0)
        .await
        .expect_err("the tree cannot describe a node it does not know");
    match err {
        GwpError::Rejected { code, message } => {
            assert_eq!(code, ErrorCode::UnknownNode, "the typed code: {message}");
            assert!(
                message.contains("node"),
                "the boundary names the handle: {message}"
            );
        }
        other => panic!("expected the typed unknown-node error, got {other:?}"),
    }

    handle.shutdown();
}
