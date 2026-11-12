//! The locator action parity surface (§16 actions row, M19): the wire's
//! semantic vocabulary (`Press` / `Check` / `Uncheck` / `Focus` / `Blur`)
//! on the kit's locator surface — the same actionability gate, auto-wait
//! and codegen recording every other locator action takes.
//!
//! ```text
//! cargo run -p guiwright-kit --example locator_actions
//! ```
//!
//! Prints the detail of every semantic action on the fixture app and the
//! codegen lines the run records (`locator.press("Enter");` …), then a
//! `gwtrace/v1` archive of the session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The fixture app: a "Save" button, a "Welcome, Ada" label and an
    // "Email" text input. Any real adapter drops in unchanged — the
    // surface is the kit's, the gate is the session's.
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("locator_actions_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("locator_actions_demo.zip"));
    app.enable_codegen();
    app.wait_for_settle();

    // Semantic actions through the locator (§8.4 vocabulary) — the
    // locator is a consuming builder, one action per handle:
    println!(
        "press        → {}",
        app.get_by_role(Role::Button, "Save")
            .press("Enter")
            .expect("press")
            .detail
    );
    println!(
        "focus        → {}",
        app.get_by_role(Role::Button, "Save")
            .focus()
            .expect("focus")
            .detail
    );
    println!(
        "blur         → {}",
        app.get_by_role(Role::Button, "Save")
            .blur()
            .expect("blur")
            .detail
    );
    println!(
        "set_checked  → {}",
        app.get_by_role(Role::Button, "Save")
            .set_checked(true)
            .expect("set_checked(true)")
            .detail
    );
    println!(
        "check        → {}",
        app.get_by_role(Role::Button, "Save")
            .check()
            .expect("check")
            .detail
    );
    println!(
        "uncheck      → {}",
        app.get_by_role(Role::Button, "Save")
            .uncheck()
            .expect("uncheck")
            .detail
    );

    // Codegen (§13): each action resolved its node facts and recorded the
    // Playwright-shaped call the kit API speaks.
    let (code, warnings) = app.codegen_code().expect("codegen recorded");
    println!("\ncodegen ({} warning(s)):", warnings.len());
    for line in code.lines() {
        println!("  {line}");
    }

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
