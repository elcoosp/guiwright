//! The client seam's description-assertion rows over the wire (m64): the
//! kit's `expect(locator).to_have_accessible_description(...)` vocabulary
//! (M38) over any door — the m62 identity rows' third fact, riding the
//! appended `DescribeAllIdentity` / `DescribedAllIdentity` domain (the
//! M39 walk's rows carrying the description fact the M39 domain left
//! out, the m62/m63 records' reserved wire change). Driven here over the
//! in-process door with the described fixture and the shared-script
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
use guiwright_proto::schema::{SelectorV1, TextMatchV1};

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

/// A fuzzy button locator — the set the wake conversation below rides
/// (the head row stays the Save button across the scripted restatement,
/// the fuzzy matcher never dropping it from the set).
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

fn fuzzy(pattern: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: pattern.into(),
        exact: false,
        ignore_case: true,
    }
}

fn exact(pattern: &str) -> TextMatchV1 {
    TextMatchV1 {
        text: pattern.into(),
        exact: true,
        ignore_case: false,
    }
}

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-description-test/0.1")
        .await
        .expect("handshake");
    client
}

/// The described fixture — the buttons base plus the Install button
/// whose row carries `"Installs the latest build"` (the m38 extension
/// band, the identity fact the M39 rows left out).
fn spawn_described() -> (GwpServerHandle, DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::with_description()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect)
}

/// The described fixture behind a shared script channel — updates pushed
/// from another thread land on the session's next pump, exactly like
/// app-side UI changes riding a real adapter.
fn spawn_scripted() -> (GwpServerHandle, DuplexConnect, Arc<Mutex<Vec<TreeUpdate>>>) {
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_app = Arc::clone(&scripts);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || {
            MockAdapter::with_fixture(&Fixture::with_description())
                .with_shared_scripts(scripts_for_app)
        },
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, scripts)
}

/// Satisfied on entry: the head row's description answers the matcher —
/// the m38 description-only read (the name and the value never answer a
/// description assertion), and the satisfying row hands back the full
/// identity row (the m64 row is a dump-row handle with the fact beside
/// it, the dump-row rule).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_description_is_satisfied_on_entry() {
    let (handle, connect) = spawn_described();
    let mut client = connected(&connect).await;

    let row = client
        .expect_description(
            named_button("Install"),
            fuzzy("Installs"),
            Duration::from_secs(1),
        )
        .await
        .expect("the head row's description answers on entry");
    assert_eq!(
        row.description.as_deref(),
        Some("Installs the latest build"),
        "the fact the assertion speaks"
    );
    assert_eq!(row.name.as_deref(), Some("Install"), "the set's head row");
    assert_eq!(row.role, "Button");

    // The exact posture rides the same read (the REPL's quoted form).
    let exact_row = client
        .expect_description(
            named_button("Install"),
            exact("Installs the latest build"),
            Duration::from_secs(1),
        )
        .await
        .expect("the exact posture answers the same row");
    assert_eq!(exact_row.node, row.node, "the same head row answers");

    handle.shutdown();
}

/// The timeout names names (§8.3): the state word (`have description`),
/// the locator render, the budget and the last evaluation — the fact the
/// row carries beside the wanted description — never a bare "timed out".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_description_row_names_the_mismatch() {
    let (handle, connect) = spawn_described();
    let mut client = connected(&connect).await;

    let err = client
        .expect_description(
            named_button("Install"),
            exact("Ghost"),
            Duration::from_millis(300),
        )
        .await
        .expect_err("the head row describes the install, not Ghost");
    match err {
        GwpError::WaitTimeout {
            state,
            selector,
            timeout_ms,
            last,
        } => {
            assert_eq!(state, "have description");
            assert!(
                selector.contains("Install"),
                "the locator renders: {selector}"
            );
            assert_eq!(timeout_ms, 300);
            assert!(
                last.contains(r#"description is "Installs the latest build""#),
                "found side: {last}"
            );
            assert!(last.contains(r#"expected "Ghost""#), "wanted side: {last}");
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// A node with no accessible description stays pending (waitable), the
/// reason naming which fact is missing — the m38 pending rule (the
/// descriptions arrive with the node's restatement) at the single-row
/// width. The Save button carries a label and a value-adjacent name but
/// no description, and the name never answers the description read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_description_stays_pending() {
    let (handle, connect) = spawn_described();
    let mut client = connected(&connect).await;

    let err = client
        .expect_description(
            named_button("Save"),
            fuzzy("Anything"),
            Duration::from_millis(300),
        )
        .await
        .expect_err("the undescribed button cannot satisfy a description assertion");
    match err {
        GwpError::WaitTimeout { state, last, .. } => {
            assert_eq!(state, "have description");
            assert!(
                last.contains("no accessible description"),
                "the missing fact is named: {last}"
            );
        }
        other => panic!("expected the typed wait timeout, got {other:?}"),
    }

    handle.shutdown();
}

/// The assertion is waitable: the Save button gains its description a
/// moment after attach — the first evaluation is pending (the missing
/// fact named), the pushed `TreeChanged` wakes the re-read, and the
/// restated row satisfies. Event-driven, no polling (§15.3 on both
/// ends). The head row stays the Save button across the restatement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_description_waits_for_the_restatement() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::with_description().describe_save_update());
    });

    let row = client
        .expect_description(
            fuzzy_button("Sav"),
            fuzzy("Saves every change"),
            Duration::from_secs(2),
        )
        .await
        .expect("the restatement wakes the re-read");
    assert_eq!(
        row.description.as_deref(),
        Some("Saves every change"),
        "the restated fact answers"
    );
    assert_eq!(row.name.as_deref(), Some("Save"), "the head row stays");

    handle.shutdown();
}
