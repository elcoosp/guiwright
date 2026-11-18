//! Deterministic time (§14 — the virtual clock): the README's
//! `app.clock_fast_forward` one-liner and the full `app.clock()`
//! vocabulary — install a virtual clock, fast-forward in frame-sized
//! steps (timers and animations settle, no sleeps), pause / resume /
//! run-for / set-time, and probe the state. Time-dependent UI becomes
//! reproducible: the same 30-second jump lands on the same frame every
//! run, in CI as on a laptop.
//!
//! ```text
//! cargo run -p guiwright-kit --example virtual_clock
//! ```
//!
//! The mock adapter answers clock commands directly (§6.2's clock row);
//! real adapters translate them to their framework's primitive — egui's
//! `RawInput.time`, slint's `mock_elapsed_time`, masonry's `AnimFrame`
//! ticks, freya's ticker (§14's per-adapter table).

use std::time::Duration;

use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("virtual_clock_demo", adapter);
    app.wait_for_settle();

    // The full vocabulary on the handle: install resets to zero, then
    // the README's 30-second jump in one call — frame-sized steps at the
    // 60 Hz pace, the step counter capped so a jump can never run away
    // (the ledger's anti-runaway guard; the virtual time still lands on
    // the full 30 s).
    let mut clock = app.clock();
    let report = clock.install_virtual().expect("install");
    println!(
        "install_virtual          → virtual = {:?} (a fresh ledger)",
        report.state.virtual_time
    );
    let report = clock
        .fast_forward(Duration::from_secs(30))
        .expect("the virtual jump");
    println!(
        "fast_forward(30s)        → virtual = {:?}, steps = {}",
        report.state.virtual_time, report.steps
    );

    // Fast-forward settles the frame queue: 1 s at 60 Hz = 60 steps.
    let report = clock.fast_forward(Duration::from_secs(1)).expect("ff");
    println!(
        "fast_forward(1s)         → virtual = {:?}, steps = {}",
        report.state.virtual_time, report.steps
    );

    // Pausing freezes the wall; virtual jumps still land (§14's
    // pause/resume semantics — run_for is the real-time pace and is
    // refused while paused).
    clock.pause().expect("pause");
    clock
        .fast_forward(Duration::from_millis(20))
        .expect("virtual jumps land while paused");
    println!("pause + fast_forward(20ms) → frozen wall, virtual jumps on");
    clock.resume().expect("resume");
    clock.run_for(Duration::from_millis(10)).expect("run_for");

    // Set-time pins an absolute instant; the probe reads the ledger back.
    clock.set_time(Duration::from_secs(120)).expect("set time");
    let state = clock.state().expect("probe");
    println!(
        "set_time(120s)           → virtual = {:?}, paused = {}",
        state.virtual_time, state.paused
    );
}
