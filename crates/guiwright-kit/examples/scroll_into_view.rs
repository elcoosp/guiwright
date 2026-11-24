//! The §8.4 scroll-into-view vocabulary on the kit's locator surface
//! (m90 — the m19 record's named §16 follow-on):
//! `locator.scroll_into_view_if_needed()` rides the same actionability
//! gate, auto-wait and codegen recording every other locator action
//! takes, and the request travels the wire as the appended
//! `SemActionV1::ScrollIntoView` variant — the a11y-first honest form
//! of Playwright's `locator.scrollIntoViewIfNeeded()`.
//!
//! ```text
//! cargo run -p guiwright-kit --example scroll_into_view
//! ```
//!
//! Scrolls the fixture's "Save" button into view, prints the adapter's
//! detail and the recorded codegen line, then archives the `gwtrace/v1`
//! session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The fixture app. Any real adapter drops in unchanged — the
    // surface is the kit's, the gate is the session's, and the a11y
    // stacks translate the request into their own scroll semantics.
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("scroll_into_view_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("scroll_into_view_demo.zip"));
    app.enable_codegen();
    app.wait_for_settle();

    let detail = app
        .get_by_role(Role::Button, "Save")
        .scroll_into_view_if_needed()
        .expect("scroll_into_view_if_needed")
        .detail;
    println!("scroll_into_view_if_needed → {detail}");

    // Codegen (§13): the recorded program names the Playwright shape.
    let (code, warnings) = app.codegen_code().expect("codegen recorded");
    println!("\ncodegen ({} warning(s)):", warnings.len());
    for line in code.lines() {
        println!("  {line}");
    }

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
