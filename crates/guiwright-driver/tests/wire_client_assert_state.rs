//! The seam's waitable state assertions (m73 — the kit's
//! `to_be_checked` / `to_be_enabled` / `to_be_focused` vocabulary over
//! any door): [`ClientConn::expect_checked`] / [`ClientConn::expect_enabled`]
//! / [`ClientConn::expect_focused`] compose the m54 wake loop with the
//! m70 set-width state read (`describe_all_identity_state` re-reads) —
//! a remote suite waits the state three exactly like the kit's
//! assertions, no wire change (the assertion rides existing domains
//! byte-for-byte). Driven here over the in-process door with the
//! checkbox-trio fixture and the shared-script channel standing in for
//! app-side UI changes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{NodeId, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{CheckedStateV1, SelectorV1, TextMatchV1};
use guiwright_proto::{DuplexClient, GwpError};

fn named(name: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: "CheckBox".into(),
        name: Some(TextMatchV1 {
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
        .handshake("wire-assert-state-test/0.1")
        .await
        .expect("handshake");
    client
}

/// The checkbox-trio fixture behind a shared script channel — updates
/// pushed from another thread land on the session's next pump, exactly
/// like app-side UI changes riding a real adapter (the m42 scripted
/// harness, restated at state width).
fn spawn_scripted() -> (GwpServerHandle, DuplexConnect, Arc<Mutex<Vec<TreeUpdate>>>) {
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_app = Arc::clone(&scripts);
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        move || {
            MockAdapter::with_fixture(&Fixture::with_checkboxes())
                .with_shared_scripts(scripts_for_app)
        },
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");
    (handle, connect, scripts)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_checked_is_satisfied_on_entry() {
    let (handle, connect, _scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    let row = client
        .expect_checked(
            named("Notifications"),
            CheckedStateV1::Checked,
            Duration::from_secs(1),
        )
        .await
        .expect("Notifications is checked from the start");
    assert_eq!(row.checked, CheckedStateV1::Checked);
    assert_eq!(row.name.as_deref(), Some("Notifications"));

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_checked_waits_for_the_flip() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    // The app checks "Dark mode" a moment after attach: the assertion
    // subscribes, evaluates (Unchecked — pending), and wakes on the
    // pushed TreeChanged — event-driven, no polling (the m54 loop's
    // edges at state width).
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        if let Some(update) =
            Fixture::with_checkboxes().checkbox_toggle_update(NodeId(10), accesskit::Toggled::True)
        {
            scripts_for_push.lock().expect("script lock").push(update);
        }
    });

    let row = client
        .expect_checked(
            named("Dark mode"),
            CheckedStateV1::Checked,
            Duration::from_secs(2),
        )
        .await
        .expect("Dark mode flips checked");
    assert_eq!(row.checked, CheckedStateV1::Checked);

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_word_times_out_naming_expected_and_found() {
    let (handle, connect, _scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    let err = client
        .expect_checked(
            named("Dark mode"),
            CheckedStateV1::Checked,
            Duration::from_millis(300),
        )
        .await
        .expect_err("Dark mode stays unchecked");
    let GwpError::WaitTimeout {
        state,
        selector,
        last,
        ..
    } = &err
    else {
        panic!("the typed timeout: {err:?}");
    };
    assert_eq!(*state, "be checked");
    assert!(selector.contains("Dark mode"), "{selector}");
    assert!(last.contains("Unchecked"), "{last}");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_enabled_answers_the_boolean_fact() {
    let (handle, connect, _scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    client
        .expect_enabled(named("Notifications"), true, Duration::from_secs(1))
        .await
        .expect("a live checkbox is enabled");
    let err = client
        .expect_enabled(named("Dark mode"), false, Duration::from_millis(300))
        .await
        .expect_err("Dark mode never disables");
    let GwpError::WaitTimeout { state, last, .. } = &err else {
        panic!("the typed timeout: {err:?}");
    };
    assert_eq!(*state, "be enabled");
    assert!(last.contains("enabled is true"), "{last}");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_focused_waits_for_the_focus_to_arrive() {
    let (handle, connect, scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    // The app focuses "Notifications" a moment after attach: the focus
    // rides the scripted update's `focus` word (the same field a real
    // toolkit's focus change carries), the wake fires on the pushed
    // TreeChanged.
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(TreeUpdate {
                nodes: vec![],
                tree: None,
                tree_id: accesskit::TreeId::ROOT,
                focus: NodeId(9),
            });
    });

    let row = client
        .expect_focused(named("Notifications"), Duration::from_secs(2))
        .await
        .expect("the focus arrives");
    assert!(row.focused, "the satisfying row carries the fact");

    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfocused_node_times_out_naming_the_fact() {
    let (handle, connect, _scripts) = spawn_scripted();
    let mut client = connected(&connect).await;

    // The fixture seeds the focus at the window root: no walk row is
    // focused, so the assertion stays pending — a spent budget names
    // `be focused` and the fact the last evaluation found.
    let err = client
        .expect_focused(named("Notifications"), Duration::from_millis(300))
        .await
        .expect_err("the focus never arrives");
    let GwpError::WaitTimeout { state, last, .. } = &err else {
        panic!("the typed timeout: {err:?}");
    };
    assert_eq!(*state, "be focused");
    assert!(last.contains("focused is false"), "{last}");

    handle.shutdown();
}
