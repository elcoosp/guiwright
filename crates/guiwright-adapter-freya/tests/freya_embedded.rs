//! Embedded freya adapter integration tests (architecture §4.3, §17 row 3,
//! m79).
//!
//! A real `freya_testing::TestingRunner` running a real freya app —
//! counter + form with stock `rect` press targets, `Label` texts and the
//! freya `Input` editor, all carrying explicit
//! `.a11y_id(AccessibilityId(n))` selectors (§4.3: "stable selectors
//! require apps to set `.a11y_id`") — driven through the standard
//! [`guiwright_driver::Session`]. The committed AccessKit `TreeUpdate`s
//! are the Level-1 diffs; input rides the frame-quantized
//! press→sync→release pipeline (`send_event` + `sync_and_update`).
//!
//! The freya stack (software Skia) compiles only where a C++ toolchain
//! exists — locally the default build stays framework-free; this leg is
//! runtime-tested by the CI `adapter-freya` job (m79 upgraded it from
//! `cargo check`).

#![cfg(feature = "freya")]

use std::time::{Duration, Instant};

use freya_components::Input;
use freya_testing::prelude::*;
use guiwright_adapter_freya::runner::{FreyaAdapter, FreyaTestingHarness};
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::InputEvent;
use guiwright_driver::session::Session;

/// The app state the hook provides and the test observes.
struct AppState {
    count: State<i64>,
    name: State<String>,
}

/// The §4.3 app contract: explicit `.a11y_id`s (stable selectors),
/// `a11y_role` on the press targets, texts as `Label`s.
fn counter_form() -> impl IntoElement {
    let state = use_consume::<AppState>();
    let count = state.count.clone();
    let name = state.name.clone();
    let name_input = name.clone();

    rect()
        .width(480.)
        .height(320.)
        .child(
            Label::new()
                .text(format!("Count: {}", count.read()))
                .a11y_id(AccessibilityId(1)),
        )
        .child(
            rect()
                .width(140.)
                .height(40.)
                .background(Color::from_rgb(200, 220, 255))
                .child("Increment")
                .on_press(move |_| *count.write() += 1)
                .a11y_role(AccessibilityRole::Button)
                .a11y_id(AccessibilityId(2)),
        )
        .child(
            rect()
                .width(140.)
                .height(40.)
                .background(Color::from_rgb(255, 220, 200))
                .child("Decrement")
                .on_press(move |_| *count.write() -= 1)
                .a11y_role(AccessibilityRole::Button)
                .a11y_id(AccessibilityId(3)),
        )
        .child(
            Input::new(name_input)
                .placeholder("Type a name")
                .a11y_id(AccessibilityId(4)),
        )
        .child(
            Label::new()
                .text(if name.read().is_empty() {
                    "Welcome, nobody".to_string()
                } else {
                    format!("Welcome, {}", name.read())
                })
                .a11y_id(AccessibilityId(5)),
        )
}

fn counter_session() -> Session<FreyaAdapter<FreyaTestingHarness<AppState>>> {
    let harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
        let count = runner.provide_root_context(|| State::create(0i64));
        let name = runner.provide_root_context(|| State::create(String::new()));
        AppState { count, name }
    });
    let adapter = FreyaAdapter::new(harness);
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    session
}

fn increment() -> Selector {
    Selector::test_id("2")
}

fn name_field() -> Selector {
    Selector::test_id("4")
}

#[test]
fn tree_update_commits_the_a11y_ids() {
    let mut session = counter_session();
    // The committed diff carries the app-assigned ids (§4.3 a11y_id
    // contract) with laid-out bounds.
    session.resolve(&increment()).expect("increment button");
    session.resolve(&name_field()).expect("name input");
    session
        .resolve(&Selector::text("Count: 0"))
        .expect("counter label reflects boot state");
}

#[test]
fn semantic_click_drives_the_real_freya_counter() {
    let mut session = counter_session();

    session.click(&increment()).expect("click Increment");

    // The on_press handler ran through the real ragnarok pipeline.
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");
}

#[test]
fn fill_types_through_the_real_input() {
    let mut session = counter_session();

    session
        .fill(&name_field(), "Ada")
        .expect("fill the name input");

    // The two-way state binding updated the greeting label.
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("greeting reflects the typed name");
}

#[test]
fn raw_input_moves_the_pointer_and_clicks() {
    let mut session = counter_session();

    // The §9 raw-injection path: the InputEvent → PlatformEvent
    // translation rides the same frame-quantized pipeline.
    session
        .adapter_mut()
        .input(InputEvent::PointerMove { x: 90.0, y: 80.0 })
        .expect("move");
    session
        .adapter_mut()
        .input(InputEvent::PointerDown { x: 90.0, y: 80.0 })
        .expect("down");
    session
        .adapter_mut()
        .input(InputEvent::PointerUp { x: 90.0, y: 80.0 })
        .expect("up");
    session.settle(Instant::now() + Duration::from_secs(1));

    session
        .resolve(&Selector::text("Count: 1"))
        .expect("raw pointer pipeline incremented the counter");
}

#[test]
fn render_frame_produces_real_pixels() {
    let mut session = counter_session();

    let frame = session.adapter_mut().render_frame(None).expect("render");
    assert_eq!(frame.size, (480, 320), "the boot viewport");
    let pixels = frame.pixels.expect("the skia rasterizer produced pixels");
    assert!(!pixels.is_empty());
    assert!(
        pixels.chunks_exact(4).any(|px| px[3] != 0),
        "the snapshot has painted pixels"
    );
}

#[test]
fn key_events_carry_the_full_sequence() {
    let mut session = counter_session();

    // §4.3 gap: modifiers AND KeyUp ride the raw Keyboard pipeline.
    session
        .adapter_mut()
        .input(InputEvent::KeyDown {
            key: "Escape".into(),
            modifiers: vec!["Shift".into()],
        })
        .expect("key down with modifier");
    session
        .adapter_mut()
        .input(InputEvent::KeyUp {
            key: "Escape".into(),
            modifiers: vec!["Shift".into()],
        })
        .expect("key up with modifier");
}
