//! End-to-end REPL conversations over real sockets (architecture §13
//! interactive session + §16 v0.4 REPL row). The M2 pattern repeats: the
//! framework-free mock adapter + a real socket is the launch-mode
//! contract — the same commands drive every adapter, GPUI included.
//!
//! The UDS test walks the full command surface (locators → actions →
//! clock → mock seams → codegen → errors → bye); the TCP test asserts
//! transport parity on the same conversation; the video test exercises
//! the §10 domain end-to-end.

use guiwright_cli::{ConnectTarget, Line, Repl};
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_driver::spawn_session_server;

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

/// Rendered output of one step (what the front-end would print).
fn rendered(outcome: &guiwright_cli::StepOutcome) -> Vec<String> {
    outcome.lines.iter().map(|line| line.render()).collect()
}

fn ok_line(outcome: &guiwright_cli::StepOutcome) -> &str {
    match outcome.lines.first() {
        Some(Line::Ok(text)) => text,
        other => panic!("expected an ok line first, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_repl_session_over_uds() {
    #[derive(Debug, Default)]
    struct ReplState {
        rows: usize,
        fixture: Option<String>,
    }
    let mut seams = guiwright_driver::SeamRegistry::new();
    seams.register("reset_state", |ctx, scope| {
        let state = ctx
            .downcast_mut::<ReplState>()
            .ok_or("wrong runner context type")?;
        state.rows = 0;
        state.fixture = Some(scope.to_string());
        Ok(format!("state reset to {scope}"))
    });
    seams.register("seed_users", |ctx, _scope| {
        let state = ctx
            .downcast_mut::<ReplState>()
            .ok_or("wrong runner context type")?;
        state.rows = 3;
        Ok("3 users seeded".into())
    });

    let path = socket_path("repl-uds.sock");
    let handle = GwpServerHandle::spawn_with_seams(
        path.clone(),
        || {
            MockAdapter::with_fixture(&Fixture::buttons()).with_state(ReplState {
                rows: 9,
                fixture: None,
            })
        },
        seams,
    )
    .expect("server binds");

    // Attach + handshake (§7): the adapter identity rides `Welcome`.
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");
    assert_eq!(repl.adapter().name, "mock");
    assert_eq!(repl.adapter().framework, "fixture");

    // Blank lines and comments produce no output.
    let empty = repl.step("   ").await;
    assert!(empty.lines.is_empty());
    let comment = repl.step("# a comment").await;
    assert!(comment.lines.is_empty());

    // Liveness.
    assert_eq!(rendered(&repl.step("ping").await), vec!["pong"]);

    // Snapshot: the fixture tree settled at attach (§6.3).
    let snapshot = repl.step("snapshot").await;
    assert_eq!(snapshot.lines.len(), 1);
    let line = ok_line(&snapshot);
    assert!(line.ends_with("8 nodes"), "{line}");
    assert!(line.starts_with("tree @"), "{line}");

    // Unknown commands are single error lines; the session survives.
    let unknown = repl.step("frobnicate").await;
    assert_eq!(
        rendered(&unknown),
        vec!["error: unknown command `frobnicate` — type `help` for the command list"]
    );
    assert_eq!(rendered(&repl.step("ping").await), vec!["pong"]);

    // Actions before a resolve fail with a hint, not a panic.
    let no_node = repl.step("hover").await;
    assert_eq!(
        rendered(&no_node),
        vec!["error: no node resolved — run `get …` first"]
    );

    // Locator resolution (§8): lowercase role canonicalizes; the rect is
    // the fixture's Save button bounds.
    let save = repl.step("get role button Save").await;
    assert_eq!(
        ok_line(&save),
        "resolved Button \"Save\" (generation 0) rect=[16,16 → 136,48]"
    );
    assert!(repl.current().is_some());

    // `last` shows the current handle.
    assert_eq!(
        rendered(&repl.step("last").await),
        vec![r#"Button "Save" (generation 0) rect=[16,16 → 136,48]"#]
    );

    // Actions apply at a frame boundary (§15.4).
    let click = repl.step("click").await;
    let line = ok_line(&click);
    assert!(
        line.starts_with(r#"Button "Save": clicked (frame "#),
        "{line}"
    );
    assert!(line.ends_with(')'), "{line}");

    // Strict-mode violation: two buttons match — a single error line and
    // a usable session (§8.2).
    let ambiguous = repl.step("get role Button").await;
    assert_eq!(ambiguous.lines.len(), 1);
    assert!(
        rendered(&ambiguous)[0].starts_with("error: strict-mode:"),
        "{:?}",
        rendered(&ambiguous)
    );

    // Fill flows through SetValue on the email input.
    let email = repl.step("get role TextInput Email").await;
    assert_eq!(
        ok_line(&email),
        "resolved TextInput \"Email\" (generation 0) rect=[16,128 → 256,156]"
    );
    let fill = repl.step("fill ada@lovelace.dev").await;
    let line = ok_line(&fill);
    assert!(line.starts_with(r#"TextInput "Email": "#), "{line}");
    assert!(line.contains("(frame "), "{line}");

    // Quoted arguments survive spaces.
    let welcome = repl.step(r#"get text "Welcome, Ada""#).await;
    assert_eq!(
        ok_line(&welcome),
        "resolved text \"Welcome, Ada\" (generation 0) rect=[16,96 → 216,120]"
    );

    // Clock domain (§14): install + stepped fast-forward, reported in ms.
    assert_eq!(
        rendered(&repl.step("clock install").await),
        vec!["clock: virtual=0ms paused=false steps=0"]
    );
    let ff = repl.step("clock ff 50").await;
    let line = ok_line(&ff);
    // 50ms folds into frame-sized 16.67ms steps and settles between them.
    assert!(
        line.starts_with("clock: virtual=50ms paused=false steps="),
        "{line}"
    );
    assert!(!line.ends_with("steps=0"), "{line}");

    // Mock domain (§14 state seams): canned override → clear → real hook
    // → reset, exactly the M2 spike flow over REPL commands.
    assert_eq!(
        rendered(&repl.step("mock install seed_users canned").await),
        vec!["mock seed_users: seam override installed"]
    );
    assert_eq!(
        rendered(&repl.step("mock call seed_users demo").await),
        vec!["mock seed_users: canned"]
    );
    assert_eq!(
        rendered(&repl.step("mock clear seed_users").await),
        vec!["mock seed_users: seam override cleared"]
    );
    assert_eq!(
        rendered(&repl.step("mock call seed_users demo").await),
        vec!["mock seed_users: 3 users seeded"]
    );
    assert_eq!(
        rendered(&repl.step("mock reset fresh").await),
        vec!["mock fresh: state reset to fresh"]
    );

    // Codegen over the wire (§13, M4): recorded actions emit kit code.
    assert_eq!(
        rendered(&repl.step("codegen start").await).len(),
        1,
        "start acknowledges"
    );
    let _ = repl.step("get role Button Save").await;
    let _ = repl.step("click").await;
    let stop = repl.step("codegen stop").await;
    assert_eq!(
        ok_line(&stop),
        "codegen: 1 line(s), 0 warning(s)",
        "{:?}",
        rendered(&stop)
    );
    assert_eq!(
        rendered(&stop)[1],
        r#"app.get_by_role(Role::Button, "Save").click();"#
    );

    // Codegen stop without a recording is a clean error.
    let stop_again = repl.step("codegen stop").await;
    assert_eq!(
        rendered(&stop_again),
        vec!["error: Internal: no codegen recording is active"]
    );

    // Timeout path (§8.2): a short deadline surfaces the server-side
    // auto-wait failure as one error line.
    let _ = repl.step("timeout 100").await;
    let missing = repl.step("get testid does-not-exist").await;
    assert_eq!(missing.lines.len(), 1);
    assert!(
        rendered(&missing)[0].starts_with("error: timeout:"),
        "{:?}",
        rendered(&missing)
    );

    // Malformed arguments are single error lines.
    assert_eq!(
        rendered(&repl.step("clock ff").await),
        vec!["error: a millisecond value is required"]
    );
    assert_eq!(
        rendered(&repl.step("clock ff soon").await),
        vec!["error: not a millisecond value: soon"]
    );
    assert_eq!(
        rendered(&repl.step("get frob x").await),
        vec!["error: unknown locator `frob` — role | text | label | testid | value"]
    );
    assert_eq!(
        rendered(&repl.step(r#"fill "unterminated"#).await),
        vec!["error: unterminated quoted argument"]
    );

    // Clean close (§7 `Bye`).
    let bye = repl.step("bye").await;
    assert_eq!(rendered(&bye), vec!["session closed"]);
    assert!(bye.exit, "bye requests the loop exit");

    handle.shutdown();
}

/// Transport parity (§7): the same conversation over TCP.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_conversation_over_tcp() {
    let handle = GwpServerHandle::spawn_tcp(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("tcp server binds");
    let addr = match handle.endpoint() {
        guiwright_driver::server::Endpoint::Tcp(addr) => *addr,
        other => panic!("expected TCP endpoint, got {other:?}"),
    };

    let mut repl = Repl::connect(&ConnectTarget::Tcp(addr.to_string()))
        .await
        .expect("repl attaches over tcp");
    assert_eq!(repl.adapter().name, "mock");

    assert_eq!(rendered(&repl.step("ping").await), vec!["pong"]);
    assert!(ok_line(&repl.step("snapshot").await).ends_with("8 nodes"));
    assert_eq!(
        ok_line(&repl.step("get role button Save").await),
        "resolved Button \"Save\" (generation 0) rect=[16,16 → 136,48]"
    );
    let click = repl.step("click").await;
    assert!(
        ok_line(&click).starts_with(r#"Button "Save": clicked (frame "#),
        "{:?}",
        rendered(&click)
    );
    let bye = repl.step("exit").await;
    assert!(bye.exit);

    handle.shutdown();
}

/// The §10 video domain through the REPL: start → drive → stop returns a
/// finalized artifact (PNG-sequence fallback encoder, no ffmpeg needed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_video_domain() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = socket_path("repl-video.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
            24,
            12,
            vec![64; 24 * 12 * 4],
        )
    })
    .expect("server binds");

    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");
    assert_eq!(
        rendered(
            &repl
                .step(&format!("video start 25 {}", dir.path().display()))
                .await
        ),
        vec!["video: recording started"]
    );
    // Drive the session: the resolve pumps, frames feed the recorder.
    let _ = repl.step("get role Button Save").await;
    let stop = repl.step("video stop").await;
    let line = ok_line(&stop);
    assert!(line.starts_with("video: "), "{line}");
    // `video: <path> (N frames, D dropped, Ms)` — the §10 summary; the
    // hash-equality dedup means a static UI records at least one frame.
    assert!(line.contains(" frames, "), "{line}");
    assert!(line.contains(" dropped, "), "{line}");
    assert!(line.ends_with("ms)"), "{line}");
    let artifact = line
        .trim_start_matches("video: ")
        .split(" (")
        .next()
        .unwrap_or("");
    assert!(
        std::path::Path::new(artifact).exists(),
        "artifact exists at {artifact}"
    );

    // Stop without a running recording is a clean error.
    let stop_again = repl.step("video stop").await;
    assert_eq!(stop_again.lines.len(), 1);
    assert!(rendered(&stop_again)[0].starts_with("error:"));

    handle.shutdown();
}

/// `help` lists every command family; `timeout` round-trips.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn help_and_timeout() {
    let path = socket_path("repl-help.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    let help = repl.step("help").await;
    let help_text: Vec<String> = rendered(&help);
    assert!(help_text.len() >= 15, "{help_text:?}");
    assert_eq!(help_text[0], "commands:");
    // Every command line is indented; the intro and the footer are not.
    assert!(
        help_text[1..help_text.len() - 1]
            .iter()
            .all(|l| l.starts_with("  ")),
        "{help_text:?}"
    );
    assert!(help_text.iter().any(|l| l.contains("codegen start")));
    assert!(help_text.iter().any(|l| l.contains("clock install")));

    assert_eq!(
        rendered(&repl.step("timeout 250").await),
        vec!["timeout = 250 ms"]
    );
    assert_eq!(repl.timeout_ms(), 250);

    handle.shutdown();
}

/// M6 (§16 v0.4 multi-window): `window list` / `window use` aim the REPL's
/// connection at one window target — the same commands the runner's spec
/// steps use (the runner drives `Repl::step`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_window_domain() {
    let fx = MultiWindowFixture::new();
    let scripted = fx.clone();
    let path = socket_path("repl-window.sock");
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted)
    })
    .expect("server binds");

    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // List: both targets with titles, none marked active yet.
    let out = rendered(&repl.step("window list").await);
    assert_eq!(out.len(), 3, "{out:?}");
    assert!(out[0].starts_with("window targets"), "{out:?}");
    let w1_handle = fx.w1.0.to_string();
    let w2_handle = fx.w2.0.to_string();
    let w1_line = out
        .iter()
        .find(|l| l.contains(&w1_handle))
        .expect("w1 line");
    assert!(w1_line.contains("Editor"), "{w1_line}");
    let w2_line = out
        .iter()
        .find(|l| l.contains(&w2_handle))
        .expect("w2 line");
    assert!(w2_line.contains("Preview"), "{w2_line}");
    assert!(
        !out.iter().any(|l| l.starts_with("*")),
        "no active target yet: {out:?}"
    );

    // Use: the connection is aimed at the editor window.
    let out = rendered(&repl.step(&format!("window use {w1_handle}")).await);
    assert_eq!(out, vec![format!("now targeting {w1_handle} (Editor)")]);

    // The list marks the active target.
    let out = rendered(&repl.step("window list").await);
    let active = out
        .iter()
        .find(|l| l.starts_with('*'))
        .expect("active mark");
    assert!(active.contains(&w1_handle), "{out:?}");

    // Locator commands now see ONLY the editor window: "Save" is unique
    // there (strict mode would fire on the unscoped two-window surface).
    // The two windows' Save buttons live at different rects — the resolved
    // rect identifies which window's node was found.
    let out = rendered(&repl.step("get role button Save").await);
    assert!(out[0].starts_with("resolved"), "{out:?}");
    assert!(out[0].contains("rect=[16,16"), "{out:?}");

    // Unknown handles fail clean and change nothing.
    let out = rendered(&repl.step("window use no-such-window").await);
    assert_eq!(out.len(), 1);
    assert!(out[0].starts_with("error:"), "{out:?}");
    assert!(out[0].contains("no window"), "{out:?}");

    // The active target is unchanged — re-aim at the preview window.
    let out = rendered(&repl.step(&format!("window use {w2_handle}")).await);
    assert_eq!(out, vec![format!("now targeting {w2_handle} (Preview)")]);
    let out = rendered(&repl.step("get role button Save").await);
    assert!(out[0].contains("rect=[12,416"), "{out:?}");

    // Usage errors.
    let out = rendered(&repl.step("window").await);
    assert!(out[0].starts_with("error:"), "{out:?}");
    let out = rendered(&repl.step("window frobnicate").await);
    assert!(out[0].starts_with("error:"), "{out:?}");

    handle.shutdown();
}
