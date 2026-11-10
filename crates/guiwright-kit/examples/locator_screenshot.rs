//! Element screenshots through the locator surface (§10, M9):
//! `locator.screenshot(name, bless)` — resolve (auto-wait), render the
//! window frame once, crop it by the resolved node's physical bounds
//! (never a per-element re-render), and assert against the same
//! per-platform baselines `app.screenshot` uses.
//!
//! ```text
//! cargo run -p guiwright-kit --example locator_screenshot
//! ```
//!
//! Bootstraps a baseline for the fixture's Save button, asserts it, then
//! shows the deterministic mismatch flow (artifacts land next to the
//! baseline) and records a `gwtrace/v1` archive of the session.

use accesskit::Role;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, BlessMode, SnapshotOutcome};

fn main() {
    // Deterministic gradient frame (480×240): pixel (x, y) = [x, y, 99].
    // Any real adapter with a pixel path (masonry's vello_cpu, egui's
    // capture pipeline) drops in unchanged.
    let mut rgba = Vec::with_capacity(480 * 240 * 4);
    for y in 0..240u32 {
        for x in 0..480u32 {
            rgba.extend_from_slice(&[(x & 0xff) as u8, (y & 0xff) as u8, 99, 255]);
        }
    }
    let dir = std::env::temp_dir().join(format!("locator_screenshot_demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let adapter = adapter.with_frame_pixels(480, 240, rgba);
    let mut app = App::launch("locator_screenshot_demo", adapter).with_baseline_dir(&dir);
    app.enable_trace(std::env::temp_dir().join("locator_screenshot_demo.zip"));
    app.wait_for_settle();

    // First run: write the baseline (GUIWRIGHT_BLESS=1 semantics).
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline bootstrapped");
    println!("first run  → {outcome:?} (the crop is the resolved rect, 120×32)");

    // Second run: matched within the §10 budget.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Off)
        .expect("deterministic frame");
    println!("second run → {outcome:?}");

    // Email input: its own baseline (same flow, different element).
    let outcome = app
        .get_by_role(Role::TextInput, "Email")
        .screenshot("email", BlessMode::Failing)
        .expect("baseline bootstrapped");
    println!("email      → {outcome:?}");

    assert!(matches!(outcome, SnapshotOutcome::Blessed { .. }));
    let trace = app.finish();
    println!(
        "baselines in {}; trace: {}",
        dir.display(),
        trace.map_or_else(|| "none".into(), |p| p.display().to_string())
    );
}
