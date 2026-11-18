//! The hit-test wire domain (M32): the §8.3 "receives events" mechanism —
//! `TreeService::hit_test` — as an append-only request/reply pair over
//! GWP, with the typed [`ClientConn::hit_test`] sugar riding the client
//! seam. "What lives at this point" is the debugging read the REPL's
//! `hit` command and viewer-style clients speak; the point is physical px
//! in the tree's bounds space (the same space the dump rows' `bounds` and
//! the resolve rects report). The four-button fixture puts the Save
//! button at 16,16 → 136,48; the two-window fixture is the scope
//! discriminator (Editor's Save at 16,16, Preview's at 12,416).

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{NodeKeyV1, RequestV1, ResolvedMatchV1, ResponseV1, SemActionV1};
use guiwright_proto::{ClientConn, DuplexClient};

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-hit-test-test/0.1")
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

async fn hit(client: &mut DuplexClient, x: f64, y: f64) -> Option<ResolvedMatchV1> {
    match client
        .request(RequestV1::HitTest { x, y })
        .await
        .expect("hit-test round-trip")
    {
        ResponseV1::HitTest { hit } => hit,
        other => panic!("expected HitTest, got {other:?}"),
    }
}

#[tokio::test]
async fn hit_test_answers_the_node_under_the_point() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The Save button's center (16,16 → 136,48): identity, generation and
    // the bounds the resolve rects report, in one row.
    let hit = hit(&mut client, 76.0, 32.0)
        .await
        .expect("the save button sits under the point");
    let NodeKeyV1::AccessKit { tree, node } = &hit.node else {
        panic!("accesskit fixture keys");
    };
    assert_eq!(*tree, [0u8; 16], "the fixture rides the root tree id");
    assert_eq!(*node, 2, "the fixture's Save button");
    assert_eq!(hit.generation, 0);
    assert_eq!(hit.rect, Some([16.0, 16.0, 136.0, 48.0]));

    handle.shutdown();
}

#[tokio::test]
async fn a_miss_is_none_never_an_error() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // Past every bounded node: the read is total (the ResolveAll
    // totality rule, one point instead of a set).
    assert!(
        hit(&mut client, 400.0, 220.0).await.is_none(),
        "empty space answers None"
    );

    handle.shutdown();
}

#[tokio::test]
async fn a_hit_row_is_directly_usable_as_an_action_handle() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The dump-row rule (M11): the hit's node + generation is a live
    // Action handle — hit, then act, no second resolve.
    let hit = hit(&mut client, 76.0, 32.0)
        .await
        .expect("the save button sits under the point");
    let done = client
        .request(RequestV1::Action {
            node: hit.node.clone(),
            generation: hit.generation,
            action: SemActionV1::Click,
            force: false,
            timeout_ms: 2000,
        })
        .await
        .expect("action on the hit row");
    assert!(
        matches!(done, ResponseV1::ActionDone { ref detail, .. } if detail == "clicked"),
        "{done:?}"
    );

    handle.shutdown();
}

#[tokio::test]
async fn the_connection_scope_narrows_the_answer() {
    let (handle, connect, w2, w2_bytes) = spawn_two_window();
    let mut client = connected(&connect).await;

    // Aimed at Preview: Editor's Save (at 16,16) is outside the aimed
    // subtree — the same scoping the resolve paths run, so the hit reads
    // None instead of a cross-window answer.
    client.window_use(&w2).await.expect("Preview aimed");
    assert!(
        hit(&mut client, 76.0, 32.0).await.is_none(),
        "a hit outside the aimed window reads None"
    );
    let scoped = hit(&mut client, 72.0, 432.0)
        .await
        .expect("Preview's own Save answers under the scope");
    let NodeKeyV1::AccessKit { tree, node } = &scoped.node else {
        panic!("accesskit fixture keys");
    };
    assert_eq!(*tree, w2_bytes, "the Preview subtree");
    assert_eq!(*node, 2, "the Preview subtree's Save button");

    handle.shutdown();
}

#[tokio::test]
async fn the_client_sugar_rides_the_trait() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    // The ClientConn default method — the same surface every shipped
    // client and downstream transport inherits (the resolve_all pattern).
    let hit = client
        .hit_test(76.0, 32.0)
        .await
        .expect("the sugar round-trip")
        .expect("the save button sits under the point");
    assert_eq!(hit.generation, 0);
    let miss = client.hit_test(400.0, 220.0).await.expect("total read");
    assert!(miss.is_none(), "the sugar keeps the totality rule");

    handle.shutdown();
}
