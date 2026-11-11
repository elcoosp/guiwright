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
    outcome
        .lines
        .iter()
        .map(guiwright_cli::Line::render)
        .collect()
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
    assert_eq!(empty.lines.len(), 0);
    let comment = repl.step("# a comment").await;
    assert_eq!(comment.lines.len(), 0);

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

    // Locator + action surface (§8/§15.4), clock + mock domains (§14).
    locator_and_action_surface(&mut repl).await;
    clock_and_mock_domains(&mut repl).await;

    // Codegen recording (§13, M4 + the §13-over-§9 gesture vocabulary).
    codegen_recording(&mut repl).await;

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

    // Inspector overlay session (§13, M8): enable, highlight, clear, off
    // (after the short timeout so the missing-node strict failure is fast).
    inspect_overlay_session(&mut repl).await;

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
        guiwright_driver::server::Endpoint::Uds(_) => {
            panic!("expected TCP endpoint")
        }
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

/// The §7 render domain through the REPL (M10): `screenshot [path]` pulls
/// the window PNG onto the client's filesystem — the client owns the
/// artifact path (Playwright parity), the summary line carries the
/// dimensions and the frame the pixels came from.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_screenshot_domain() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = socket_path("repl-shot.sock");
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

    let artifact = dir.path().join("window.png");
    let out = rendered(
        &repl
            .step(&format!("screenshot {}", artifact.display()))
            .await,
    );
    assert_eq!(out.len(), 1, "{out:?}");
    // `shot: <path> (W×H, frame N)` — the frame number rides the session's
    // pump history (handshake + attach already presented frames), so the
    // shape is pinned, not the exact value.
    let line = &out[0];
    let rest = line
        .strip_prefix(&format!("shot: {} (", artifact.display()))
        .unwrap_or_else(|| panic!("shot line shape: {line}"));
    assert!(
        rest.starts_with("24×12, frame ") && rest.ends_with(')'),
        "{line}"
    );
    // The artifact is a real PNG with the scripted dimensions.
    let bytes = std::fs::read(&artifact).expect("shot written");
    let image = guiwright_shot::codec::decode_png(&bytes).expect("valid PNG");
    assert_eq!((image.width, image.height), (24, 12));
    assert_eq!(image.pixel(3, 5), [64, 64, 64, 64]);

    // An adapter without a pixel path is an honest error, not a file.
    let bare = socket_path("repl-shot-bare.sock");
    let bare_handle = spawn_session_server(bare.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut bare_repl = Repl::connect(&ConnectTarget::Uds(bare))
        .await
        .expect("repl attaches");
    let miss = rendered(&bare_repl.step("screenshot").await);
    assert_eq!(miss.len(), 1, "{miss:?}");
    assert!(
        miss[0].starts_with("error:") && miss[0].contains("Unsupported"),
        "{miss:?}"
    );
    bare_handle.shutdown();

    handle.shutdown();
}

/// The §7 tree-dump domain through the REPL (M11): `snapshot full` renders
/// the whole tree depth-first, indented by the children counts, the
/// focused node marked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_snapshot_full_dump() {
    let path = socket_path("repl-dump.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // The summary form is unchanged (the M0 reply).
    assert_eq!(rendered(&repl.step("snapshot").await).len(), 1);

    let out = rendered(&repl.step("snapshot full").await);
    // Header + 8 rows: window, group, six fixture children — the group's
    // six children all at one indent level under it. The window root
    // carries the focus marker and (like the fixture laid it out) no
    // bounds; the hidden "Invisible" button has bounds too.
    assert_eq!(out.len(), 9, "{out:?}");
    assert!(out[0].ends_with("nodes (full)"), "{out:?}");
    assert_eq!(out[1], "Window \"Test App\" (focus)", "{out:?}");
    assert_eq!(out[2], "  Group", "{out:?}");
    assert_eq!(out[3], "    Button \"Save\" @ [16,16 → 136,48]", "{out:?}");
    assert_eq!(
        out[6], "    TextInput \"Email\" = ada@example.dev @ [16,128 → 256,156]",
        "{out:?}"
    );
    assert_eq!(out[7], "    Button \"Invisible\"", "{out:?}");
    assert_eq!(
        out[8], "    Button \"Confirm\" @ [16,164 → 136,196]",
        "{out:?}"
    );

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
        !out.iter().any(|l| l.starts_with('*')),
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

/// M7 (§9 input pipeline): `mouse` / `key` / `text` — the raw-input and
/// gesture vocabulary. Coordinates are relative to the connection's active
/// window target (the M6 `window use` story feeds straight in); the
/// session synthesizes the raw batch driver-side (per-adapter pacing).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_input_and_gestures() {
    let path = socket_path("repl-gestures.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");

    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // Gesture click: the synthesized batch (move→down→up) applies inside
    // one frame; the detail names the synthesis.
    let out = rendered(&repl.step("mouse click 10 20").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].starts_with("clicked at (10, 20) — 3 events @ frame "),
        "{out:?}"
    );

    // Double-click (count = 2), a named button, and the drag ratchet
    // (100 px @ 5 px/tick → 20 interpolated moves).
    let out = rendered(&repl.step("mouse dblclick 4 8").await);
    assert!(
        out[0].starts_with("double-clicked at (4, 8) — 5 events @ frame "),
        "{out:?}"
    );
    let out = rendered(&repl.step("mouse click 1 2 right").await);
    assert!(
        out[0].starts_with("clicked at (1, 2) — 3 events @ frame "),
        "{out:?}"
    );
    let out = rendered(&repl.step("mouse drag 0 0 100 0").await);
    assert!(
        out[0].starts_with("dragged (0, 0) → (100, 0) — 22 events @ frame "),
        "{out:?}"
    );

    // Wheel and the keyboard vocabulary.
    let out = rendered(&repl.step("mouse wheel 0 -120").await);
    assert!(
        out[0].starts_with("wheeled (0, -120) — 1 event @ frame "),
        "{out:?}"
    );
    let out = rendered(&repl.step("key Enter").await);
    assert!(
        out[0].starts_with("pressed Enter — 2 events @ frame "),
        "{out:?}"
    );
    let out = rendered(&repl.step("key a ctrl shift").await);
    assert!(
        out[0].starts_with("pressed ctrl+shift+a — 2 events @ frame "),
        "{out:?}"
    );
    let out = rendered(&repl.step("text héllo").await);
    assert!(
        out[0].starts_with("typed 5 chars — 1 event @ frame "),
        "{out:?}"
    );
    // The quoting tokenizer keeps multi-word text one argument.
    let out = rendered(&repl.step("text \"two words\"").await);
    assert!(
        out[0].starts_with("typed 9 chars — 1 event @ frame "),
        "{out:?}"
    );

    // The raw primitive events ride the (pre-M7 dead) `Input` request.
    for cmd in ["mouse move 5 6", "mouse down 1 2", "mouse up 1 2"] {
        let out = rendered(&repl.step(cmd).await);
        assert_eq!(out.len(), 1, "{cmd}: {out:?}");
        assert!(
            out[0].starts_with("input queued @ frame "),
            "{cmd}: {out:?}"
        );
    }

    // Usage errors are single error lines and leave the session usable.
    for cmd in [
        "mouse",
        "mouse frobnicate",
        "mouse click 10",
        "mouse click abc 20",
        "mouse drag 1 2 3",
        "mouse wheel 0",
        "key",
        "text",
    ] {
        let out = rendered(&repl.step(cmd).await);
        assert_eq!(out.len(), 1, "{cmd}: {out:?}");
        assert!(out[0].starts_with("error:"), "{cmd}: {out:?}");
    }
    let out = rendered(&repl.step("ping").await);
    assert_eq!(out, vec!["pong"], "session usable after errors");

    // The help text names the new domains.
    let out = rendered(&repl.step("help").await);
    assert!(out.iter().any(|l| l.contains("mouse click")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("key <key>")), "{out:?}");

    handle.shutdown();
}

/// The §8 locator/action surface over one session: resolution, `last`,
/// frame-boundary actions, strict mode, `fill`, quoted arguments.
async fn locator_and_action_surface(repl: &mut Repl) {
    // Lowercase role canonicalizes; the rect is the fixture's Save button
    // bounds.
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
}

/// The §14 clock + mock domains over one session: stepped fast-forward,
/// canned seams → clear → real hook → reset (the M2 spike flow).
async fn clock_and_mock_domains(repl: &mut Repl) {
    // Clock (§14): install + stepped fast-forward, reported in ms.
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
    assert_ne!(line.chars().last(), Some('0'), "{line}");

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
}

/// The §13 overlay conversation (M8): `inspect on`, `inspect <get-args…>`
/// (resolve + highlight in one step), `inspect clear`, `inspect off` —
/// every step answers `Pong` and the session survives unknown nodes.
async fn inspect_overlay_session(repl: &mut Repl) {
    assert_eq!(
        ok_line(&repl.step("inspect on").await),
        "inspect: overlay on — badges track interactive nodes"
    );

    // `inspect <get-args…>` resolves like `get` and highlights: two ok
    // lines — the resolution + the highlight acknowledgement.
    let highlighted = repl.step("inspect role Button Save").await;
    let lines = rendered(&highlighted);
    assert!(
        lines[0].starts_with("resolved Button \"Save\""),
        "{lines:?}"
    );
    assert_eq!(lines[1], "highlighted Button \"Save\"", "{lines:?}");

    // The outline rides the current node: `last` still knows it (the
    // exact `get` bookkeeping).
    assert!(ok_line(&repl.step("last").await).contains("Save"));

    // Clear keeps the session; off ends it.
    assert_eq!(
        ok_line(&repl.step("inspect clear").await),
        "inspect: outline cleared"
    );
    assert_eq!(
        ok_line(&repl.step("inspect off").await),
        "inspect: overlay off"
    );

    // Highlighting an unknown node is a clean error; the session
    // survives (a debugging tool must survive its user's mistakes).
    let missing = repl.step("inspect role Button Missing").await;
    assert_eq!(missing.lines.len(), 1, "{:?}", rendered(&missing));
    assert!(
        rendered(&missing)[0].starts_with("error: timeout:"),
        "the resolve half fails first (auto-wait deadline), got {:?}",
        rendered(&missing)
    );
    assert_eq!(rendered(&repl.step("ping").await), vec!["pong"]);
}

async fn codegen_recording(repl: &mut Repl) {
    // The §13 codegen surface over one session: locator actions record as
    // kit code, stop-without-recording is a clean error, and the gesture
    // vocabulary (§13 over §9) records as `app.mouse()` / `app.keyboard()`
    // calls.
    // Locator actions record as kit code (§13, M4).
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

    // The gesture vocabulary records too (§13 over §9): the REPL's mouse /
    // key commands land as kit code while a recording runs.
    assert_eq!(
        rendered(&repl.step("codegen start").await).len(),
        1,
        "start acknowledges"
    );
    let _ = repl.step("mouse click 10 20").await;
    let _ = repl.step("key Enter").await;
    let stop_gestures = repl.step("codegen stop").await;
    assert_eq!(
        ok_line(&stop_gestures),
        "codegen: 2 line(s), 0 warning(s)",
        "{:?}",
        rendered(&stop_gestures)
    );
    assert_eq!(
        rendered(&stop_gestures)[1],
        "app.mouse().click(10.0, 20.0);"
    );
    assert_eq!(
        rendered(&stop_gestures)[2],
        "app.keyboard().press(\"Enter\");"
    );
}

/// M13 (§7 tree domain): `snapshot under <get-args…>` renders only the
/// match's subtree — same rows, same indentation rules, subtree-scoped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_snapshot_under_dump() {
    let path = socket_path("repl-dump-under.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    let out = rendered(&repl.step("snapshot under role Group").await);
    // Header + the Group and its six children (the window root excluded).
    assert_eq!(out.len(), 8, "{out:?}");
    assert!(out[0].ends_with("nodes (under Group)"), "{out:?}");
    assert_eq!(out[1], "Group", "{out:?}");
    assert_eq!(out[2], "  Button \"Save\" @ [16,16 → 136,48]", "{out:?}");
    assert_eq!(
        out[5], "  TextInput \"Email\" = ada@example.dev @ [16,128 → 256,156]",
        "{out:?}"
    );

    // A miss is the runner-shaped rejection: with the default timeout it
    // auto-waits and expires as `timeout`; at timeout 0 it fails fast as
    // strict-mode (the same wording `expect` uses).
    assert_eq!(
        rendered(&repl.step("timeout 0").await),
        vec!["timeout = 0 ms"]
    );
    let err = repl.step("snapshot under role TabPanel").await;
    assert!(
        matches!(err.lines.first(), Some(Line::Err(text)) if text.contains("strict-mode")),
        "{err:?}"
    );
    assert_eq!(
        rendered(&repl.step("timeout 5000").await),
        vec!["timeout = 5000 ms"]
    );

    handle.shutdown();
}

/// M12 (§7 event channel): `events on` subscribes; tree/frame events
/// flushed between commands print ahead of the next command's output;
/// `events off` stops the flow.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_events_command_flushes_between_commands() {
    use std::sync::{Arc, Mutex};

    use accesskit::{Node, NodeId, Role, TreeUpdate};

    let path = socket_path("repl-events.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = spawn_session_server(path.clone(), move || {
        MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts)
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // Subscribe: the ack names the honored kinds. The drain may already
    // print events ahead of it (the subscribe request's own pump presents
    // a frame — events start flowing immediately, that is the point).
    let out = rendered(&repl.step("events on").await);
    assert!(
        out.iter().any(|l| l
            == "events: subscribed (tree_changed, frame_presented) — flushed between commands"),
        "{out:?}"
    );

    // Script a rename; the events flush with the NEXT command's output
    // (the line-oriented contract — what lands mid-prompt shows up then).
    let mut save = Node::new(Role::Button);
    save.set_label("Save (renamed)");
    save.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 16.0,
        x1: 136.0,
        y1: 48.0,
    });
    shared.lock().unwrap().push(TreeUpdate {
        nodes: vec![(NodeId(2), save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    });

    // Give the session thread's frame tick a moment to pump + publish,
    // then run any command: the drain prints ahead of its output.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let out = rendered(&repl.step("ping").await);
    assert_eq!(out.last(), Some(&"pong".to_string()), "{out:?}");
    let events: Vec<&String> = out.iter().filter(|l| l.starts_with("event ")).collect();
    assert!(
        events.iter().any(|l| l.contains("tree_changed")),
        "the tree event flushed between commands: {out:?}"
    );
    assert!(
        events.iter().any(|l| l.contains("frame_presented")),
        "frame events flush too (one pass per pump): {out:?}"
    );

    // Off: the flow stops.
    assert_eq!(
        rendered(&repl.step("events off").await)[0],
        "events: unsubscribed"
    );
    let out = rendered(&repl.step("ping").await);
    assert_eq!(out, vec!["pong"], "no events after `off`");

    handle.shutdown();
}
