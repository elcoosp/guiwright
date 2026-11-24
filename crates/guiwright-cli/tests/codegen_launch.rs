//! The §13 standalone codegen launcher end-to-end (the m4/m5/m6/m7
//! deferred row): `guiwright codegen <app-cmd…>` launches the app binary
//! with the §5.3 `GUIWRIGHT_DRIVER` door env, waits for the door, runs
//! the overlay session (M8 `EnableOverlay` + the recorder), and hands
//! back the generated kit code when the window ends. The plan line this
//! pins (docs/m5-test-runner.md §4): "`guiwright codegen <app-cmd>` —
//! the app-launching wrapper lands with the overlay session (§13
//! highlight handles)."
//!
//! The "app" is this test binary re-invoked in child mode: the parent
//! passes `--exact guiwright_codegen_demo_app_child`, and the launcher
//! injects `GUIWRIGHT_DRIVER` into the child's env — the app-side
//! contract every remote-mode app implements (§5.3, the gpui
//! `contract.rs` doc example). A normal test sweep runs the guard with
//! the env unset, so the child body no-ops.

use std::time::Duration;

use guiwright_cli::ConnectTarget;
use guiwright_cli::args::{Command, parse_args};
use guiwright_cli::codegen::{CodegenError, CodegenLaunch, codegen_launch};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;
use guiwright_proto::schema::{RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1};
use guiwright_proto::uds;

/// The door path for a test (fresh per call; `spawn_session_server`
/// unlinks stale sockets, but a unique path keeps runs independent).
fn socket_path(tag: &str) -> std::path::PathBuf {
    // Leak the tempdir: the child serves out of it for the whole test
    // process (the live_html e2e's `Box::leak` pattern — the OS reaps
    // at exit).
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(format!("{tag}.sock"))
}

/// The child half of the e2e: serve the buttons fixture over the
/// `GUIWRIGHT_DRIVER` door (the §5.3 launch-mode contract) until the
/// launcher kills the process — or, when the test touches the door's
/// `<path>.exit` sentinel, serve a short window and exit (the
/// door-closed path). The sentinel rides the launcher-carried door path,
/// so no extra child-side channel is needed.
#[test]
fn guiwright_codegen_demo_app_child() {
    let Ok(door) = std::env::var("GUIWRIGHT_DRIVER") else {
        return; // normal test sweep: the launcher env is absent, no-op.
    };
    let invoked_as_child = std::env::args().any(|arg| arg == "guiwright_codegen_demo_app_child");
    if !invoked_as_child {
        return;
    }
    let path = door
        .strip_prefix("uds://")
        .expect("the demo child serves a uds door")
        .to_owned();
    let sentinel = std::path::PathBuf::from(&path).with_extension("exit");
    // `spawn_session_server` bridges over a tokio accept task — the
    // child hosts its own current-thread runtime (the CLI front-ends'
    // runtime shape).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("child runtime");
    rt.block_on(async move {
        let _handle = spawn_session_server(std::path::PathBuf::from(&path), || {
            MockAdapter::with_fixture(&Fixture::buttons())
        })
        .expect("the demo child binds its door");
        // Serve until the sentinel (the door-closed fixture) or until
        // the launcher's kill (the capture fixture).
        while !sentinel.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
}

/// The full row (m4 §4 → m7 §4 "unchanged"): launch → door wait → the
/// §13 overlay session (`EnableOverlay` + `StartRecording`) → a real click
/// driven inside the window by a second client → `--duration-ms` ends
/// the window while the door still lives → the recorder's exact kit
/// program comes back, the child is reaped, the out file carries the
/// code.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codegen_launch_captures_the_recorded_session_code() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let out = dir.path().join("generated.rs");
    let door_path = socket_path("codegen-e2e");
    let exe = std::env::current_exe().expect("test binary");

    // The "user": one real click inside the capture window (a second
    // client, the live_html e2e's driver shape).
    let driver = tokio::spawn({
        let path = door_path.clone();
        async move {
            // Give the launcher its attach window head start.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut client = uds::Client::connect(&path).await.expect("driver connects");
            client
                .handshake("guiwright-codegen-e2e-driver/0.1")
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
                ResponseV1::Resolved {
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

    let outcome = codegen_launch(&CodegenLaunch {
        command: vec![
            exe.display().to_string(),
            "--exact".into(),
            "guiwright_codegen_demo_app_child".into(),
            "--nocapture".into(),
        ],
        door: Some(ConnectTarget::Uds(door_path.clone())),
        attach_ms: Some(10_000),
        duration_ms: Some(1_500),
        out: Some(out.clone()),
    })
    .await
    .expect("the launch captures the recording");

    driver.await.expect("driver task joins");
    // The recorder's exact kit program for one click (the m87 wire
    // codegen shape, `guiwright-driver/tests/codegen_wire.rs`).
    assert_eq!(
        outcome.code,
        "app.get_by_role(Role::Button, \"Save\").click();\n"
    );
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    // The `--out` file carries the same code.
    let written = std::fs::read_to_string(&out).expect("out file written");
    assert_eq!(written, outcome.code);
}

/// The door-closed early end: the app closes its door before the window
/// ends, so the server-side single-slot recorder died with the session —
/// the typed `RecordingLost` answer names the loss instead of pretending
/// to capture (the honest counterpart of the m87 live-html early end).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codegen_launch_reports_the_lost_recording_when_the_door_closes() {
    let exe = std::env::current_exe().expect("test binary");
    let door_path = socket_path("codegen-lost");
    // The app exits 300ms into the window (the sentinel-file handshake
    // the child fixture serves until).
    let sentinel = door_path.with_extension("exit");
    let closer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(&sentinel, b"exit").expect("sentinel written");
    });
    let err = codegen_launch(&CodegenLaunch {
        command: vec![
            exe.display().to_string(),
            "--exact".into(),
            "guiwright_codegen_demo_app_child".into(),
            "--nocapture".into(),
        ],
        door: Some(ConnectTarget::Uds(door_path)),
        attach_ms: Some(10_000),
        // The window is longer than the app's life, so the door closes
        // first.
        duration_ms: Some(6_000),
        out: None,
    })
    .await
    .expect_err("the lost recording is a typed error, not a capture");
    closer.await.expect("closer task joins");
    assert!(matches!(err, CodegenError::RecordingLost), "{err:?}");
    assert!(
        err.to_string().contains("recording"),
        "the message names the recording: {err}"
    );
}

/// A command that cannot spawn answers the typed `Spawn` error naming
/// the program — the wrapper never falls back to attaching to some
/// other session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codegen_launch_reports_spawn_failures() {
    let err = codegen_launch(&CodegenLaunch {
        command: vec!["definitely-not-a-real-binary-guiwright-e2e".into()],
        door: Some(socket_path_uds("codegen-spawn")),
        attach_ms: Some(1_000),
        duration_ms: Some(1_000),
        out: None,
    })
    .await
    .expect_err("a bad command is a typed spawn error");
    assert!(matches!(err, CodegenError::Spawn { .. }), "{err:?}");
}

/// `uds://`-shaped `ConnectTarget` for the tests.
fn socket_path_uds(tag: &str) -> ConnectTarget {
    ConnectTarget::Uds(socket_path(tag))
}

/// The argv surface: `guiwright codegen [--flags…] <app-cmd…>` — launcher
/// flags precede the app command; everything after the program belongs
/// to the app (the `env`-wrapper rule). The plan names the shape
/// (`guiwright codegen <app-cmd>`); the parser pins it.
#[test]
fn codegen_args_parse_flags_then_the_app_command() {
    let cmd = parse_args(&args(&[
        "codegen",
        "--uds",
        "/tmp/app.sock",
        "--duration-ms",
        "5000",
        "--out",
        "gen.rs",
        "./target/debug/demo-app",
        "--serve",
        "--port",
        "5",
    ]))
    .unwrap();
    let Command::Codegen {
        door,
        duration_ms,
        out,
        command,
        ..
    } = cmd
    else {
        panic!("expected Command::Codegen, got {cmd:?}");
    };
    assert_eq!(
        door,
        Some(ConnectTarget::Uds(std::path::PathBuf::from(
            "/tmp/app.sock"
        )))
    );
    assert_eq!(duration_ms, Some(5_000));
    assert_eq!(out, Some(std::path::PathBuf::from("gen.rs")));
    assert_eq!(
        command,
        vec!["./target/debug/demo-app", "--serve", "--port", "5",]
    );

    // Defaults: no door (fresh temp socket), no window (until the door
    // closes), no out file; the app command is required.
    let cmd = parse_args(&args(&["codegen", "demo-app"])).unwrap();
    let Command::Codegen {
        door,
        duration_ms,
        out,
        command,
        attach_ms,
    } = cmd
    else {
        panic!("expected Command::Codegen, got {cmd:?}");
    };
    assert_eq!(door, None);
    assert_eq!(duration_ms, None);
    assert_eq!(attach_ms, None);
    assert_eq!(out, None);
    assert_eq!(command, vec!["demo-app"]);
    assert!(parse_args(&args(&["codegen"])).is_err());
    assert!(parse_args(&args(&["codegen", "--duration-ms", "x", "app"])).is_err());
    // The runner trace flag parses beside the other test flags.
    let cmd = parse_args(&args(&["test", "--trace", "tests/guiwright"])).unwrap();
    let Command::Test { trace, paths, .. } = cmd else {
        panic!("expected Command::Test, got {cmd:?}");
    };
    assert!(trace);
    assert_eq!(paths, vec![std::path::PathBuf::from("tests/guiwright")]);
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}
