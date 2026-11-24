//! The offset drags (§16 actions row's drag — m33's honest-list
//! widening): `source.drag_to_with_positions(target, source_position,
//! target_position)` — the Playwright
//! `locator.dragTo(target, { sourcePosition, targetPosition })` shape on
//! the kit's borrow-and-act doctrine. A position is a `(dx, dy)` offset
//! from the resolved node's bounds top-left (the Playwright
//! padding-box-corner shape): `Some` aims that exact point, `None` keeps
//! the M33 center. Playwright validates nothing and neither does this —
//! the pointer lands exactly where the suite aimed; the same auto-waits,
//! the same one-gesture one-window discipline, the same §9 ratchet run
//! between the aimed points, and the codegen line carries the aimed
//! coordinates.
//!
//! ```text
//! cargo run -p guiwright-kit --example drag_to_offsets
//! ```
//!
//! Drags Save → Cancel three ways on the button fixture — both centers
//! (the `None` contract), both offsets, a one-sided offset — and prints
//! the recorded kit code for each.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // Save: 16,16 → 136,48 (top-left 16,16); Cancel: 16,56 → 136,88
    // (top-left 16,56). Any real adapter exposing AccessKit bounds drops
    // in unchanged.
    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("drag_to_offsets_demo", adapter);
    app.wait_for_settle();
    app.enable_codegen();

    let cancel = Selector::role_named(Role::Button, "Cancel");

    // The centers contract, spelled through the widening: `None` keeps
    // the M33 byte-identical shape (76, 32) → (76, 72).
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .drag_to_with_positions(&cancel, None, None)
        .expect("the drag runs");
    println!("centers (None, None) → {}", outcome.detail);

    // The offset drag: aim 8 px inside each bounds corner — the suite
    // asks (24, 24) → (136, 64), the ratchet runs between the aimed
    // points exactly as between the centers.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .drag_to_with_positions(&cancel, Some((8.0, 8.0)), Some((120.0, 8.0)))
        .expect("the drag runs");
    println!("offsets (8,8)/(120,8) → {}", outcome.detail);

    // One-sided: only the source end carries an offset — the source aims
    // where the suite asks, the target keeps its center.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .drag_to_with_positions(&cancel, Some((100.0, 30.0)), None)
        .expect("the drag runs");
    println!("one-sided (100,30)/None → {}", outcome.detail);

    // The §13-over-§9 record: the applied drags land as the raw kit
    // lines, and the aimed coordinates are the coordinates recorded.
    let (code, warnings) = app.codegen_code().expect("the recorder is on");
    println!("codegen → {code}");
    if !warnings.is_empty() {
        println!("codegen warnings → {warnings:?}");
    }
}
