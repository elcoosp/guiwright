//! The client seam's bulk-content reads over the wire (m46 — the m41
//! record's Rust-client half, closed): [`ClientConn::all_text_contents`]
//! and [`ClientConn::all_values`] derive the kit's m29/m31 source orders
//! over the M39 describe-all round trip — one read per set, **no wire
//! change** (the rows ride `DescribeAll` byte-for-byte as M39 blessed
//! them, the m41 fail-fast rule: a driver without the domain surfaces
//! the typed rejection rather than degrading). Driven here over the
//! in-process door with the four-button fixture — the same reality the
//! kit reads in-process and the TypeScript client derives over TCP.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{SelectorV1, TextMatchV1};
use guiwright_proto::{DuplexClient, GwpError};

/// A Role-selector, exact-named or bare (the bare form matches all four
/// fixture buttons).
fn role(role: &str, name: Option<&str>) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: name.map(|name| TextMatchV1 {
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
        .handshake("wire-texts-values/0.1")
        .await
        .expect("handshake");
    client
}

/// The set's text, label first (the m29 order the text locators match):
/// document order, every match carried — including the fixture's hidden
/// "Invisible" button (the non-strict walk) — and the Email input
/// answering its *label*, never its value (the m29 order's whole point:
/// these are the rows `get_by_text` matches).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_text_contents_carries_the_m29_label_first_order() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let texts = client
        .all_text_contents(role("Button", None))
        .await
        .expect("the button set's text");
    assert_eq!(
        texts,
        vec!["Save", "Cancel", "Invisible", "Confirm"],
        "document order, hidden included, label first"
    );

    let texts = client
        .all_text_contents(role("TextInput", None))
        .await
        .expect("the input's text");
    assert_eq!(
        texts,
        vec!["Email"],
        "label first — the value never satisfies the m29 read"
    );

    handle.shutdown();
}

/// The same walk with the source order flipped (the m31 form-dump read):
/// the Email input answers its *value*, and the value-less buttons fall
/// back to their labels — the two reads can never silently agree on the
/// wrong datum (the m31 rule the kit and TS clients carry).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_values_flips_to_the_m31_value_first_order() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let values = client
        .all_values(role("TextInput", None))
        .await
        .expect("the input's value");
    assert_eq!(
        values,
        vec!["ada@example.dev"],
        "value first — the label is the fallback"
    );

    let values = client
        .all_values(role("Button", None))
        .await
        .expect("the button set's values");
    assert_eq!(
        values,
        vec!["Save", "Cancel", "Invisible", "Confirm"],
        "value-less rows fall back to the label, positions 1:1"
    );

    handle.shutdown();
}

/// Snapshot semantics, unchanged from the `count()`/`all()` family: no
/// strict mode, no waiting — an empty set answers an empty list, never an
/// error (the m30 rule over the m39 domain).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_set_answers_an_empty_list() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let texts = client
        .all_text_contents(role("CheckBox", None))
        .await
        .expect("the empty set");
    assert!(texts.is_empty(), "no matches — no rows: {texts:?}");
    let values = client
        .all_values(role("CheckBox", None))
        .await
        .expect("the empty set");
    assert!(values.is_empty(), "no matches — no rows: {values:?}");

    handle.shutdown();
}

/// The m41 fail-fast rule on the Rust seam: a driver that predates M39
/// answers `DescribeAll` with the typed `Unsupported`, and the read
/// surfaces that rejection rather than degrading (the REPL's per-row
/// fallback is the interactive surface's concession, never this one's —
/// the wait module's boundary, restated for the content reads).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_driver_without_describe_all_is_a_typed_rejection() {
    use guiwright_proto::Handler;
    use guiwright_proto::duplex;
    use guiwright_proto::schema::{ErrorCode, PROTO_VERSION};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The seam-test echo handler: it serves the conversation vocabulary
    /// but answers every domain read with `Unsupported` — a pre-M39
    /// driver's exact shape for the content reads.
    struct PreM39Handler {
        handshakes: Arc<AtomicUsize>,
    }

    impl Handler for PreM39Handler {
        fn handle(
            &mut self,
            request: guiwright_proto::schema::RequestV1,
        ) -> impl std::future::Future<Output = guiwright_proto::schema::ResponseV1> + Send {
            use guiwright_proto::schema::{AdapterInfoV1, CapsV1, RequestV1, ResponseV1};
            if matches!(request, RequestV1::Hello { .. }) {
                self.handshakes.fetch_add(1, Ordering::SeqCst);
            }
            let response = match request {
                RequestV1::Hello { .. } => ResponseV1::Welcome {
                    proto_version: PROTO_VERSION,
                    session_id: "pre-m39".into(),
                    tree_seq: 1,
                    frame_seq: 0,
                    adapter: AdapterInfoV1 {
                        name: "mock".into(),
                        framework: "fixture".into(),
                        framework_version: "0".into(),
                        caps: CapsV1::default(),
                    },
                },
                RequestV1::Ping => ResponseV1::Pong,
                _ => ResponseV1::Error {
                    code: ErrorCode::Unsupported,
                    message: "not implemented in the pre-M39 handler".into(),
                },
            };
            async move { response }
        }
    }

    let (listener, connect) = duplex::channel();
    let handshakes = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&handshakes);
    tokio::spawn(async move {
        duplex::serve(listener, move || PreM39Handler {
            handshakes: Arc::clone(&h2),
        })
        .await;
    });

    let mut client = connected(&connect).await;
    let err = client
        .all_text_contents(role("Button", None))
        .await
        .expect_err("the read fails fast");
    match err {
        GwpError::Rejected { code, .. } => {
            assert_eq!(code, ErrorCode::Unsupported, "the typed rejection: {err}");
        }
        other => panic!("expected the typed rejection, got {other:?}"),
    }
    assert_eq!(
        handshakes.load(Ordering::SeqCst),
        1,
        "one handshake, one door"
    );
}
