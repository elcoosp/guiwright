//! The resolve-all wire domain (M30 — the m22/m26/m29 records' named
//! protocol half): the §8.2 set read — `locator.count()` /
//! `locator.all()`'s `resolve_all` walk — as an append-only request /
//! reply pair over GWP, with the typed [`ClientConn::resolve_all`]
//! sugar riding the client seam. Driven here over the in-process door
//! with the four-button fixture (the ambiguity strict mode would reject
//! single resolves with is the point of the set read) and the
//! two-window fixture (the scope discriminator).

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{
    NodeKeyV1, RequestV1, ResolvedMatchV1, ResponseV1, SelectorV1, SemActionV1,
};
use guiwright_proto::{ClientConn, DuplexClient};

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-bulk-read-test/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn_fixture() -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

fn spawn_two_window() -> (
    GwpServerHandle,
    guiwright_proto::duplex::DuplexConnect,
    String,
    [u8; 16],
) {
    let fx = MultiWindowFixture::default();
    let w2 = fx.w2.0.to_string();
    let w2_bytes = *fx.w2.0.as_bytes();
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_multi_window(&fx),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, w2, w2_bytes)
}

fn role_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(guiwright_proto::schema::TextMatchV1 {
            text: name.into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

fn all_buttons() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: None,
    }
}

async fn matches_of(client: &mut DuplexClient, selector: SelectorV1) -> Vec<ResolvedMatchV1> {
    match client
        .request(RequestV1::ResolveAll { selector })
        .await
        .expect("resolve-all round-trip")
    {
        ResponseV1::ResolvedAll { matches } => matches,
        other => panic!("expected ResolvedAll, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_all_enumerates_the_set_in_document_order() {
    let (_handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let matches = matches_of(&mut client, all_buttons()).await;
    assert_eq!(matches.len(), 4, "the fixture's four buttons");
    // Document order is the set read's contract (the count()/all()
    // ordering): the fixture lays Save, Cancel, Hidden, Confirm out in
    // attach order, so the rows follow it.
    let names: Vec<u64> = matches
        .iter()
        .map(|m| match &m.node {
            NodeKeyV1::AccessKit { node, .. } => *node,
            other @ NodeKeyV1::Logical { .. } => panic!("expected AccessKit keys, got {other:?}"),
        })
        .collect();
    // The fixture's button NodeIds in document order (test_util's
    // canonical diagram: Save 2, Cancel 3, Hidden 6, Confirm 7).
    assert_eq!(
        names,
        vec![2, 3, 6, 7],
        "document order, not resolution luck"
    );
    // Every row carries the handle facts a follow-up Action needs; the
    // rect rides the same rule the single Resolve answers with — the
    // bounds the resolve found, `None` when the node has none (the
    // fixture's hidden "Invisible" button, NodeId 6).
    for m in &matches {
        assert_eq!(m.generation, 0, "first generation, fresh tree");
        let is_hidden = matches!(m.node, NodeKeyV1::AccessKit { node, .. } if node == 6);
        if is_hidden {
            assert!(
                m.rect.is_none(),
                "the unbounded hidden row carries no rect: {m:?}"
            );
        } else {
            assert!(
                m.rect.is_some_and(|[x0, y0, x1, y1]| x1 > x0 && y1 > y0),
                "bounded nodes carry their rect: {m:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_set_read_is_total_over_the_empty_set() {
    let (_handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let matches = matches_of(&mut client, role_button("Ghost")).await;
    assert!(
        matches.is_empty(),
        "an empty set answers with an empty list, never an error"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_all_never_consults_strict_mode() {
    let (_handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // Four buttons would trip a single Resolve with StrictModeViolation;
    // the set read is the escape hatch by definition (the count()
    // contract over the wire).
    let matches = matches_of(&mut client, all_buttons()).await;
    assert_eq!(matches.len(), 4, "ambiguity is the set read's data");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_all_rides_the_connection_window_scope() {
    let (_handle, connect, w2, w2_bytes) = spawn_two_window();
    let mut client = connected(&connect).await;

    // Aim at Preview, then enumerate the ambiguous role: the connection's
    // active target scopes the walk (§16 v0.4), exactly like Resolve.
    client.window_use(&w2).await.expect("Preview aimed");
    let matches = matches_of(&mut client, all_buttons()).await;
    assert_eq!(matches.len(), 1, "exactly one Save inside Preview");
    match &matches[0].node {
        NodeKeyV1::AccessKit { tree, .. } => {
            assert_eq!(
                tree, &w2_bytes,
                "the enumerated row belongs to the aimed window"
            );
        }
        other @ NodeKeyV1::Logical { .. } => panic!("expected an AccessKit key, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resolved_all_row_is_a_direct_action_handle() {
    let (_handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The dump-row rule (M11): a row of an enumeration is directly
    // usable as a handle — resolve-all, then act on row 0 without a
    // second resolve.
    let matches = matches_of(&mut client, all_buttons()).await;
    let save = &matches[0];
    let done = client
        .request(RequestV1::Action {
            node: save.node.clone(),
            generation: save.generation,
            action: SemActionV1::Click,
            force: false,
            timeout_ms: 2000,
        })
        .await
        .expect("action on the enumerated row");
    match done {
        ResponseV1::ActionDone { frame_seq, .. } => assert!(frame_seq > 0, "applied at a frame"),
        other => panic!("expected ActionDone, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_all_rides_the_client_seam_not_a_transport() {
    // The sugar lives on the [`ClientConn`] trait, so a consumer written
    // against the seam — the viewer's live feed, a downstream recorder —
    // gets the set read over any door (the window-methods rule, M6/M18).
    async fn button_count<C: ClientConn>(conn: &mut C) -> usize {
        conn.resolve_all(all_buttons())
            .await
            .expect("the set read over the seam")
            .len()
    }

    let (_handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;
    assert_eq!(
        button_count(&mut client).await,
        4,
        "the four buttons through the generic seam"
    );
}
