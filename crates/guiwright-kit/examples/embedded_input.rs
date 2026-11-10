//! The embedded input surface (§9 over the kit): `app.mouse()` /
//! `app.keyboard()` on the fixture app — the same Playwright-shaped
//! vocabulary the REPL and the TypeScript client speak over the wire,
//! driven in-process.
//!
//! ```text
//! cargo run -p guiwright-kit --example embedded_input
//! ```
//!
//! Prints the synthesis detail of every gesture and the raw event log the
//! adapter received, then records a `gwtrace/v1` archive of the session.

use std::time::Duration;

use guiwright_driver::PointerButton;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The M1 fixture app: a "Save" button, a "Welcome, Ada" label and an
    // "Email" text input. Any real adapter drops in unchanged — the
    // surface is the kit's, the synthesis is the session's.
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("embedded_input_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("embedded_input_demo.zip"));
    app.wait_for_settle();

    // Gestures (§9 synthesis): the adapter's pacing table drives the
    // synthesized batches; one rendezvous pump applies each.
    let click = app.mouse().click(10.0, 20.0).expect("click");
    println!("mouse.click  → {}", click.detail);
    let dbl = app.mouse().dblclick(4.0, 8.0).expect("dblclick");
    println!("mouse.dbl    → {}", dbl.detail);
    let drag = app.mouse().drag(10.0, 10.0, 110.0, 10.0).expect("drag");
    println!("mouse.drag   → {}", drag.detail);
    let wheel = app.mouse().wheel(0.0, -120.0).expect("wheel");
    println!("mouse.wheel  → {}", wheel.detail);

    // Keyboard: key down → key up; text commits ride `TextInput` (§4.2).
    let press = app.keyboard().press("Enter").expect("press");
    println!("key.press    → {}", press.detail);
    let typed = app.keyboard().type_text("ada@lovelace.dev").expect("type");
    println!("key.type     → {}", typed.detail);

    // A right-click through the named-button variant.
    let right = app
        .mouse()
        .click_button(12.5, 30.0, PointerButton::Secondary)
        .expect("right click");
    println!("mouse.right  → {}", right.detail);

    // The raw primitives (the `Input` half of the vocabulary): move →
    // down → up, one event per command.
    let mut mouse = app.mouse();
    mouse.move_to(30.0, 40.0).expect("move");
    mouse
        .down(30.0, 40.0, PointerButton::Primary)
        .expect("down");
    mouse.up(30.0, 40.0, PointerButton::Primary).expect("up");
    drop(mouse);
    println!("mouse.raw    → move → down → up applied");

    // Everything reached the adapter through the per-window pipeline
    // (§9): the single-window fixture reports one `main` target.
    let logged = app.session().adapter().window_events();
    println!(
        "\nadapter input log ({} events, target `main`):",
        logged.len()
    );
    for (window, ev) in logged {
        println!("  [{window}] {ev:?}");
    }

    // The same commands landed in the trace's event log — the embedded
    // surface is observable exactly like the wire one.
    app.wait_for_settle();
    std::thread::sleep(Duration::from_millis(50));
    let path = app.finish();
    match path {
        Some(path) => println!("\ntrace: {}", path.display()),
        None => println!("\ntrace: not recorded"),
    }
}
