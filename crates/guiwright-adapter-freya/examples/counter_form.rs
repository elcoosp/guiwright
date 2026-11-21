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

use freya_components::input::Input;
use freya_testing::prelude::*;
use guiwright_adapter_freya::runner::{FreyaAdapter, FreyaTestingHarness};
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::session::Session;
// freya 0.5: element sizes ride torin's `Size` (`Size::px` — floats no
// longer convert implicitly).
use torin::prelude::Size;

#[derive(Clone)]
struct AppState {
    count: State<i64>,
    name: State<String>,
    /// The mount gate (§9 quirk): the harness flips this after freya's
    /// internal boot commit, so the first post-boot diff is the full tree.
    mounted: State<bool>,
}

fn counter_form() -> Element {
    let state = use_consume::<AppState>();
    if !*state.mounted.read() {
        return rect()
            .width(Size::px(480.))
            .height(Size::px(320.))
            .a11y_id(AccessibilityId(0))
            .into();
    }
    let mut count = state.count;
    let name = state.name;
    let name_input = name;

    rect()
        .width(Size::px(480.))
        .height(Size::px(320.))
        .a11y_id(AccessibilityId(0))
        .child(
            // freya 0.5 aggregates the first value-carrying child's text
            // into the container's a11y name — this marker keeps the root
            // from colliding with the dynamic count/greeting texts (§9).
            label().text("Guiwright demo").a11y_id(AccessibilityId(100)),
        )
        .child(
            label()
                .text(format!("Count: {}", count.read()))
                .a11y_id(AccessibilityId(101)),
        )
        .child(
            rect()
                .width(Size::px(140.))
                .height(Size::px(40.))
                .background(Color::from_rgb(200, 220, 255))
                .child("Increment")
                .on_press(move |_| *count.write() += 1)
                .a11y_role(AccessibilityRole::Button)
                .a11y_id(AccessibilityId(102)),
        )
        .child(
            rect()
                .width(Size::px(140.))
                .height(Size::px(40.))
                .background(Color::from_rgb(255, 220, 200))
                .child("Decrement")
                .on_press(move |_| *count.write() -= 1)
                .a11y_role(AccessibilityRole::Button)
                .a11y_id(AccessibilityId(103)),
        )
        .child(
            Input::new(name_input)
                .placeholder("Type a name")
                .a11y_id(AccessibilityId(104)),
        )
        .child(
            label()
                .text(if name.read().is_empty() {
                    "Welcome, nobody".to_string()
                } else {
                    format!("Welcome, {}", name.read())
                })
                .a11y_id(AccessibilityId(105)),
        )
        .into()
}

fn main() {
    let harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
        // The context the app consumes IS the AppState (freya 0.5
        // registers contexts by type — `use_consume::<AppState>()`).
        runner.provide_root_context(|| AppState {
            count: State::create(0i64),
            name: State::create(String::new()),
            mounted: State::create(false),
        })
    });
    // The mount flip lands after freya's internal boot commit and before
    // the session's first sync — the next diff IS the full tree (§9).
    let mut mounted = harness.state().mounted;
    *mounted.write() = true;
    let mut session = Session::new(FreyaAdapter::new(harness));
    session.settle(Instant::now() + Duration::from_secs(2));
    // freya is Level-1: `.a11y_id(n)` numbers ARE the AccessKit NodeIds —
    // the §8.1 test-id registry rides an explicit registration.
    for n in [100u64, 101, 102, 103, 104, 105] {
        session.register_test_id(
            guiwright_core::NodeKey::AccessKit {
                tree: accesskit::TreeId::ROOT,
                node: accesskit::NodeId(n),
            },
            n.to_string(),
        );
    }

    let increment = Selector::test_id("102");
    let name_field = Selector::test_id("104");

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
