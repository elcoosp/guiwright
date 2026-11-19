//! The hit-test read at the kit surface (M32): `app.hit_test(x, y)`
//! answers which node occupies a physical point — the same coordinate
//! space the resolve rects print, so the coordinates of a resolve reply
//! (or a trace frame) can be fed straight back. The window-scoped
//! companion `window.hit_test(x, y)` reads only inside one target's
//! subtree, the same pin every resolve path takes.
//!
//! ```text
//! cargo run -p guiwright-kit --example hit_testing
//! ```
//!
//! Hits inside the fixture's Save button, misses outside the tree (the
//! read is total — a miss is `None`, never an error), then hits inside
//! each window's Save button on the two-window fixture, where the scope
//! is what tells the two identical buttons apart.

use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // The buttons fixture: the Save button occupies rect
    // [16,16 → 136,48] (physical px), so (50, 30) is inside it.
    let mut app = App::launch(
        "hit_testing_demo",
        MockAdapter::with_fixture(&Fixture::buttons()),
    );
    app.wait_for_settle();

    let hit = app
        .hit_test(50.0, 30.0)
        .expect("the hit-test read resolves");
    match hit {
        Some(resolved) => println!(
            "hit (50, 30) → key {:?} (generation {}) inside {:?}",
            resolved.key, resolved.generation, resolved.bounds
        ),
        None => println!("hit (50, 30) → nothing"),
    }

    // A miss: the read is total, `None` is the answer (never an error),
    // and nothing stores (the read leaves no current node behind).
    let miss = app
        .hit_test(470.0, 230.0)
        .expect("the hit-test read resolves");
    println!("hit (470, 230) → {:?}", miss.is_none());

    // The window-scoped read (M6/M32): both windows carry a "Save"
    // button; the point inside the Editor's button answers from the
    // Editor subtree, the same point read through the Preview handle
    // answers from the Preview subtree.
    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "hit_testing_windows_demo",
        MockAdapter::with_multi_window(&fx),
    );
    app.wait_for_settle();

    let editor = &app.windows()[0].handle;
    let preview = &app.windows()[1].handle;
    let editor_hit = app
        .window(editor)
        .expect("the Editor is a live target")
        .hit_test(50.0, 30.0)
        .expect("the scoped hit resolves");
    let preview_hit = app
        .window(preview)
        .expect("the Preview is a live target")
        .hit_test(50.0, 430.0)
        .expect("the scoped hit resolves");
    println!(
        "window {editor} hit (50, 30) → key {:?}",
        editor_hit.as_ref().map(|r| r.key)
    );
    println!(
        "window {preview} hit (50, 430) → key {:?}",
        preview_hit.as_ref().map(|r| r.key)
    );
    // The scope is the pin: the Editor point read through the Preview
    // handle answers `None` — the Preview subtree holds nothing there,
    // so the two identical "Save" buttons cannot be confused.
    let cross = app
        .window(preview)
        .expect("the Preview is a live target")
        .hit_test(50.0, 30.0)
        .expect("the scoped hit resolves");
    println!(
        "window {preview} hit (50, 30) → {:?} (the Editor point misses this subtree)",
        cross.is_none()
    );
    assert_ne!(
        editor_hit.map(|r| r.key),
        preview_hit.map(|r| r.key),
        "each window's own button answers — the scope is the pin"
    );
}
