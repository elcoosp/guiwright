//! The m78 consumer walkthrough (§17 row 5): drive the real
//! `counter_form.slint` app — stock `Button` + `LineEdit` + `Text` —
//! through the standard [`Session`], printing every step's observed tree
//! change. The embedded mode's remote-semantics contract (§5.1) rides the
//! same adapter a remote client would see.
//!
//! Run:
//!
//! ```ignore
//! cargo run --manifest-path glue/slint/Cargo.toml --example counter_form
//! ```
//!
//! [`Session`]: guiwright_driver::session::Session

use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_adapter_slint_glue::harness::SlintAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::session::Session;
use slint::include_modules;

include_modules!();

fn main() {
    let adapter =
        SlintAdapter::boot(CounterForm::new, (480.0, 320.0)).expect("slint harness boots");
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

    session.fill(&name_input, "Ada").expect("fill the name");
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("greeting follows the typed name");
    println!("fill     LineEdit ← \"Ada\" → tree reads \"Welcome, Ada\"");

    // The §14 clock: the testing backend's mock time is first-class.
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

    // The §10 pixel row: the software rasterizer renders headlessly.
    let frame = session
        .adapter_mut()
        .render_frame(Some((480, 320)))
        .expect("headless render");
    println!(
        "render   480×320 → {} bytes of RGBA pixels (frame hash {:016x})",
        frame.pixels.as_ref().map_or(0, |px| px.len()),
        frame.stamp.hash
    );

    println!("done — every step ran through the real TestingBackend pipeline");
}
