//! The m80 consumer walkthrough (§17 row 6): drive the real floem
//! counter+form app — stock `button` + `text_input` + derived labels —
//! through the standard [`Session`], printing every step's observed tree
//! change. The embedded mode's remote-semantics contract (§5.1) rides
//! the same adapter a remote client would see.
//!
//! Run:
//!
//! ```ignore
//! cargo run --manifest-path glue/floem/Cargo.toml --example counter_form
//! ```
//!
//! [`Session`]: guiwright_driver::session::Session

use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::Role;
use floem::view::IntoView;
use floem::views::{Button, Decorators, Label, Stack, TextInput};
use floem_reactive::{RwSignal, SignalGet, SignalUpdate};

use guiwright_adapter_floem_glue::harness::{FloemAdapter, SemReader};
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::session::Session;

struct CounterForm {
    count: RwSignal<i32>,
    name: RwSignal<String>,
}

impl CounterForm {
    fn new() -> Self {
        Self {
            count: RwSignal::new(0),
            name: RwSignal::new(String::new()),
        }
    }
}

fn counter_view(app: &Rc<CounterForm>) -> impl IntoView {
    let count = app.count;
    let name = app.name;
    let name_input = app.name;
    Stack::new((
        Button::new("Increment")
            .action(move || count.set(count.get_untracked() + 1))
            .debug_name("increment"),
        Button::new("Decrement")
            .action(move || count.set(count.get_untracked() - 1))
            .debug_name("decrement"),
        TextInput::new(name_input).debug_name("name-input"),
        Label::derived(move || format!("Count: {}", count.get())).debug_name("count-label"),
        Label::derived(move || format!("Welcome, {}", name.get())).debug_name("greeting"),
    ))
    .style(|s| s.flex_col().items_center())
}

fn counter_readers(app: &Rc<CounterForm>) -> Vec<SemReader> {
    let count = app.count;
    let name = app.name;
    let name_value = app.name;
    vec![
        SemReader::text("increment", move || "Increment".to_string()),
        SemReader::text("decrement", move || "Decrement".to_string()),
        SemReader::text("count-label", move || format!("Count: {}", count.get())),
        SemReader::text("greeting", move || format!("Welcome, {}", name.get())),
        SemReader::value("name-input", move || name_value.get_untracked()),
    ]
}

fn main() {
    let app = Rc::new(CounterForm::new());
    let adapter = FloemAdapter::boot(
        (480.0, 320.0),
        {
            let app = Rc::clone(&app);
            move || counter_view(&app).into_any()
        },
        counter_readers(&app),
    )
    .expect("floem harness boots");
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));

    let increment = Selector::role_named(Role::Button, "Increment");
    let decrement = Selector::role_named(Role::Button, "Decrement");
    let name_input = Selector::role(Role::TextInput);

    let resolved = session
        .resolve(&increment)
        .expect("Increment button in the logical tree");
    println!(
        "resolve  role=Button name=\"Increment\" bounds={:?}",
        resolved.bounds
    );

    session.click(&increment).expect("click Increment");
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");
    println!("click    Increment → tree reads \"Count: 1\"");

    session.click(&decrement).expect("click Decrement");
    session.click(&decrement).expect("click Decrement");
    session
        .resolve(&Selector::text("Count: -1"))
        .expect("counter decremented twice");
    println!("click    Decrement ×2 → tree reads \"Count: -1\"");

    match session.fill(&name_input, "Ada") {
        Ok(outcome) => println!("fill     TextInput ← \"Ada\" → {outcome:?}"),
        Err(err) => {
            // The one documented upstream blocker (m80): floem's
            // text_input requests IME on focus
            // (`src/views/text_input.rs:1403`) and the headless
            // MockWindow rejects every IME request
            // (`src/window/mock.rs:262`), unwrapped into a panic at
            // `src/window/handle.rs:1270`. The adapter reports the
            // capability row honestly; the tree/input contract stays
            // real around it.
            println!("fill     TextInput ← \"Ada\" → documented upstream blocker: {err}");
        }
    }
    let input_still_real = session.resolve(&Selector::role(Role::TextInput)).is_ok();
    println!("resolve  TextInput still real after the blocked fill: {input_still_real}");

    // The §14 clock: harness update-queue ticks with virtual time.
    let state = session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::FastForward(
            Duration::from_millis(1_000),
        ))
        .expect("advance virtual time");
    println!(
        "clock    fastForward(1s) → virtual_time={:?}",
        state.virtual_time
    );

    // The §10 pixel row: the engine's tiny-skia PaintState rasterizes
    // the real laid-out view geometry headlessly (§4.7 contribution 2).
    let frame = session
        .adapter_mut()
        .render_frame(Some((480, 320)))
        .expect("headless render");
    println!(
        "render   480×320 → {} bytes of RGBA pixels (frame hash {:016x})",
        frame.pixels.as_ref().map_or(0, |px| px.len()),
        frame.stamp.hash
    );

    println!("done — every step ran through the real floem HeadlessHarness pipeline");
}
