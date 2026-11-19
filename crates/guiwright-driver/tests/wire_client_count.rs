//! The client seam's count-assertion rows over the wire (m59): the kit's
//! `expect(locator).to_have_count(...)` family (M22 exact + the M25
//! one-sided comparators) over any door, composed client-side as
//! [`ClientConn::expect_count`] / [`ClientConn::expect_count_greater_than`]
//! / [`ClientConn::expect_count_less_than`] — the m54 content assertions'
//! wake-loop composition riding the M30 resolve-all round trip instead of
//! describe-all, woken by the §7 event channel the way the m42 wait wakes.
//! **No wire change**: the assertion rides existing domains byte-for-byte;
//! every [`ClientConn`] impl inherits the rows. Driven here over the
//! in-process door with the four-button fixture and the shared-script
//! channel standing in for app-side UI changes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::TreeUpdate;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::SelectorV1;

/// Every `Button` the fixture carries — the set the count vocabulary
/// speaks about (the m22 record's whole-match-set read, strict mode
/// never consulted).
fn any_button() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: None,
    }
}

/// One named button — the count of a narrowed set.
fn named_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(guiwright_proto::schema::TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-count-test/0.1")
        .await
        .expect("handshake");
    client
}

fn spawn_fixture() -> (GwpServerHandle, DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

/// The buttons fixture behind a shared script channel — updates pushed
/// from another thread land on the session's next pump, exactly like
/// app-side UI changes riding a real adapter.
fn spawn_scripted() -> (GwpServerHandle, DuplexConnect, Arc<Mutex<Vec<TreeUpdate>>>) {
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_app = Arc::clone(&scripts);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts_for_app),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, scripts)
}

/// `expect_count` satisfied on entry: the four-button fixture answers 4
/// on the whole set, 1 on the narrowed Save row — the found count rides
/// back as the answer (the fact the assertion speaks, the m57 summary
/// shape).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_count_is_satisfied_on_entry() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let found = client
        .expect_count(any_button(), 4, Duration::from_secs(1))
        .await
        .expect("the whole set answers on entry");
    assert_eq!(found, 4, "the fixture's four buttons");

    let save = client
        .expect_count(named_button("Save"), 1, Duration::from_secs(1))
        .await
        .expect("the narrowed set answers on entry");
    assert_eq!(save, 1, "the Save row alone");

    handle.shutdown();
}

/// The M25 comparators ride the same loop: greater-than and less-than
/// answer the found count, and the wrong side of each comparison times
/// out naming the assertion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_comparators_ride_the_same_loop() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let more = client
        .expect_count_greater_than(any_button(), 3, Duration::from_secs(1))
        .await
        .expect("four is more than three");
    assert_eq!(more, 4);

    let fewer = client
        .expect_count_less_than(any_button(), 5, Duration::from_secs(1))
        .await
        .expect("four is fewer than five");
    assert_eq!(fewer, 4);

    let wrong_low = client
        .expect_count_greater_than(any_button(), 99, Duration::from_millis(300))
        .await
        .expect_err("four is not more than ninety-nine");
    assert!(
        matches!(wrong_low, GwpError::WaitTimeout { state, .. } if state == "have count"),
        "the timeout names the assertion: {wrong_low}"
    );

    let wrong_high = client
        .expect_count_less_than(any_button(), 2, Duration::from_millis(300))
        .await
        .expect_err("four is not fewer than two");
    assert!(
        matches!(wrong_high, GwpError::WaitTimeout { state, .. } if state == "have count"),
        "the timeout names the assertion: {wrong_high}"
    );

    handle.shutdown();
}

/// Absence is an answer (the count vocabulary's difference from the
/// content reads): an empty set is found 0 — satisfying `count 0` and
/// every less-than, never a permanent pending (Playwright's
/// `toHaveCount(0)` parity).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absence_is_an_answer() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let none = client
        .expect_count(named_button("Ghost"), 0, Duration::from_secs(1))
        .await
        .expect("an empty set satisfies count 0");
    assert_eq!(none, 0);

    let under = client
        .expect_count_less_than(named_button("Ghost"), 3, Duration::from_secs(1))
        .await
        .expect("an empty set is fewer than three");
    assert_eq!(under, 0);

    handle.shutdown();
}

/// The assertion is waitable: a fifth button joins a moment after
/// attach — the first evaluation is pending (found 4), the pushed
/// `TreeChanged` wakes the re-read, and the grown set satisfies.
/// Event-driven, no polling (§15.3 on both ends).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_count_waits_for_the_set_to_grow() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().add_button_update("Later"));
    });

    let found = client
        .expect_count(any_button(), 5, Duration::from_secs(2))
        .await
        .expect("the set grows with the tree update");
    assert_eq!(found, 5, "the joined button raised the count");

    handle.shutdown();
}

/// The timeout names names (§8.3): the assertion (`have count`), the
/// locator render, the budget and the last evaluation — the demanded
/// comparison and the count the last evaluation found — never a bare
/// "timed out".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_timeout_names_the_comparison_and_the_found_count() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let err = client
        .expect_count(any_button(), 99, Duration::from_millis(300))
        .await
        .expect_err("a mismatch must not satisfy");
    match err {
        GwpError::WaitTimeout {
            state,
            selector,
            timeout_ms,
            last,
        } => {
            assert_eq!(state, "have count");
            assert!(
                selector.contains("Button"),
                "the locator renders: {selector}"
            );
            assert_eq!(timeout_ms, 300);
            assert!(last.contains("count is 4"), "found side: {last}");
            assert!(last.contains("expected"), "expected side: {last}");
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// The assertion owns the event filter for its duration and drops it on
/// return — the m42/m54 subscription-window rule, row for row: a
/// satisfied assertion leaves the connection quiet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_count_leaves_no_subscription_behind() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    client
        .expect_count(any_button(), 4, Duration::from_secs(1))
        .await
        .expect("satisfied on entry");

    // Any events that interleaved the short window are drained first;
    // after that the wire is quiet — no wake-ups flow.
    let deadline = std::time::Instant::now() + Duration::from_millis(120);
    while tokio::time::timeout(Duration::from_millis(30), client.next_event())
        .await
        .is_ok()
    {
        assert!(std::time::Instant::now() < deadline, "drain took too long");
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(150), client.next_event())
            .await
            .is_err(),
        "a post-assert connection must be quiet — the assertion unsubscribed"
    );

    handle.shutdown();
}
