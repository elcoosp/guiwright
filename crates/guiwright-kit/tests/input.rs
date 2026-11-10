//! The embedded input surface (§9 over the kit, the M7 deferred row
//! "kit-embedded input"): `app.mouse()` / `app.keyboard()` drive the
//! session's gesture synthesis and raw-input injection directly — the
//! same Playwright-shaped vocabulary the REPL and the TypeScript client
//! speak over the wire, embedded and in-process.

use std::io::Read;

use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::{InputEvent, PointerButton};
use guiwright_kit::App;

fn settle<A: guiwright_driver::FrameworkAdapter>(app: &mut App<A>) {
    app.wait_for_settle();
}

// ---------------------------------------------------------------------------
// Gestures through the handles
// ---------------------------------------------------------------------------

#[test]
fn mouse_click_reaches_the_adapter_through_the_session() {
    let mut app = App::launch_fixture("mouse_click");
    settle(&mut app);

    let outcome = app.mouse().click(10.0, 20.0).expect("click applies");
    assert_eq!(outcome.events, 3, "move + down + up");
    assert_eq!(outcome.detail, "clicked at (10, 20) — 3 events");
    assert!(outcome.applied_at_frame >= 1, "applied inside a frame");

    // The batch reached the adapter through the per-window surface, tagged
    // with the resolved default target (single-window → `main`).
    let logged = app.session().adapter().window_events();
    assert_eq!(logged.len(), 3);
    assert_eq!(logged[0].0, "main");
    assert_eq!(logged[0].1, InputEvent::PointerMove { x: 10.0, y: 20.0 });
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
fn mouse_dblclick_repeats_the_press_at_one_point() {
    let mut app = App::launch_fixture("mouse_dblclick");
    settle(&mut app);

    let outcome = app.mouse().dblclick(4.0, 8.0).expect("dblclick applies");
    assert_eq!(outcome.events, 5, "move + down/up ×2");
    assert_eq!(outcome.detail, "double-clicked at (4, 8) — 5 events");
    let logged = app.session().adapter().window_events();
    assert_eq!(logged.len(), 5);
    assert!(matches!(logged[1].1, InputEvent::PointerDown { .. }));
    assert!(matches!(logged[2].1, InputEvent::PointerUp { .. }));
    assert!(matches!(logged[3].1, InputEvent::PointerDown { .. }));
    assert!(matches!(logged[4].1, InputEvent::PointerUp { .. }));
}

#[test]
fn mouse_drag_ratchets_through_the_adapter_pacing() {
    let mut app = App::launch_fixture("mouse_drag");
    settle(&mut app);

    // The mock's default §9 ratchet is 5 px/tick: a 100 px drag is down +
    // 20 interpolated moves + up = 22 events, the last landing exactly on
    // the target so the release coordinates are honest.
    let outcome = app
        .mouse()
        .drag(10.0, 10.0, 110.0, 10.0)
        .expect("drag applies");
    assert_eq!(outcome.events, 22);
    assert_eq!(outcome.detail, "dragged (10, 10) → (110, 10) — 22 events");
    let logged = app.session().adapter().window_events();
    assert_eq!(logged.len(), 22);
    let last = &logged[logged.len() - 1].1;
    match last {
        InputEvent::PointerUp { x, y, .. } => {
            assert_eq!((*x, *y), (110.0, 10.0), "release lands on the target");
        }
        other => panic!("expected PointerUp, got {other:?}"),
    }
}

#[test]
fn mouse_wheel_and_named_buttons() {
    let mut app = App::launch_fixture("mouse_wheel_buttons");
    settle(&mut app);

    let wheel = app.mouse().wheel(0.0, -120.0).expect("wheel applies");
    assert_eq!(wheel.events, 1);
    assert_eq!(wheel.detail, "wheeled (0, -120) — 1 event");

    let right = app
        .mouse()
        .click_button(12.0, 30.0, PointerButton::Secondary)
        .expect("right click applies");
    assert_eq!(right.detail, "clicked at (12, 30) — 3 events");
    let logged = app.session().adapter().window_events();
    assert!(matches!(
        &logged[logged.len() - 1].1,
        InputEvent::PointerUp {
            button: PointerButton::Secondary,
            ..
        }
    ));
}

// ---------------------------------------------------------------------------
// Raw primitives (the `Input` half of the vocabulary)
// ---------------------------------------------------------------------------

#[test]
fn raw_mouse_primitives_move_down_up() {
    let mut app = App::launch_fixture("mouse_raw");
    settle(&mut app);

    let mut mouse = app.mouse();
    mouse.move_to(30.0, 40.0).expect("move applies");
    mouse
        .down(30.0, 40.0, PointerButton::Primary)
        .expect("down applies");
    mouse
        .up(30.0, 40.0, PointerButton::Primary)
        .expect("up applies");

    let logged = app.session().adapter().window_events();
    assert_eq!(logged.len(), 3, "one event per primitive");
    assert!(matches!(logged[0].1, InputEvent::PointerMove { .. }));
    assert!(matches!(logged[1].1, InputEvent::PointerDown { .. }));
    assert!(matches!(logged[2].1, InputEvent::PointerUp { .. }));
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

#[test]
fn keyboard_press_and_type_text() {
    let mut app = App::launch_fixture("keyboard");
    settle(&mut app);

    let press = app.keyboard().press("Enter").expect("press applies");
    assert_eq!(press.events, 2, "key down + key up");
    assert_eq!(press.detail, "pressed Enter — 2 events");

    let combo = app
        .keyboard()
        .press_with("KeyA", &["Control"])
        .expect("combo applies");
    assert_eq!(combo.detail, "pressed Control+KeyA — 2 events");
    let logged = app.session().adapter().window_events();
    assert!(matches!(
        &logged[logged.len() - 2].1,
        InputEvent::KeyDown { key, modifiers }
            if key == "KeyA" && modifiers == &["Control".to_string()]
    ));

    let typed = app.keyboard().type_text("ada").expect("text applies");
    assert_eq!(typed.events, 1, "one TextInput commit");
    assert_eq!(typed.detail, "typed 3 chars — 1 event");
    assert!(matches!(
        &app.session().adapter().window_events().last().unwrap().1,
        InputEvent::TextInput { text } if text == "ada"
    ));
}

// ---------------------------------------------------------------------------
// Window targeting (the M6 vocabulary on the embedded surface)
// ---------------------------------------------------------------------------

#[test]
fn in_window_targets_a_multi_window_session() {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "multi_window_input",
        guiwright_driver::mock::MockAdapter::with_multi_window(&fx),
    );
    settle(&mut app);
    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();

    app.mouse()
        .in_window(&w2)
        .click(50.0, 50.0)
        .expect("w2 click");
    app.keyboard()
        .in_window(&w1)
        .press("Enter")
        .expect("w1 press");

    let logged = app.session().adapter().window_events();
    let click_w: Vec<&str> = logged[..3].iter().map(|(w, _)| w.as_str()).collect();
    let key_w: Vec<&str> = logged[3..].iter().map(|(w, _)| w.as_str()).collect();
    assert!(click_w.iter().all(|w| *w == w2), "click → w2: {logged:?}");
    assert!(key_w.iter().all(|w| *w == w1), "press → w1: {logged:?}");
}

#[test]
fn unknown_window_fails_clean_without_injecting() {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "unknown_window_input",
        guiwright_driver::mock::MockAdapter::with_multi_window(&fx),
    );
    settle(&mut app);

    let err = app
        .mouse()
        .in_window("a6000000-dead-beef-0000-000000000000")
        .click(10.0, 10.0)
        .expect_err("dead handle rejected");
    // The tree is the scoping source of truth (§16 v0.4): the error names
    // the handle and nothing was injected.
    assert!(
        err.to_string().contains("no window")
            && err
                .to_string()
                .contains("a6000000-dead-beef-0000-000000000000"),
        "{err}"
    );
    assert!(
        app.session().adapter().window_events().is_empty(),
        "nothing injected for a dead target"
    );
}

// ---------------------------------------------------------------------------
// Observability: input events land in the trace
// ---------------------------------------------------------------------------

#[test]
fn mouse_and_keyboard_events_land_in_the_trace() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut app = App::launch_fixture("traced_input");
    app.enable_trace(tmp.path().join("trace.zip"));
    app.mouse().click(10.0, 20.0).expect("click");
    app.keyboard().press("Enter").expect("press");
    let path = app.finish().expect("trace path");
    assert!(path.exists(), "{path:?}");

    let bytes = std::fs::read(&path).expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .expect("events")
        .read_to_string(&mut events)
        .expect("read");
    assert!(events.contains("\"mouse\""), "mouse recorded: {events}");
    assert!(
        events.contains("clicked at (10, 20)"),
        "click detail: {events}"
    );
    assert!(
        events.contains("\"keyboard\""),
        "keyboard recorded: {events}"
    );
    assert!(events.contains("pressed Enter"), "press detail: {events}");
}

// ---------------------------------------------------------------------------
// Ergonomics: sequential commands without borrow fights
// ---------------------------------------------------------------------------

#[test]
fn handles_reborrow_between_commands() {
    let mut app = App::launch_fixture("reborrow");
    settle(&mut app);

    // Each command borrows the app mutably only for its duration — a
    // sequence of calls must compile and apply in order.
    app.mouse().click(10.0, 10.0).expect("1");
    app.mouse().click(20.0, 20.0).expect("2");
    app.keyboard().type_text("ada").expect("3");
    app.mouse().wheel(0.0, 120.0).expect("4");
    assert_eq!(app.session().adapter().window_events().len(), 3 + 3 + 1 + 1);
}

// ---------------------------------------------------------------------------
// Codegen (§13 over §9): the input surface records as kit code while a
// recording is active — same vocabulary the remote recorder emits.
// ---------------------------------------------------------------------------

#[test]
fn codegen_records_the_input_surface() {
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "codegen_input",
        guiwright_driver::mock::MockAdapter::with_multi_window(&fx),
    );
    settle(&mut app);
    let w2 = fx.w2.0.to_string();

    app.enable_codegen();
    app.mouse().click(10.0, 20.0).expect("click");
    app.mouse()
        .in_window(&w2)
        .dblclick(5.0, 6.0)
        .expect("dblclick");
    app.keyboard().press("Enter").expect("press");
    app.keyboard()
        .press_with("KeyA", &["Control"])
        .expect("chord");
    app.keyboard().type_text("ada").expect("type");
    app.mouse().drag(1.0, 2.0, 3.0, 4.0).expect("drag");
    app.mouse().wheel(0.0, -120.0).expect("wheel");
    app.mouse().move_to(30.0, 40.0).expect("move");

    let (code, warnings) = app.codegen_code().expect("recorder active");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(code.contains("app.mouse().click(10.0, 20.0);"), "{code}");
    assert!(
        code.contains(&format!(
            "app.mouse().in_window({w2:?}).dblclick(5.0, 6.0);"
        )),
        "{code}"
    );
    assert!(code.contains("app.keyboard().press(\"Enter\");"), "{code}");
    assert!(
        code.contains("app.keyboard().press_with(\"KeyA\", [\"Control\"]);"),
        "{code}"
    );
    assert!(
        code.contains("app.keyboard().type_text(\"ada\");"),
        "{code}"
    );
    assert!(
        code.contains("app.mouse().drag(1.0, 2.0, 3.0, 4.0);"),
        "{code}"
    );
    assert!(code.contains("app.mouse().wheel(0.0, -120.0);"), "{code}");
    assert!(code.contains("app.mouse().move_to(30.0, 40.0);"), "{code}");
}

#[test]
fn codegen_inactive_input_records_nothing() {
    let mut app = App::launch_fixture("codegen_off_input");
    settle(&mut app);
    app.mouse().click(10.0, 20.0).expect("click");
    assert!(
        app.codegen_code().is_none(),
        "no recorder — no code surfaced"
    );
}
