//! The client seam's identity-assertion rows over the wire (m62): the
//! kit's `expect(locator).to_have_role(...)` /
//! `to_have_accessible_name(...)` vocabulary (M38) over any door — the
//! m54 content assertions' wake-loop composition evaluating the identity
//! facts the M39 describe-all rows already carry. **No wire change**: the
//! rows ride existing domains byte-for-byte; every [`ClientConn`] impl
//! inherits them. Driven here over the in-process door with the
//! four-button fixture and the shared-script channel standing in for
//! app-side UI changes.

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
use guiwright_proto::schema::{SelectorV1, TextMatchV1};

/// Every `Button` the fixture carries — the set whose head row the
/// identity assertions pin (the m54 set-head rule: document order, the
/// Save button first).
fn any_button() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: None,
    }
}

/// One named button — a narrowed set whose head is that row alone.
fn named_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
    }
}

/// A fuzzy button locator — the set the rename conversation below rides
/// (the head row stays the Save button across the rename, the fuzzy
/// matcher never dropping it from the set).
fn fuzzy_button(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: name.into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-identity-test/0.1")
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

/// Satisfied on entry: the head row's role answers the exact string
/// (the AccessKit debug name the describe rows print), the label-only
/// name answers the matcher — the m38 vocabulary's narrower read
/// (never the m29 label-or-value fallback).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_role_and_name_are_satisfied_on_entry() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let row = client
        .expect_role(any_button(), "Button".into(), Duration::from_secs(1))
        .await
        .expect("the head row's role answers on entry");
    assert_eq!(row.role, "Button", "the fact the assertion speaks");
    assert_eq!(row.name.as_deref(), Some("Save"), "the set's head row");

    let named = client
        .expect_name(
            named_button("Save"),
            TextMatchV1 {
                text: "Save".into(),
                exact: false,
                ignore_case: true,
            },
            Duration::from_secs(1),
        )
        .await
        .expect("the label-only name answers on entry");
    assert_eq!(named.name.as_deref(), Some("Save"));

    handle.shutdown();
}

/// The timeout names names (§8.3): the state word (`have role` / `have
/// name`), the locator render, the budget and the last evaluation — the
/// fact the row carries beside the wanted identity — never a bare
/// "timed out".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_identity_rows_name_the_mismatch() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let wrong_role = client
        .expect_role(any_button(), "TextInput".into(), Duration::from_millis(300))
        .await
        .expect_err("the head row is a Button, not a TextInput");
    match wrong_role {
        GwpError::WaitTimeout {
            state,
            selector,
            timeout_ms,
            last,
        } => {
            assert_eq!(state, "have role");
            assert!(
                selector.contains("Button"),
                "the locator renders: {selector}"
            );
            assert_eq!(timeout_ms, 300);
            assert!(last.contains(r#"role is "Button""#), "found side: {last}");
            assert!(
                last.contains(r#"expected "TextInput""#),
                "wanted side: {last}"
            );
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    let wrong_name = client
        .expect_name(
            any_button(),
            TextMatchV1 {
                text: "Ghost".into(),
                exact: false,
                ignore_case: true,
            },
            Duration::from_millis(300),
        )
        .await
        .expect_err("the head row is named Save, not Ghost");
    match wrong_name {
        GwpError::WaitTimeout { state, last, .. } => {
            assert_eq!(state, "have name");
            assert!(last.contains(r#"name is "Save""#), "found side: {last}");
            assert!(last.contains(r#"expected "Ghost""#), "wanted side: {last}");
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// The name row reads the label only — a node with no accessible name
/// stays pending (waitable), and the pending reason says which fact is
/// missing (the m38 narrower read, the m54 empty-content rule's shape).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_name_stays_pending() {
    let (handle, connect) = spawn_fixture();
    let mut client = connected(&connect).await;

    let err = client
        .expect_name(
            SelectorV1::Role {
                role: "Group".into(),
                name: None,
            },
            TextMatchV1 {
                text: "Editor".into(),
                exact: false,
                ignore_case: true,
            },
            Duration::from_millis(300),
        )
        .await
        .expect_err("the unnamed group cannot satisfy a name assertion");
    match err {
        GwpError::WaitTimeout { state, last, .. } => {
            assert_eq!(state, "have name");
            assert!(
                last.contains("no accessible name"),
                "the missing fact is named: {last}"
            );
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// The assertion is waitable: the Save button renames a moment after
/// attach — the first evaluation is pending (`name is "Save"`), the
/// pushed `TreeChanged` wakes the re-read, and the renamed row
/// satisfies. Event-driven, no polling (§15.3 on both ends).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_name_waits_for_the_rename() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().rename_save_update());
    });

    let row = client
        .expect_name(
            fuzzy_button("Sav"),
            TextMatchV1 {
                text: "Save All".into(),
                exact: false,
                ignore_case: true,
            },
            Duration::from_secs(2),
        )
        .await
        .expect("the rename wakes the re-read");
    assert_eq!(row.name.as_deref(), Some("Save All"));

    handle.shutdown();
}
