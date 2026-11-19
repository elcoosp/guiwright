//! End-to-end REPL conversations over real sockets (architecture §13
//! interactive session + §16 v0.4 REPL row). The M2 pattern repeats: the
//! framework-free mock adapter + a real socket is the launch-mode
//! contract — the same commands drive every adapter, GPUI included.
//!
//! The UDS test walks the full command surface (locators → actions →
//! clock → mock seams → codegen → errors → bye); the TCP test asserts
//! transport parity on the same conversation; the video test exercises
//! the §10 domain end-to-end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::TreeUpdate;
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
        guiwright_driver::server::Endpoint::Uds(_)
        | guiwright_driver::server::Endpoint::Ws(_)
        | guiwright_driver::server::Endpoint::InProcess => panic!("expected TCP endpoint"),
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

/// Transport parity (§7, M15): the same conversation over WebSocket — the
/// RFC 6455 door the REPL reaches with `--ws ws://host:port`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_conversation_over_ws() {
    let handle = GwpServerHandle::spawn_ws(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("ws server binds");
    let addr = match handle.endpoint() {
        guiwright_driver::server::Endpoint::Ws(addr) => *addr,
        other => panic!("expected WS endpoint, got {other:?}"),
    };

    let mut repl = Repl::connect(&ConnectTarget::Ws(addr.to_string()))
        .await
        .expect("repl attaches over ws");
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

/// The hit-test read through the REPL (M32): `hit <x> <y>` answers which
/// node occupies the point (physical px, the same space `get`'s rects
/// print), stores it as the current node (so `last` and the actions aim
/// at it — the click-what-you-see debugging loop), and a miss is an info
/// line, never an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_hit_test_read() {
    let path = socket_path("repl-hit.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // The Save button's center: identity + generation + bounds, in the
    // same shape `last` renders. The M37 pass renders the role/name
    // beside the handle — one `Describe` round trip behind the hit (the
    // m32 record's named rendering gap, closed).
    let out = rendered(&repl.step("hit 76 32").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].starts_with("hit (76, 32) → Button \"Save\" (accesskit#2)"),
        "{out:?}"
    );
    assert!(out[0].contains("(generation 0)"), "{out:?}");
    assert!(out[0].contains("rect=[16,16 → 136,48]"), "{out:?}");

    // The hit stored the current node: `last` shows it and the action
    // vocabulary aims at it — no second resolve.
    assert!(ok_line(&repl.step("last").await).contains("accesskit#2"));
    let click = repl.step("click").await;
    assert!(
        ok_line(&click).starts_with("accesskit#2: clicked (frame "),
        "{click:?}"
    );

    // A miss is an info line (the read is total), and it does not disturb
    // the current node.
    let miss = rendered(&repl.step("hit 400 220").await);
    assert_eq!(miss, vec!["nothing at (400, 220)"], "{miss:?}");
    assert!(ok_line(&repl.step("last").await).contains("accesskit#2"));

    handle.shutdown();
}

/// The REPL read-vocabulary pass (M35 — the m30 record's named REPL gap:
/// "`get`, `count` does not exist either; a REPL row lands with the
/// REPL's own read-vocabulary pass"): `count <get-args…>` answers the
/// set size and `all <get-args…>` enumerates the set rows — both reads
/// over the M30 `ResolveAll` domain, strict mode never consulted, the
/// current node untouched (a read, not a resolve-store), an empty set a
/// valid answer (the snapshot rule, never an error).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_read_vocabulary_count_and_all() {
    let path = socket_path("repl-reads.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // count: the set size in one line — the fixture's four buttons (the
    // set read is non-strict: hidden + disabled included), singular and
    // empty forms included.
    assert_eq!(
        rendered(&repl.step("count role Button").await),
        vec!["4 matches (Button)"],
    );
    assert_eq!(
        rendered(&repl.step("count role Button Save").await),
        vec!["1 match (Button \"Save\")"],
    );
    assert_eq!(
        rendered(&repl.step("count role Button Ghost").await),
        vec!["0 matches (Button \"Ghost\")"],
    );

    // all: one row per match, document order — the M39 enrichment (the
    // m37 record's standing gap): one DescribeAll round trip carries the
    // identity facts beside the generation and rect, the describe row's
    // rendering at set width.
    let out = rendered(&repl.step("all role Button").await);
    assert_eq!(out.len(), 4, "{out:?}");
    assert_eq!(
        out[0], "Button \"Save\" (generation 0) rect=[16,16 → 136,48]",
        "document order: Save first (node 2), facts at row width"
    );
    assert!(
        out[1].starts_with("Button \"Cancel\" (generation 0)"),
        "{out:?}"
    );
    assert!(
        out[2].starts_with("Button \"Invisible\" (generation 0)"),
        "{out:?}"
    );
    assert!(
        out[3].starts_with("Button \"Confirm\" (generation 0)"),
        "{out:?}"
    );

    // An empty set is one info line (the read is total, never an error).
    assert_eq!(
        rendered(&repl.step("all role Button Ghost").await),
        vec!["no matches"],
    );

    // A read, not a resolve-store: `last` keeps the node `get` aimed at
    // (the get's selector describe, not the reads' set rows).
    let get = repl.step("get role Button Save").await;
    assert!(ok_line(&get).starts_with("resolved "), "{get:?}");
    assert_eq!(
        rendered(&repl.step("count role Button").await),
        vec!["4 matches (Button)"],
    );
    assert!(ok_line(&repl.step("last").await).contains("\"Save\""));

    // The help lists the new reads (the vocabulary is discoverable).
    let help_text: Vec<String> = rendered(&repl.step("help").await);
    assert!(
        help_text.iter().any(|l| l.contains("count <get-args")),
        "{help_text:?}"
    );
    assert!(
        help_text.iter().any(|l| l.contains("all <get-args")),
        "{help_text:?}"
    );

    handle.shutdown();
}

/// The REPL bulk-read rows (M40 — the m35 record's named follow-up: "a
/// `texts <get-args…>` row lands when a debugging session asks for it —
/// one more client-side rendering over the same domain"): `texts` and
/// `values` read the set's *content* through the M39 `DescribeAll`
/// domain — the m29/m31 kit readers' REPL half, one round trip instead
/// of the m35 batch note's one `Describe` per row. Both are reads in the
/// M35 discipline (no resolve-store), with the kit readers' source
/// orders: `texts` reads label-first (what the text locators match),
/// `values` reads value-first (what the form carries) — the fixture's
/// Email input answers `"Email"` to one and `"ada@example.dev"` to the
/// other, so the two reads can never silently agree on the wrong datum.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_bulk_read_rows_texts_and_values() {
    let path = socket_path("repl-bulk-reads.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // The source orders on the same set: the Email input carries both a
    // label and a value, so `texts` answers the label (what the text
    // locators match, the m29 read) and `values` answers the value (what
    // the form carries, the m31 read).
    assert_eq!(
        rendered(&repl.step("texts role TextInput").await),
        vec![r#"accesskit#5 "Email""#],
        "texts reads label-first",
    );
    assert_eq!(
        rendered(&repl.step("values role TextInput").await),
        vec![r#"accesskit#5 "ada@example.dev""#],
        "values reads value-first",
    );

    // The button set walks in document order — hidden and disabled
    // included (the non-strict set semantics, the m30 rule), each row
    // carrying the handle form the action vocabulary aims with.
    let out = rendered(&repl.step("texts role Button").await);
    assert_eq!(out.len(), 4, "{out:?}");
    assert_eq!(out[0], r#"accesskit#2 "Save""#, "{out:?}");
    assert_eq!(out[1], r#"accesskit#3 "Cancel""#, "{out:?}");
    assert_eq!(out[2], r#"accesskit#6 "Invisible""#, "{out:?}");
    assert_eq!(out[3], r#"accesskit#7 "Confirm""#, "{out:?}");

    // An empty set is one info line (the read is total, never an error).
    assert_eq!(
        rendered(&repl.step("values role CheckBox").await),
        vec!["no matches"],
    );

    // A read, not a resolve-store: `last` keeps the node `get` aimed at
    // (the M35 read rule, unchanged for the bulk rows).
    let get = repl.step("get role Button Save").await;
    assert!(ok_line(&get).starts_with("resolved "), "{get:?}");
    rendered(&repl.step("texts role Button").await);
    assert!(ok_line(&repl.step("last").await).contains("\"Save\""));

    // The help lists the new rows (the vocabulary is discoverable).
    let help_text: Vec<String> = rendered(&repl.step("help").await);
    assert!(
        help_text.iter().any(|l| l.contains("texts <get-args")),
        "{help_text:?}"
    );
    assert!(
        help_text.iter().any(|l| l.contains("values <get-args")),
        "{help_text:?}"
    );

    handle.shutdown();
}

/// The bulk-read rows over a set where every input carries both a label
/// and a value (the M28 extension band): the walk stays 1:1 with the
/// document order — three inputs, three rows — and the two source orders
/// disagree on every labelled input, which is the point.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_bulk_read_rows_source_orders_on_labelled_inputs() {
    let path = socket_path("repl-bulk-reads-labelled.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::with_readonly())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    let texts = rendered(&repl.step("texts role TextInput").await);
    assert_eq!(
        texts,
        vec![
            r#"accesskit#5 "Email""#,
            r#"accesskit#12 "Bio""#,
            r#"accesskit#13 "Serial""#,
        ],
        "label-first, document order (5, 12, 13)"
    );
    let values = rendered(&repl.step("values role TextInput").await);
    assert_eq!(
        values,
        vec![
            r#"accesskit#5 "ada@example.dev""#,
            r#"accesskit#12 "hello""#,
            r#"accesskit#13 "GW-001""#,
        ],
        "value-first, document order (5, 12, 13)"
    );

    handle.shutdown();
}

/// The node-describe REPL pass (M37 — the m32/m36 records' named REPL
/// consumer: "the REPL renders the hit as the handle (`accesskit#2`) +
/// rect — role/name would ride a node-describe domain"): the `describe`
/// command reads one node's identity facts through the M36 `Describe`
/// domain, and the `hit` row renders the role/name beside the handle.
/// Both are reads in the M35 discipline: `last` and the action
/// vocabulary keep aiming where `get`/`hit` aimed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_describe_read() {
    let path = socket_path("repl-describe.sock");
    let handle = spawn_session_server(path.clone(), || {
        MockAdapter::with_fixture(&Fixture::buttons())
    })
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // The identity facts in one row — the dump row's facts, one node
    // instead of a walk (the M36 domain's REPL row).
    assert_eq!(
        rendered(&repl.step("describe role Button Save").await),
        vec![r#"Button "Save" (generation 0) rect=[16,16 → 136,48]"#],
        "{:?}",
        rendered(&repl.step("describe role Button Save").await),
    );

    // The value an input carries rides the same read (the fact the
    // `value` locator vocabulary matches).
    assert_eq!(
        rendered(&repl.step("describe role TextInput Email").await),
        vec![r#"TextInput "Email" value "ada@example.dev" (generation 0) rect=[16,128 → 256,156]"#],
    );

    // A read, not a resolve-store: `describe` never re-aims the current
    // node (the M35 read rule, unchanged for the M37 rows).
    let get = repl.step("get role Button Save").await;
    assert!(ok_line(&get).starts_with("resolved "), "{get:?}");
    rendered(&repl.step("describe role TextInput Email").await);
    assert!(ok_line(&repl.step("last").await).contains("\"Save\""));

    // A missing match is the strict-mode error (the same surface `get`
    // speaks), never a partial row.
    let miss = rendered(&repl.step("describe role Button Ghost").await);
    assert!(miss[0].starts_with("error:"), "{miss:?}");
    assert!(miss[0].contains("no node matches"), "{miss:?}");

    // The help lists the row (the vocabulary is discoverable).
    let help_text: Vec<String> = rendered(&repl.step("help").await);
    assert!(
        help_text.iter().any(|l| l.contains("describe <get-args")),
        "{help_text:?}"
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

/// M12 (§7 event channel) + the log-tail milestone: `events on` subscribes
/// to tree, frame **and** log kinds; tree/frame events and log records
/// flushed between commands print ahead of the next command's output;
/// `events off` stops the flow. The session spawns with the log tail
/// enabled (`ServerOptions::log_tail`) — the driver's opt-in for the
/// `Log` kind.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_events_command_flushes_between_commands() {
    use std::sync::{Arc, Mutex};

    use accesskit::{Node, NodeId, Role, TreeUpdate};
    use guiwright_driver::ServerOptions;

    let path = socket_path("repl-events.sock");
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);
    let handle = guiwright_driver::spawn_session_server_with(
        path.clone(),
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
        ServerOptions {
            log_tail: true,
            ..ServerOptions::default()
        },
    )
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // Subscribe: the ack names the honored kinds — now including the log
    // tail's records. The drain may already print events ahead of it (the
    // subscribe request's own pump presents a frame — events start flowing
    // immediately, that is the point).
    let out = rendered(&repl.step("events on").await);
    assert!(
        out.iter().any(|l| l
            == "events: subscribed (tree_changed, frame_presented, log) — flushed between commands"),
        "{out:?}"
    );
    // The subscription's own session-thread record (`event subscription
    // registered`) was captured while the command ran, so it flushes with
    // this step — pretty-printed by the drain renderer.
    assert!(
        out.iter().any(|l| l
            == "event log DEBUG guiwright_driver::server: event subscription registered"),
        "the tap's own record flushes with the subscribe step: {out:?}"
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

/// The REPL wait row (m43 — the m42 client-side wait's interactive half,
/// the m35/m40 one-milestone-per-surface pattern): `wait <state>
/// <get-args…>` rides the `ClientConn::wait_for` seam over a real socket
/// — attached/visible/hidden/stable, the `timeout <ms>` budget, the
/// typed timeout naming the state and the last evaluation, and the
/// `events on` filter restored after the wait's subscription window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_wait_rows_wait_on_the_client_seam() {
    let path = socket_path("repl-wait.sock");
    // The log tail is on: the `events on` row subscribes the Log kind too
    // (the m14 vocabulary), so the restore check below exercises the real
    // filter the interactive flow carries.
    let handle = guiwright_driver::spawn_session_server_with(
        path.clone(),
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::ServerOptions {
            log_tail: true,
            ..guiwright_driver::ServerOptions::default()
        },
    )
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    // A positive state answers one ok line naming the satisfying row —
    // the handle form the action vocabulary aims with.
    let out = rendered(&repl.step("wait visible role Button Save").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].starts_with("wait visible — "), "{out:?}");
    assert!(out[0].contains(r#"accesskit#2 "Save""#), "{out:?}");

    // Hidden is satisfied by absence (the fixture's "Invisible" button
    // carries no bounds) — the answer says so, not an error.
    let out = rendered(&repl.step("wait hidden role Button Invisible").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("satisfied by absence"), "{out:?}");

    // A spent budget: the typed timeout — state, locator, last evaluation
    // (the failures-name-names discipline over the REPL surface).
    assert_eq!(
        rendered(&repl.step("timeout 150").await)[0],
        "timeout = 150 ms"
    );
    let out = rendered(&repl.step("wait visible role Button Invisible").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].contains("wait visible timed out after 150 ms"),
        "{out:?}"
    );
    assert!(out[0].contains("no bounds"), "{out:?}");

    // The state word is validated: one error line, the session usable.
    let out = rendered(&repl.step("wait shiny role Button Save").await);
    assert_eq!(out.len(), 1, "{out:?}");

    // The wait owns the event filter for its duration — an `events on`
    // filter keeps flowing after (the row restores it, best effort).
    assert_eq!(
        rendered(&repl.step("timeout 5000").await)[0],
        "timeout = 5000 ms"
    );
    rendered(&repl.step("events on").await);
    rendered(&repl.step("wait visible role Button Save").await);
    // The restored filter arms the server's frame-cadence pump (§7, M12);
    // give it a few ticks so the next drain has something honest to flush.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let out = rendered(&repl.step("ping").await);
    assert!(
        out.iter().any(|l| l.starts_with("event ")),
        "the `events on` stream keeps flowing after a wait: {out:?}"
    );

    // A read, not a resolve-store: `last` keeps the node `get` aimed at.
    rendered(&repl.step("get role Button Cancel").await);
    rendered(&repl.step("wait visible role Button Save").await);
    assert!(ok_line(&repl.step("last").await).contains("\"Cancel\""));

    // The help lists the row (the vocabulary is discoverable).
    let help_text = rendered(&repl.step("help").await);
    assert!(
        help_text.iter().any(|l| l.contains("wait <state>")),
        "{help_text:?}"
    );

    handle.shutdown();
}

/// The REPL assertion rows (m61 — the m54/m59 client-seam assertions'
/// interactive half, the m43 one-milestone-per-surface pattern):
/// `assert text|value|count <get-args…> is <expected>` rides the
/// `ClientConn::expect_text` / `expect_value` / `expect_count` seam over
/// a real socket — the `timeout <ms>` budget, the satisfying-row /
/// found-count renders, absence answering found 0 on counts, the typed
/// timeout naming the seam's state word and the last evaluation, the
/// wake-and-re-read on a scripted tree update, the `events on` filter
/// restored after the assertion's subscription window, and the
/// read-not-store rule (`last` keeps the node `get` aimed at).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_assert_rows_wait_on_the_client_seam() {
    // The shared-script channel stands in for app-side UI changes: the
    // late-button update proves the wake-and-re-read below (the m59
    // wire-conversation shape over the interactive surface).
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_app = Arc::clone(&scripts);
    let path = socket_path("repl-assert.sock");
    let handle = guiwright_driver::spawn_session_server_with(
        path.clone(),
        move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts_for_app),
        guiwright_driver::ServerOptions {
            log_tail: true,
            ..guiwright_driver::ServerOptions::default()
        },
    )
    .expect("server binds");
    let mut repl = Repl::connect(&ConnectTarget::Uds(path))
        .await
        .expect("repl attaches");

    assert_rows_satisfied_on_entry(&mut repl).await;
    assert_rows_validate_their_grammar(&mut repl).await;
    assert_rows_print_the_typed_timeout(&mut repl).await;
    assert_rows_wake_and_re_read(&mut repl, &scripts).await;
    assert_rows_discoverable_and_append_only(&mut repl).await;

    handle.shutdown();
}

/// The satisfied forms: the content pair's satisfying-row render, the
/// count family's found-count answer, absence answering found 0, and the
/// read-not-store rule (`last` keeps the node `get` aimed at).
async fn assert_rows_satisfied_on_entry(repl: &mut Repl) {
    // A satisfied content assertion answers one ok line naming the row —
    // the m43 satisfying-row form over the m54 seam.
    let out = rendered(&repl.step(r#"assert text role Button Save is "Save""#).await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].starts_with("assert text — "), "{out:?}");
    assert!(
        out[0].contains(r#"Button "Save" (generation 0)"#),
        "{out:?}"
    );

    // The matcher rides the REPL's posture (substring, case-insensitive):
    // `sav` satisfies against "Save".
    let out = rendered(&repl.step(r#"assert text role Button Save is "sav""#).await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].starts_with("assert text — "), "{out:?}");

    // The value row reads the m31 order (the value first): the Email
    // input answers its value here, never its label.
    let out = rendered(
        &repl
            .step(r#"assert value role TextInput Email is "ada@example.dev""#)
            .await,
    );
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].starts_with("assert value — "), "{out:?}");
    assert!(
        out[0].contains(r#"TextInput "Email" value "ada@example.dev""#),
        "{out:?}"
    );

    // The identity rows (m62 — the m38 vocabulary over the door): the
    // role answers exactly (the AccessKit debug name), the name reads
    // the label-only fact.
    let out = rendered(&repl.step("assert role role Button Save is Button").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].starts_with("assert role — "), "{out:?}");
    assert!(
        out[0].contains(r#"Button "Save" (generation 0)"#),
        "{out:?}"
    );
    let out = rendered(&repl.step("assert name role TextInput Email is email").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].starts_with("assert name — "), "{out:?}");
    assert!(out[0].contains(r#"TextInput "Email""#), "{out:?}");

    // The count rows: the exact form and both comparators — the found
    // count is the answer (the m59 fact), the locator echoed in the
    // REPL's describe vocabulary.
    assert_eq!(
        rendered(&repl.step("assert count role Button is 4").await),
        vec!["assert count — found 4 (Button)"],
    );
    assert_eq!(
        rendered(&repl.step("assert count role Button is > 3").await),
        vec!["assert count — found 4 (Button)"],
    );
    assert_eq!(
        rendered(&repl.step("assert count role Button is < 5").await),
        vec!["assert count — found 4 (Button)"],
    );

    // Absence is an answer for counts (the m59 parity rule): an empty
    // set satisfies `is 0` — unlike the content pair's pending.
    assert_eq!(
        rendered(&repl.step("assert count testid ghost is 0").await),
        vec![r#"assert count — found 0 (testid "ghost")"#],
    );

    // A read, not a resolve-store: `last` keeps the node `get` aimed at
    // (the m35 read rule the m43 row rides).
    rendered(&repl.step("get role Button Cancel").await);
    rendered(&repl.step("assert count role Button is 4").await);
    assert!(ok_line(&repl.step("last").await).contains("\"Cancel\""));
}

/// The grammar validation: one error line naming the grammar per mistake,
/// the session usable after (the survival rule).
async fn assert_rows_validate_their_grammar(repl: &mut Repl) {
    let out = rendered(&repl.step("assert").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("text | value | count"), "{out:?}");
    let out = rendered(&repl.step("assert shiny role Button is 1").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("text | value | count"), "{out:?}");
    let out = rendered(&repl.step("assert count role Button").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("`is`"), "{out:?}");
    let out = rendered(&repl.step("assert count role Button is = 4").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("> <N>"), "{out:?}");
    assert!(out[0].contains("< <N>"), "{out:?}");
    assert_eq!(rendered(&repl.step("ping").await), vec!["pong"]);
}

/// A spent budget prints the typed timeout — the seam's state word
/// (`have text` / `have count`), the locator and the last evaluation
/// (expected and found; the failures-name-names discipline).
async fn assert_rows_print_the_typed_timeout(repl: &mut Repl) {
    assert_eq!(
        rendered(&repl.step("timeout 150").await)[0],
        "timeout = 150 ms"
    );
    let out = rendered(
        &repl
            .step(r#"assert text role Button Save is "Ghost""#)
            .await,
    );
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].contains("wait have text timed out after 150 ms"),
        "{out:?}"
    );
    assert!(out[0].contains(r#"expected "Ghost""#), "{out:?}");
    let out = rendered(&repl.step(r#"assert text testid ghost is "Ghost""#).await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].contains("no match yet (the set is empty)"),
        "{out:?}"
    );
    let out = rendered(&repl.step("assert count role Button is > 9").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].contains("wait have count timed out after 150 ms"),
        "{out:?}"
    );
    assert!(out[0].contains("count is 4, expected > 9"), "{out:?}");
    let out = rendered(&repl.step("assert count role Button is < 2").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("count is 4, expected < 2"), "{out:?}");
    let out = rendered(&repl.step("assert role role Button Save is TextInput").await);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].contains("wait have role timed out after 150 ms"),
        "{out:?}"
    );
    assert!(out[0].contains(r#"role is "Button""#), "{out:?}");
    assert!(out[0].contains(r#"expected "TextInput""#), "{out:?}");
    assert_eq!(
        rendered(&repl.step("timeout 5000").await)[0],
        "timeout = 5000 ms"
    );
}

/// The wake-and-re-read conversation: the assertion stays pending until
/// the scripted update joins a fifth button — event-driven, no polling —
/// and the assertion's subscription window restores the `events on`
/// filter afterwards (the m43 restore rule).
async fn assert_rows_wake_and_re_read(repl: &mut Repl, scripts: &Arc<Mutex<Vec<TreeUpdate>>>) {
    let scripts_for_push = Arc::clone(scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().add_button_update("Later"));
    });
    let out = rendered(&repl.step("assert count role Button is 5").await);
    assert_eq!(out, vec!["assert count — found 5 (Button)"], "{out:?}");

    rendered(&repl.step("events on").await);
    rendered(&repl.step("assert count role Button is 5").await);
    // The restored filter arms the server's frame-cadence pump (§7, M12);
    // give it a few ticks so the next drain has something honest to flush.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let out = rendered(&repl.step("ping").await);
    assert!(
        out.iter().any(|l| l.starts_with("event ")),
        "the `events on` stream keeps flowing after an assertion: {out:?}"
    );
    rendered(&repl.step("events off").await);
}

/// The vocabulary is discoverable (the help lists the rows) and the
/// append-only rule held: the legacy `expect` row is untouched.
async fn assert_rows_discoverable_and_append_only(repl: &mut Repl) {
    let help_text = rendered(&repl.step("help").await);
    assert!(
        help_text.iter().any(|l| l.contains("assert text")),
        "{help_text:?}"
    );
    assert!(
        help_text.iter().any(|l| l.contains("assert count")),
        "{help_text:?}"
    );

    let out = rendered(&repl.step("expect role Button Save").await);
    assert!(out[0].starts_with("resolved "), "{out:?}");
}
