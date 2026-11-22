//! The §12 live-mode emitter end-to-end (m87): a real UDS session
//! server (the remote-mode door, §5.1), the CLI's `live_html` capture
//! over it, and the mid-session scrubber HTML stamped
//! `gwtrace/v1 (live)` — the M23 emitter fed by a running session, no
//! protocol change (the capture rides the M12 subscribe vocabulary).

use std::time::Duration;

use guiwright_cli::ConnectTarget;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{RequestV1, SelectorV1, SemActionV1, TextMatchV1};
use guiwright_proto::uds;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

/// The full row: attach → the session moves (a real click from a second
/// client, plus the server's subscribed idle ticks) → the mid-session
/// emission → the file carries the live stamp, the adapter header and
/// the captured tree.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_html_emits_mid_session_over_a_real_uds_door() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = socket_path("live-html-e2e.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");

    // The driver: a second client performing one real click — the tree
    // change the live feed captures (a `TreeChanged` re-synced dump).
    let driver = tokio::spawn({
        let path = path.clone();
        async move {
            let mut client = uds::Client::connect(&path).await.expect("driver connects");
            client
                .handshake("guiwright-live-e2e-driver/0.1")
                .await
                .expect("driver handshake");
            let resolved = client
                .request(RequestV1::Resolve {
                    selector: SelectorV1::Role {
                        role: "Button".into(),
                        name: Some(TextMatchV1 {
                            text: "Save".into(),
                            exact: false,
                            ignore_case: true,
                        }),
                    },
                    timeout_ms: 2_000,
                })
                .await
                .expect("driver resolve");
            let (node, generation) = match resolved {
                guiwright_proto::schema::ResponseV1::Resolved {
                    node, generation, ..
                } => (node, generation),
                other => panic!("expected Resolved, got {other:?}"),
            };
            client
                .request(RequestV1::Action {
                    node,
                    generation,
                    action: SemActionV1::Click,
                    force: false,
                    timeout_ms: 2_000,
                })
                .await
                .expect("driver click");
        }
    });

    // The capture: attach over the same door, hold the window while the
    // session runs (the click lands inside it), emit mid-session.
    let out = dir.path().join("guiwright-live.html");
    let written = guiwright_cli::live_html::live_html(
        &ConnectTarget::Uds(path.clone()),
        Some(out.clone()),
        600,
        Some("live-e2e".to_string()),
    )
    .await
    .expect("the capture emits");
    assert_eq!(written, out, "the explicit path is honored");
    driver.await.expect("driver finishes");
    handle.shutdown();

    let html = std::fs::read_to_string(&written).expect("html written");
    assert!(html.contains("gwtrace/v1 (live)"), "the live schema stamp");
    assert!(html.contains("live-e2e"), "the test label is in the header");
    assert!(html.contains("mock"), "the adapter name is in the header");
    // The captured tree: the fixture's buttons are in the scrubber
    // payload (the attach dump + the post-click re-sync).
    assert!(html.contains("Save"), "the captured tree rides the payload");
    assert!(html.contains("Cancel"), "the full dump is in the payload");
}

/// A closing door ends the capture early — the server shuts down
/// mid-window and the command still emits what the feed saw (the
/// session's end never eats the capture; a subscribed session's idle
/// ticks keep the full window busy, so the early end is the
/// door-closed path).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_html_ends_early_when_the_door_closes() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = socket_path("live-html-closed.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");

    // The session ends 300ms into the capture window.
    let closer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        handle.shutdown();
    });

    let started = std::time::Instant::now();
    let written = guiwright_cli::live_html::live_html(
        &ConnectTarget::Uds(path.clone()),
        Some(dir.path().join("closed.html")),
        10_000,
        None,
    )
    .await
    .expect("the capture emits despite the close");
    let elapsed = started.elapsed();
    closer.await.expect("closer finishes");

    assert!(
        elapsed < Duration::from_secs(9),
        "the closing door ended the capture early: {elapsed:?}"
    );
    let html = std::fs::read_to_string(&written).expect("html written");
    assert!(html.contains("gwtrace/v1 (live)"), "the live schema stamp");
    assert!(html.contains("mock"), "the adapter header");
}
