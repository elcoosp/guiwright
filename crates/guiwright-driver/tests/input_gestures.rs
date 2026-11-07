//! Raw input & gesture integration tests (architecture §9 input pipeline,
//! M7): session-level synthesis and per-window routing, and the
//! remote-mode story over a real UDS socket — the `Input` request
//! un-blocked (`InputQueued`, the pre-M7 `Unsupported` hole), the
//! `Gesture` domain (`GestureDone`), and per-connection window targets
//! steering coordinates (the §11 + §16 v0.4 handoff M6 recorded).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::adapter::GesturePacing;
use guiwright_driver::gesture::Gesture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::{InputEvent, PointerButton, Session};
use guiwright_proto::schema::{
    ErrorCode, GestureV1, InputEventV1, PointerButtonV1, RequestV1, ResponseV1,
};
use guiwright_proto::uds::Client;

fn settle<A: guiwright_driver::FrameworkAdapter>(session: &mut Session<A>) {
    session.settle(std::time::Instant::now() + Duration::from_secs(5));
}

fn click(x: f64, y: f64, count: u32) -> Gesture {
    Gesture::Click {
        x,
        y,
        button: PointerButton::Primary,
        count,
    }
}

// ---------------------------------------------------------------------------
// Session level (embedded surface, no wire)
// ---------------------------------------------------------------------------

#[test]
fn gesture_click_synthesizes_move_down_up() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);

    let outcome = session
        .perform_gesture(click(10.0, 20.0, 1), None)
        .expect("click applies");
    assert_eq!(outcome.events, 3, "move + down + up");
    assert_eq!(outcome.detail, "clicked at (10, 20) — 3 events");
    assert!(outcome.applied_at_frame >= 1, "applied inside a frame");

    // The batch reached the adapter through the per-window surface, tagged
    // with the resolved default target (single-window → `main`).
    let logged = session.adapter().window_events();
    assert_eq!(logged.len(), 3);
    assert_eq!(logged[0].0, "main");
    assert_eq!(
        logged[0].1,
        InputEvent::PointerMove {
            x: 10.0,
            y: 20.0
        }
    );
    assert_eq!(
        logged[1].1,
        InputEvent::PointerDown {
            x: 10.0,
            y: 20.0,
            button: PointerButton::Primary
        }
    );
    assert_eq!(
        logged[2].1,
        InputEvent::PointerUp {
            x: 10.0,
            y: 20.0,
            button: PointerButton::Primary
        }
    );
}

#[test]
fn double_click_repeats_the_press_at_one_point() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);

    let outcome = session
        .perform_gesture(click(4.0, 8.0, 2), None)
        .expect("double click applies");
    assert_eq!(outcome.events, 5, "move + down/up ×2");
    assert_eq!(outcome.detail, "double-clicked at (4, 8) — 5 events");
    let logged = session.adapter().window_events();
    assert_eq!(logged.len(), 5);
    assert!(matches!(logged[1].1, InputEvent::PointerDown { .. }));
    assert!(matches!(logged[2].1, InputEvent::PointerUp { .. }));
    assert!(matches!(logged[3].1, InputEvent::PointerDown { .. }));
    assert!(matches!(logged[4].1, InputEvent::PointerUp { .. }));
}

/// A custom adapter whose pacing table overrides the §9 default (§18
/// "per-adapter gesture pacing tables").
struct SlowDragAdapter(MockAdapter);
impl guiwright_driver::FrameworkAdapter for SlowDragAdapter {
    fn meta(&self) -> &guiwright_driver::AdapterMeta {
        self.0.meta()
    }
    fn pump(&mut self, deadline: guiwright_driver::FrameDeadline) -> guiwright_driver::PumpOutcome {
        self.0.pump(deadline)
    }
    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        self.0.tree_update()
    }
    fn act(
        &mut self,
        node: accesskit::NodeId,
        action: guiwright_driver::SemAction,
    ) -> Result<guiwright_driver::ActionOutcome, guiwright_driver::AdapterError> {
        self.0.act(node, action)
    }
    fn gesture_pacing(&self) -> GesturePacing {
        GesturePacing {
            drag_ratchet_px: 25.0,
        }
    }
    fn input_window(
        &mut self,
        window: &str,
        ev: InputEvent,
    ) -> Result<(), guiwright_driver::AdapterError> {
        self.0.input_window(window, ev)
    }
}

#[test]
fn drag_interpolates_moves_with_the_pacing_ratchet() {
    // Default pacing: §9 "ratchet ~5 px/tick" — a 100 px drag synthesizes
    // 20 interpolated moves between down and up.
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);
    let outcome = session
        .perform_gesture(
            Gesture::Drag {
                from_x: 10.0,
                from_y: 10.0,
                to_x: 110.0,
                to_y: 10.0,
                button: PointerButton::Primary,
            },
            None,
        )
        .expect("drag applies");
    assert_eq!(outcome.events, 22, "down + 20 moves + up");
    assert_eq!(
        outcome.detail,
        "dragged (10, 10) → (110, 10) — 22 events"
    );
    let logged = session.adapter().window_events();
    assert_eq!(
        logged[0].1,
        InputEvent::PointerDown {
            x: 10.0,
            y: 10.0,
            button: PointerButton::Primary
        }
    );
    // First ratchet step lands one ratchet along the path; the last move
    // lands exactly on the target so the release coordinates are honest.
    assert_eq!(
        logged[1].1,
        InputEvent::PointerMove { x: 15.0, y: 10.0 }
    );
    assert_eq!(
        logged[20].1,
        InputEvent::PointerMove {
            x: 110.0,
            y: 10.0
        }
    );
    assert_eq!(
        logged[21].1,
        InputEvent::PointerUp {
            x: 110.0,
            y: 10.0,
            button: PointerButton::Primary
        }
    );

    // The adapter's pacing table drives the synthesis: 25 px ratchet →
    // 4 moves for the same 100 px drag.
    let mut slow = Session::new(SlowDragAdapter(MockAdapter::with_fixture(&Fixture::buttons())));
    settle(&mut slow);
    let outcome = slow
        .perform_gesture(
            Gesture::Drag {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 100.0,
                to_y: 0.0,
                button: PointerButton::Primary,
            },
            None,
        )
        .expect("slow drag applies");
    assert_eq!(outcome.events, 6, "down + 4 moves + up");
}

#[test]
fn key_and_type_gestures_synthesize_keyboard_events() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);

    let outcome = session
        .perform_gesture(
            Gesture::Key {
                key: "Enter".into(),
                modifiers: vec!["Shift".into()],
            },
            None,
        )
        .expect("key applies");
    assert_eq!(outcome.events, 2, "down + up");
    assert_eq!(outcome.detail, "pressed Shift+Enter — 2 events");
    let logged = session.adapter().window_events();
    assert_eq!(
        logged[0].1,
        InputEvent::KeyDown {
            key: "Enter".into(),
            modifiers: vec!["Shift".into()]
        }
    );
    assert_eq!(
        logged[1].1,
        InputEvent::KeyUp {
            key: "Enter".into(),
            modifiers: vec!["Shift".into()]
        }
    );

    let outcome = session
        .perform_gesture(
            Gesture::Type {
                text: "héllo".into(),
            },
            None,
        )
        .expect("type applies");
    assert_eq!(outcome.events, 1, "one text commit; per-adapter translation");
    assert_eq!(outcome.detail, "typed 5 chars — 1 event");
    assert_eq!(
        session.adapter().window_events()[2].1,
        InputEvent::TextInput {
            text: "héllo".into()
        }
    );
}

#[test]
fn wheel_gesture_routes_one_wheel_event() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);
    let outcome = session
        .perform_gesture(
            Gesture::Wheel {
                dx: 0.0,
                dy: -120.0,
            },
            None,
        )
        .expect("wheel applies");
    assert_eq!(outcome.events, 1);
    assert_eq!(outcome.detail, "wheeled (0, -120) — 1 event");
    assert_eq!(
        session.adapter().window_events()[0].1,
        InputEvent::Wheel { dx: 0.0, dy: -120.0 }
    );
}

#[test]
fn send_input_routes_raw_events_to_the_active_target() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);

    let frame = session
        .send_input(
            vec![InputEvent::PointerMove { x: 1.0, y: 2.0 }],
            None,
        )
        .expect("raw batch applies");
    assert!(frame >= 1);
    let logged = session.adapter().window_events();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].0, "main");
    assert_eq!(
        logged[0].1,
        InputEvent::PointerMove { x: 1.0, y: 2.0 }
    );

    // An empty batch is an honest no-op: one rendezvous, nothing injected.
    let before = session.adapter().window_events().len();
    session.send_input(Vec::new(), None).expect("empty batch ok");
    assert_eq!(session.adapter().window_events().len(), before);
}

#[test]
fn input_and_gestures_validate_the_window_target() {
    let fx = MultiWindowFixture::new();
    let mut session = Session::new(MockAdapter::with_multi_window(&fx));
    settle(&mut session);

    // Unknown handles fail clean — the tree is the source of truth (§16
    // v0.4) — and a failed gesture applies nothing.
    let err = session
        .perform_gesture(click(1.0, 2.0, 1), Some("00000000-0000-0000-0000-00000000dead"))
        .expect_err("unknown window");
    assert!(matches!(err, guiwright_core::CoreError::UnknownWindow { .. }), "{err}");
    assert!(session.adapter().window_events().is_empty());

    // An explicit target routes there; the default (None) is the session's
    // first window target.
    session
        .perform_gesture(click(30.0, 40.0, 1), Some(&fx.w2.0.to_string()))
        .expect("click into w2");
    let logged = session.adapter().window_events();
    assert_eq!(logged.len(), 3);
    assert!(logged.iter().all(|(w, _)| w == &fx.w2.0.to_string()));

    session
        .perform_gesture(click(50.0, 60.0, 1), None)
        .expect("default target");
    let logged = session.adapter().window_events();
    assert_eq!(logged.len(), 6);
    assert!(
        logged[3..].iter().all(|(w, _)| w == &fx.w1.0.to_string()),
        "the default target is the first window (got {logged:?})"
    );
}

#[test]
fn zero_count_click_is_rejected_before_any_injection() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);
    let err = session
        .perform_gesture(click(1.0, 2.0, 0), None)
        .expect_err("count 0 is a client bug");
    assert!(
        err.to_string().contains("count"),
        "the error names the problem: {err}"
    );
    assert!(session.adapter().window_events().is_empty());
}

// ---------------------------------------------------------------------------
// Remote mode (real UDS socket, §11)
// ---------------------------------------------------------------------------

fn socket_path(tag: &str) -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tmpdir")));
    dir.path().join(tag)
}

async fn connect(path: &std::path::Path) -> Client {
    let mut client = Client::connect(path).await.expect("client connects");
    client
        .handshake("guiwright-tests/0.1")
        .await
        .expect("hello");
    client
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_input_request_returns_input_queued() {
    let log: Arc<Mutex<Vec<(String, InputEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let path = socket_path("input-queued.sock");
    let server = spawn_with_log(&path, Fixture::buttons(), Arc::clone(&log));

    let mut conn = connect(&path).await;
    let resp = conn
        .request(RequestV1::Input {
            events: vec![InputEventV1::PointerMove { x: 5.0, y: 6.0 }],
        })
        .await
        .expect("input request");
    match resp {
        ResponseV1::InputQueued { frame_seq } => assert!(frame_seq >= 1),
        other => panic!("expected InputQueued, got {other:?}"),
    }
    let logged = log.lock().unwrap().clone();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].0, "main");

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gesture_request_returns_gesture_done() {
    let log: Arc<Mutex<Vec<(String, InputEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let path = socket_path("gesture-done.sock");
    let server = spawn_with_log(&path, Fixture::buttons(), Arc::clone(&log));

    let mut conn = connect(&path).await;
    let resp = conn
        .request(RequestV1::Gesture(GestureV1::Click {
            x: 12.5,
            y: 30.0,
            button: PointerButtonV1::Primary,
            count: 1,
        }))
        .await
        .expect("gesture request");
    match resp {
        ResponseV1::GestureDone { frame_seq, detail } => {
            assert!(frame_seq >= 1);
            assert_eq!(detail, "clicked at (12.5, 30) — 3 events");
        }
        other => panic!("expected GestureDone, got {other:?}"),
    }
    assert_eq!(log.lock().unwrap().len(), 3);

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_connections_inject_into_their_own_windows() {
    let fx = MultiWindowFixture::new();
    let log: Arc<Mutex<Vec<(String, InputEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let path = socket_path("gesture-two-windows.sock");
    let scripted = fx.clone();
    let shared = Arc::clone(&log);
    let server = guiwright_driver::spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted).with_input_log(Arc::clone(&shared))
    })
    .expect("server binds");

    let mut conn_a = connect(&path).await;
    let mut conn_b = connect(&path).await;
    for (conn, w1) in [(&mut conn_a, true), (&mut conn_b, false)] {
        let handle = if w1 { &fx.w1 } else { &fx.w2 };
        let resp = conn
            .request(RequestV1::Window(guiwright_proto::schema::WindowV1::Use {
                handle: handle.0.to_string(),
            }))
            .await
            .expect("use");
        assert!(matches!(resp, ResponseV1::WindowUsed { .. }));
    }

    // The same coordinates land in different windows — per-connection
    // targeting steers the input pipeline (§11, the M6 handoff).
    for conn in [&mut conn_a, &mut conn_b] {
        let resp = conn
            .request(RequestV1::Gesture(GestureV1::Click {
                x: 50.0,
                y: 50.0,
                button: PointerButtonV1::Primary,
                count: 1,
            }))
            .await
            .expect("gesture");
        assert!(matches!(resp, ResponseV1::GestureDone { .. }));
    }

    let logged = log.lock().unwrap().clone();
    assert_eq!(logged.len(), 6);
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();
    assert!(
        logged[..3].iter().all(|(w, _)| w == &w1),
        "connection A's click landed in w1: {logged:?}"
    );
    assert!(
        logged[3..].iter().all(|(w, _)| w == &w2),
        "connection B's click landed in w2: {logged:?}"
    );

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_window_gesture_fails_clean_over_the_wire() {
    let fx = MultiWindowFixture::new();
    let path = socket_path("gesture-unknown.sock");
    // Shared script channel: the "app" closes W2 mid-session (the same
    // pump stream every adapter uses).
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let scripted = fx.clone();
    let channel = scripts.clone();
    let server = guiwright_driver::spawn_session_server(path.clone(), move || {
        MockAdapter::with_multi_window(&scripted).with_shared_scripts(channel)
    })
    .expect("server binds");

    let mut conn = connect(&path).await;
    let resp = conn
        .request(RequestV1::Window(guiwright_proto::schema::WindowV1::Use {
            handle: fx.w2.0.to_string(),
        }))
        .await
        .expect("use");
    assert!(matches!(resp, ResponseV1::WindowUsed { .. }));

    // Close the window: the next rendezvous delivers the update and the
    // target stops resolving (the tree is the source of truth, §16 v0.4).
    scripts
        .lock()
        .expect("scripts")
        .push(fx.close_w2_update());
    conn.request(RequestV1::Ping).await.expect("rendezvous pump");

    // The stale target never reaches the adapter: the wire error carries
    // the UnknownWindow code (pre-M6 clients see a clean error).
    let err = conn
        .request(RequestV1::Gesture(GestureV1::Click {
            x: 1.0,
            y: 1.0,
            button: PointerButtonV1::Primary,
            count: 1,
        }))
        .await
        .expect_err("dead target");
    match err {
        guiwright_proto::GwpError::Rejected { code, .. } => {
            assert_eq!(code, ErrorCode::UnknownWindow);
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    server.shutdown();
}

/// Spawn a single-window server whose mock shares its input log with the
/// test (the `with_shared_scripts` pattern: the test observes what landed
/// without owning the session).
fn spawn_with_log(
    path: &std::path::Path,
    fixture: Fixture,
    log: Arc<Mutex<Vec<(String, InputEvent)>>>,
) -> guiwright_driver::GwpServerHandle {
    guiwright_driver::spawn_session_server(path.to_path_buf(), move || {
        MockAdapter::with_fixture(&fixture).with_input_log(Arc::clone(&log))
    })
    .expect("server binds")
}
