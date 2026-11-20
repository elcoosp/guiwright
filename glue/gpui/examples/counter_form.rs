//! The m80 consumer walkthrough: drive the real gpui counter+form app
//! through the standard [`Session`], printing every step's observed
//! tree change.
//!
//! Run: `cargo run --manifest-path glue/gpui/Cargo.toml --example counter_form`
//!
//! [`Session`]: guiwright_driver::session::Session

use std::time::{Duration, Instant};

use accesskit::Role;

use guiwright_adapter_gpui_glue::harness::GpuiAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::session::Session;

fn main() {
    let adapter = GpuiAdapter::boot((480.0, 320.0), 0).expect("gpui harness boots");
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));

    let increment = Selector::role_named(Role::Button, "Increment");
    let decrement = Selector::role_named(Role::Button, "Decrement");

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
    println!("click    Increment -> tree reads \"Count: 1\"");

    session.click(&decrement).expect("click Decrement");
    session.click(&decrement).expect("click Decrement");
    session
        .resolve(&Selector::text("Count: -1"))
        .expect("counter decremented twice");
    println!("click    Decrement x2 -> tree reads \"Count: -1\"");

    // The one documented upstream blocker (m80): `fill` gates on zed's
    // test-scheduler session-id overflow on the real KeyDown path
    // (scheduler/src/test_scheduler.rs:232); the runtime test drives
    // the adapter's own gate and asserts the evidence.
    println!("fill     TextInput — documented upstream blocker (see tests/gpui_embedded.rs)");

    let state = session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::FastForward(
            Duration::from_millis(1_000),
        ))
        .expect("advance virtual time");
    println!(
        "clock    fastForward(1s) -> virtual_time={:?}",
        state.virtual_time
    );

    let frame = session.adapter_mut().last_frame().expect("scene frame");
    println!(
        "scene    digest {:016x} (frame {})",
        frame.stamp.hash, frame.stamp.seq
    );

    println!("done — every step ran through the real gpui test-support pipeline");
}
