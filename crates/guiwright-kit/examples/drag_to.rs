//! The locator drag surface (§16 actions row — the m19 parity stream's
//! named drag item, M33): `source.drag_to(target_selector)` — the
//! Playwright `locator.dragTo(target)` shape on the kit's borrow-and-act
//! doctrine. Both ends auto-wait `Attached` in one window context (the
//! source locator's pin, §16 v0.4), the drag runs from the source's
//! bounds center to the target's through the session's gesture synthesis
//! (the adapter's pacing table ratchets the pointer path, §9), and the
//! applied gesture records the raw `mouse.drag(...)` kit line while a
//! codegen recording runs (§13 over §9).
//!
//! ```text
//! cargo run -p guiwright-kit --example drag_to
//! ```
//!
//! Drags Save → Cancel on the button fixture (both centers, the
//! synthesized path), aims a window-pinned drag inside the two-window
//! fixture, and prints the recorded kit code.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // --- The embedded drag: both ends auto-wait, the path synthesizes ---
    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("drag_to_demo", adapter);
    app.wait_for_settle();
    app.enable_codegen();

    let outcome = app
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Cancel"))
        .expect("the drag runs");
    println!("drag_to(Cancel) → {}", outcome.detail);
    println!(
        "  pointer path: {} events, press at the source's center (76, 32), release at the target's (76, 72)",
        app.session().adapter().window_events().len()
    );

    // The §13-over-§9 record: the applied drag lands as the raw kit line
    // (the input vocabulary has no two-locator shape to validate).
    let (code, warnings) = app.codegen_code().expect("the recorder is on");
    println!("codegen → {code}");
    if !warnings.is_empty() {
        println!("codegen warnings → {warnings:?}");
    }

    // --- The window-pinned drag: the aim resolves inside the subtree ---
    let fx = MultiWindowFixture::default();
    let w1 = fx.w1.0.to_string();
    let mut app = App::launch("drag_to_demo_window", MockAdapter::with_multi_window(&fx));
    app.wait_for_settle();
    let mut editor = app.window(&w1).expect("Editor is a live target");
    let outcome = editor
        .get_by_role(Role::Button, "Save")
        .drag_to(&Selector::role_named(Role::Button, "Save"))
        .expect("the aim resolves inside the Editor subtree");
    println!("editor drag_to(Save) → {}", outcome.detail);
}
