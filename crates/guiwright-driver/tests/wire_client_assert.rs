//! The client seam's content-assertion rows over the wire (m54): the kit's
//! `expect(locator).to_have_text(...)` / `to_have_value(...)` half of the
//! §8 check vocabulary, composed client-side as [`ClientConn::expect_text`]
//! / [`ClientConn::expect_value`] — the m46 bulk reads' source orders
//! (label first vs value first) as waitable assertions over the m39
//! describe-all round trip, woken by the §7 event channel the way the m42
//! wait wakes. **No wire change**: the assertions ride existing domains
//! byte-for-byte; every [`ClientConn`] impl inherits the rows. Driven here
//! over the in-process door with the four-button fixture and the
//! shared-script channel standing in for app-side UI changes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{DescribedV1, NodeKeyV1, SelectorV1, TextMatchV1};
use guiwright_proto::{GwpError, expect::ExpectKind};

fn named_role(role: &str, name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

fn named_button(name: &str) -> SelectorV1 {
    named_role("Button", name)
}

/// The fixture's Email input (`getByRole(TextInput, "Email")` — its name
/// "Email" and its value "ada@example.dev" are the m29/m31 order pins).
fn email_input() -> SelectorV1 {
    named_role("TextInput", "Email")
}

fn fuzzy(pattern: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: pattern.into(),
        exact: false,
        ignore_case: true,
    }
}

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-assert-test/0.1")
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

/// The Email input's value restated (node 5): the same node identity, a
/// new value — the update shape a real toolkit emits when a form field
/// changes while the test watches.
fn email_value_update(value: &str) -> TreeUpdate {
    let mut input = Node::new(Role::TextInput);
    input.set_label("Email");
    input.set_value(value);
    input.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 128.0,
        x1: 256.0,
        y1: 156.0,
    });
    TreeUpdate {
        nodes: vec![(NodeId(5), input)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    }
}

fn node_id_of(row: &DescribedV1) -> u64 {
    match &row.node {
        NodeKeyV1::AccessKit { node, .. } => *node,
        other @ NodeKeyV1::Logical { .. } => {
            panic!("expected AccessKit keys, got {other:?}")
        }
    }
}

/// `expect_text` satisfied on entry: the Save button's accessible name
/// answers, and the assertion hands back the row itself — a directly
/// usable `RequestV1::Action` handle (the dump-row rule, M11).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_text_is_satisfied_on_entry() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let row = client
        .expect_text(
            named_button("Save"),
            TextMatchV1 {
                text: "sav".into(),
                exact: false,
                ignore_case: true,
            },
            Duration::from_secs(1),
        )
        .await
        .expect("the label satisfies the text read from the start");
    assert_eq!(
        node_id_of(&row),
        2,
        "the Save row, usable as an Action handle"
    );
    assert_eq!(row.name.as_deref(), Some("Save"));

    handle.shutdown();
}

/// The m29/m31 flip pinned over the door: the Email input's name answers
/// `expect_text` and never `expect_value`, its value answers
/// `expect_value` and never `expect_text` — the two reads can never
/// silently agree on the wrong datum (the m46 derivation rule, now
/// waitable).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_source_orders_flip_between_the_two_reads() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let text_row = client
        .expect_text(email_input(), fuzzy("Email"), Duration::from_secs(1))
        .await
        .expect("the label-first read answers the label");
    assert_eq!(text_row.name.as_deref(), Some("Email"));

    let value_row = client
        .expect_value(
            email_input(),
            fuzzy("ada@example.dev"),
            Duration::from_secs(1),
        )
        .await
        .expect("the value-first read answers the value");
    assert_eq!(value_row.value.as_deref(), Some("ada@example.dev"));

    let wrong = client
        .expect_text(
            email_input(),
            fuzzy("ada@example.dev"),
            Duration::from_millis(300),
        )
        .await
        .expect_err("a value must not satisfy the text read");
    assert!(
        matches!(wrong, GwpError::WaitTimeout { state, .. } if state == "have text"),
        "the timeout names the assertion: {wrong}"
    );

    let wrong_value = client
        .expect_value(email_input(), fuzzy("Email"), Duration::from_millis(300))
        .await
        .expect_err("a label must not satisfy the value read");
    assert!(
        matches!(wrong_value, GwpError::WaitTimeout { state, .. } if state == "have value"),
        "the timeout names the assertion: {wrong_value}"
    );

    handle.shutdown();
}

/// The assertion is waitable: the input's value changes a moment after
/// attach — the first evaluation is pending (the m31 order still reads
/// the old value), the pushed `TreeChanged` wakes the re-read, and the
/// new value satisfies. Event-driven, no polling (§15.3 on both ends).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_value_waits_for_the_value_to_arrive() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(email_value_update("ada@corp.example"));
    });

    let row = client
        .expect_value(
            email_input(),
            fuzzy("ada@corp.example"),
            Duration::from_secs(2),
        )
        .await
        .expect("the value arrives with the tree update");
    assert_eq!(row.value.as_deref(), Some("ada@corp.example"));

    handle.shutdown();
}

/// The timeout names names (§8.3): the locator render, the budget and the
/// last evaluation — expected and found — never a bare "timed out".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_timeout_names_the_locator_expected_and_found() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let err = client
        .expect_text(
            named_button("Save"),
            TextMatchV1 {
                text: "Done".into(),
                exact: true,
                ignore_case: false,
            },
            Duration::from_millis(300),
        )
        .await
        .expect_err("a mismatch must not satisfy");
    match err {
        GwpError::WaitTimeout {
            state,
            selector,
            timeout_ms,
            last,
        } => {
            assert_eq!(state, "have text");
            assert!(selector.contains("Save"), "the locator renders: {selector}");
            assert_eq!(timeout_ms, 300);
            assert!(last.contains("Save"), "found content: {last}");
            assert!(last.contains("expected"), "expected side: {last}");
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    let empty = client
        .expect_text(
            named_button("Ghost"),
            fuzzy("Ghost"),
            Duration::from_millis(300),
        )
        .await
        .expect_err("an empty set cannot satisfy");
    match empty {
        GwpError::WaitTimeout { last, .. } => {
            assert!(last.contains("no match"), "the empty-set reason: {last}");
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// The assertion owns the event filter for its duration and drops it on
/// return — the m42 subscription-window rule, row for row: a satisfied
/// assertion leaves the connection quiet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_text_leaves_no_subscription_behind() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    client
        .expect_text(named_button("Save"), fuzzy("Save"), Duration::from_secs(1))
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

/// The vocabulary is client-side only: [`ExpectKind`] names the two source
/// orders without ever touching the wire (the m42 `WaitState` rule) — the
/// state word each timeout names is pinned here so the REPL/TS mirrors
/// can quote it.
#[test]
fn the_kind_state_words_are_stable() {
    assert_eq!(
        ExpectKind::Text(TextMatchV1 {
            text: "x".into(),
            exact: false,
            ignore_case: true,
        })
        .state(),
        "have text"
    );
    assert_eq!(
        ExpectKind::Value(TextMatchV1 {
            text: "x".into(),
            exact: false,
            ignore_case: true,
        })
        .state(),
        "have value"
    );
}
