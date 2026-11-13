//! The `all()` enumeration surface (§16 assertions row — the m22/m25
//! records' open kit gap, closed here): `locator.all(visit)` enumerates the
//! locator's current match set in document order, handing each match to the
//! visitor as an nth-pinned locator — Playwright's `Locator.all()`, adapted
//! to the kit's borrow-and-act shape (a `Vec<Locator>` cannot exist while
//! every locator holds the `&mut App`, so the matches arrive one at a time).
//!
//! ```text
//! cargo run -p guiwright-kit --example all_matches
//! ```
//!
//! Enumerates the fixture's buttons immediately (document order, pinned
//! selectors), collects per-item data through the generic return, acts on a
//! visited item, scripts a late-appearing button through the mock's shared
//! channel so a re-enumeration shows the set grow, and enumerates a
//! window-scoped locator on a two-window app.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut app = App::launch("all_matches_demo", MockAdapter::with_fixture(&fixture));
    app.enable_trace(std::env::temp_dir().join("all_matches_demo.zip"));
    app.wait_for_settle();

    // The snapshot read: every button the tree holds right now, in
    // document order — the same set `count()` reads, one visit per match.
    let buttons = app
        .role_locator(Role::Button)
        .all(|item| Ok(item.selector().render()))
        .expect("enumeration resolves");
    println!("{} button(s), document order:", buttons.len());
    for pinned in &buttons {
        println!("  {pinned}");
    }

    // The generic return collects data: each visited locator maps to the
    // identity of the node it resolves to (the whole-set read).
    let keys = app
        .role_locator(Role::Button)
        .all(|item| Ok(item.resolve()?.key))
        .expect("identity collection");
    println!("collected {} node identities from the same set", keys.len());

    // Iterate-and-act: each visited locator acts like a hand-written
    // `.nth(i)` locator — auto-waits, records, everything. The Email
    // input is the fixture's one match for its label.
    app.get_by_label("Email")
        .all(|item| {
            let pinned = item.selector().render();
            item.fill("ada@lovelace.dev")?;
            println!("filled the visited input: {pinned}");
            Ok(())
        })
        .expect("the visited fill applies");

    // The enumeration is a snapshot, not a wait: a late button shows up
    // on the NEXT enumeration. Script it 300 ms in, re-enumerate, and the
    // set has grown by one (the count assertion pins the arrival itself).
    let scripts = Arc::new(Mutex::new(Vec::new()));
    let late = fixture.add_button_update("Export");
    let scripted = MockAdapter::with_fixture(&fixture).with_shared_scripts(scripts.clone());
    let mut grower = App::launch("all_matches_grower", scripted);
    grower.wait_for_settle();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        scripts.lock().expect("script channel").push(late);
    });
    guiwright_kit::expect(grower.role_locator(Role::Button))
        .to_have_count(5)
        .expect("the export button lands the fifth");
    let after = grower
        .role_locator(Role::Button)
        .all(|item| Ok(item.selector().render()))
        .expect("re-enumeration");
    println!("after the late button: {} matches", after.len());
    for pinned in &after {
        println!("  {pinned}");
    }

    // The window pin scopes the set: the two-window fixture carries one
    // "Save" per window, so the whole-tree locator enumerates both and
    // the Preview-pinned one enumerates exactly its own.
    let fx = MultiWindowFixture::default();
    let w2 = fx.w2.0.to_string();
    let mut multi = App::launch("all_matches_windows", MockAdapter::with_multi_window(&fx));
    multi.wait_for_settle();
    let everywhere = multi
        .get_by_role(Role::Button, "Save")
        .all(|item| Ok(item.resolve()?.key))
        .expect("whole-tree enumeration");
    let preview = multi
        .window(&w2)
        .expect("Preview is a live target")
        .get_by_role(Role::Button, "Save")
        .all(|item| Ok(item.resolve()?.key))
        .expect("scoped enumeration");
    println!(
        "window pin: {} save(s) tree-wide, {} inside Preview",
        everywhere.len(),
        preview.len()
    );
}
