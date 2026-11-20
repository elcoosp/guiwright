//! The identity read's state facts at set width (m70): the m68/m69
//! records' named standing gap — "no state facts at *set* width: the
//! `DescribeAllIdentity` rows stay identity-row shaped; a state-fact set
//! widening lands when a consumer asks — the same append-only move, its
//! own domain" — closed by the appended
//! `DescribeAllIdentityState` / `DescribedAllIdentityState` domain. The
//! M39/m64 walk rides in and the M68 row rides out per match, so one
//! round trip answers "what does every node in this set say about itself
//! *and its state*" where the handle-width read answers one handle at a
//! time. Snapshot semantics are the `describe_all` family's (the m30
//! rule): the read never waits, an empty set answers with an empty list.
//! Driven here over the in-process door with the state fixture.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{CheckedStateV1, SelectorV1, TextMatchV1};

/// The checkbox role, un-named — the walk the set read runs (the
/// whole `CheckBox` match set, document order, the m30 rule).
fn checkboxes() -> SelectorV1 {
    SelectorV1::Role {
        role: "CheckBox".into(),
        name: None,
    }
}

/// One named row of one role — the narrowed walk the disabled-node
/// conversation reads through (the m68 test's shape, at set width).
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
        .handshake("wire-client-identity-state-all-test/0.1")
        .await
        .expect("handshake");
    client
}

/// The state fixture — the buttons base plus the checkbox trio (the M20
/// tri-state rows) and the described Install button (the M38 extension
/// band): every state fact the set read answers has a node that carries
/// it.
fn spawn_state() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::with_state()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

/// The checkbox walk answers the full tri-state per row, document order:
/// `Checked` / `Unchecked` / `Mixed` — one round trip instead of one
/// handle-width describe per row, the m68 row riding per match.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_checkbox_set_answers_the_tri_state_per_row() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let rows = client
        .describe_all_identity_state(checkboxes())
        .await
        .expect("the set read enumerates");
    assert_eq!(rows.len(), 3, "the trio rides one walk, document order");
    let checked: Vec<_> = rows
        .iter()
        .map(|row| (row.name.as_deref(), &row.checked))
        .collect();
    assert_eq!(
        checked,
        vec![
            (Some("Notifications"), &CheckedStateV1::Checked),
            (Some("Dark mode"), &CheckedStateV1::Unchecked),
            (Some("Partially installed"), &CheckedStateV1::Mixed),
        ],
        "the tri-state rides per row, never squeezed into a bool"
    );
    for row in &rows {
        assert!(
            row.enabled,
            "the trio's checkboxes are live — a total fact per row"
        );
        assert!(
            !row.focused,
            "the tree's focus is the window root, not a checkbox"
        );
        assert_eq!(
            row.description, None,
            "the absent fact is None, never the name or the value"
        );
    }

    handle.shutdown();
}

/// The identity five ride beside the state three: the handle echoes the
/// resolve row's (a row is directly usable as the `Action` node, the
/// dump-row rule), and the narrower M64 row projects off the wider one —
/// the m64 projection chain's fourth link.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_set_rows_carry_the_identity_row_beside_the_state_three() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let resolved = client
        .resolve_all(named_row("CheckBox", "Notifications"))
        .await
        .expect("the set read enumerates");
    let minted = resolved.first().expect("the checked checkbox resolves");
    let rows = client
        .describe_all_identity_state(checkboxes())
        .await
        .expect("the set read enumerates");
    let row = rows
        .iter()
        .find(|row| row.name.as_deref() == Some("Notifications"))
        .expect("the checked checkbox rides the walk");
    assert_eq!(row.node, minted.node, "the handle echoes the tree's id");
    assert_eq!(row.generation, minted.generation);
    assert_eq!(row.role, "CheckBox");
    let identity = guiwright_proto::schema::DescribedIdentityV1::from(row);
    assert_eq!(identity.node, row.node, "the projection keeps the handle");
    assert_eq!(identity.description, None);
    assert!(
        identity.name.is_some(),
        "the identity row is readable off the state row"
    );

    handle.shutdown();
}

/// A narrowed walk answering one disabled row: `enabled: false` beside
/// `NotCheckable` — the total read, never an error (the m38 rule at set
/// width, the m68 arm's facts riding the walk).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disabled_row_answers_enabled_false_and_not_checkable() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let rows = client
        .describe_all_identity_state(named_row("Button", "Confirm"))
        .await
        .expect("the set read enumerates");
    assert_eq!(rows.len(), 1, "the narrowed walk answers the one match");
    let described = rows.first().expect("the disabled button resolves");
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

/// An empty set answers with an empty list — the set read is total,
/// never an error (the m30 rule, the describe-all family's semantics
/// carried unchanged).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_set_answers_an_empty_list() {
    let (handle, connect) = spawn_state();
    let mut client = connected(&connect).await;

    let rows = client
        .describe_all_identity_state(named_row("CheckBox", "No such toggle"))
        .await
        .expect("the set read answers");
    assert!(
        rows.is_empty(),
        "the empty set is the answer, never an error"
    );

    handle.shutdown();
}
