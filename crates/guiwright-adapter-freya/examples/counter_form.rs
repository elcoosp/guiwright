//! The m79 consumer walkthrough (§17 row 3): drive the real freya
//! counter+form app — stock `rect` press targets, `Label` texts and the
//! freya `Input` editor with explicit `.a11y_id` selectors — through the
//! standard [`Session`], printing every step's observed tree change.
//!
//! Run (the freya stack compiles where a C++ toolchain exists — CI):
//!
//! ```ignore
//! cargo run -p guiwright-adapter-freya --features freya --example counter_form
//! ```

use std::time::{Duration, Instant};

use freya_components::Input;
use freya_testing::prelude::*;
use guiwright_adapter_freya::runner::{FreyaAdapter, FreyaTestingHarness};
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::session::Session;

struct AppState {
    count: State<i64>,
    name: State<String>,
}

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

fn main() {
    let harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
        let count = runner.provide_root_context(|| State::create(0i64));
        let name = runner.provide_root_context(|| State::create(String::new()));
        AppState { count, name }
    });
    let mut session = Session::new(FreyaAdapter::new(harness));
    session.settle(Instant::now() + Duration::from_secs(2));

    let increment = Selector::test_id("2");
    let name_field = Selector::test_id("4");

    let resolved = session.resolve(&increment).expect("increment button");
    println!(
        "resolve  a11y_id=2 role=Button bounds={:?}",
        resolved.bounds
    );

    session.click(&increment).expect("click Increment");
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");
    println!("click    a11y_id=2 → tree reads \"Count: 1\"");

    session.fill(&name_field, "Ada").expect("fill the name");
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("greeting follows the typed name");
    println!("fill     Input ← \"Ada\" → tree reads \"Welcome, Ada\"");

    let frame = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render");
    println!(
        "render   480×320 → {} bytes of RGBA pixels (frame hash {:016x})",
        frame.pixels.as_ref().map_or(0, |px| px.len()),
        frame.stamp.hash
    );

    println!("done — every step ran through the real TestingRunner pipeline");
}
