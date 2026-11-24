//! The §8.4 select vocabulary on the kit's locator surface (m89 — the
//! m19 record's named §16 follow-on): `locator.select_option(option)`
//! rides the same actionability gate, auto-wait and codegen recording
//! every other locator action takes, and the option travels the wire as
//! the appended `SemActionV1::SelectOption` variant.
//!
//! ```text
//! cargo run -p guiwright-kit --example select_option
//! ```
//!
//! Selects an option on the fixture's "Country" combobox, prints the
//! adapter's detail and the recorded codegen line, then archives the
//! `gwtrace/v1` session.

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The fixture app with the select extension band: a "Country"
    // combobox beside the M0 nodes. Any real adapter drops in unchanged —
    // the surface is the kit's, the gate is the session's.
    let fixture = guiwright_core::test_util::Fixture::with_combobox();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("select_option_demo", adapter);
    app.enable_trace(std::env::temp_dir().join("select_option_demo.zip"));
    app.enable_codegen();
    app.wait_for_settle();

    // The option rides by accessible name — the same string a text
    // locator matches (§16 actions row's `select`).
    let detail = app
        .get_by_role(Role::ComboBox, "Country")
        .select_option("France")
        .expect("select_option")
        .detail;
    println!("select_option → {detail}");

    // Codegen (§13): the recorded program carries the option argument,
    // the Playwright `selectOption` shape in kit vocabulary.
    let (code, warnings) = app.codegen_code().expect("codegen recorded");
    println!("\ncodegen ({} warning(s)):", warnings.len());
    for line in code.lines() {
        println!("  {line}");
    }

    if let Some(path) = app.finish() {
        println!("\ntrace: {}", path.display());
    }
}
