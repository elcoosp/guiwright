//! The locator filter surface (§16 locators row — the m34 filter
//! milestone): `locator.has_not(inner)` and `locator.locator(child)` —
//! the Playwright `filter({ hasNot })` and chained `locator(selector)`
//! shapes on the kit's borrow-and-act doctrine. `has_not` keeps only the
//! matches whose subtree holds no node matching the inner selector (the
//! inverted `has_text`); `locator(child)` resolves the locator's own set
//! first, then narrows to `child` inside each match's subtree. Both
//! compose with `first`/`nth`/`has_text`, ride the window pin, and act
//! through the same strict-mode resolve as every locator action.
//!
//! ```text
//! cargo run -p guiwright-kit --example locator_filter
//! ```
//!
//! Walks the button fixture — the group minus the buttons that carry a
//! Confirm, the `group >> button` chain, the composed filter (chain +
//! `has_not` + `first`) — and a window-pinned chain on the two-window
//! fixture, printing each set read.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::{Fixture, MultiWindowFixture};
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    // --- The inverted filter: containers that hold no matching node ---
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("locator_filter_demo", adapter);
    app.wait_for_settle();

    // The group's subtree carries Confirm (and the Email input): the
    // inverted filter drops it, a leaf that carries neither stays.
    let without_confirm = app
        .role_locator(Role::Group)
        .has_not(&Selector::role_named(Role::Button, "Confirm"))
        .count()
        .expect("the group read");
    let without_form = app
        .role_locator(Role::Group)
        .has_not(&Selector::role(Role::TextInput))
        .count()
        .expect("the group read");
    let leaf = app
        .get_by_text("Welcome, Ada")
        .has_not(&Selector::role(Role::Button))
        .count()
        .expect("the leaf read");
    println!("group .has_not(Confirm)   → {without_confirm} match(es)");
    println!("group .has_not(TextInput) → {without_form} match(es)");
    println!("label .has_not(Button)    → {leaf} match(es) (the leaf's subtree is itself)");

    // --- The chained descendant: group >> button, narrowed further ---
    let buttons = app
        .role_locator(Role::Group)
        .locator(&Selector::role(Role::Button))
        .count()
        .expect("the chain read");
    let save = app
        .role_locator(Role::Group)
        .locator(&Selector::role_named(Role::Button, "Save"))
        .first()
        .resolve()
        .expect("the chain resolves");
    println!("group >> Button        → {buttons} match(es)");
    println!(
        "group >> Button \"Save\" → resolved (generation {})",
        save.generation
    );

    // --- Composition: chain + has_not + first, one strict resolve ---
    let live = app
        .role_locator(Role::Group)
        .locator(&Selector::role(Role::Button))
        .has_not(&Selector::role_named(Role::Button, "Confirm"))
        .count()
        .expect("the composed read");
    println!("group >> Button, minus Confirm → {live} match(es)");

    // --- The window pin scopes the whole composition (§16 v0.4) ---
    let fx = MultiWindowFixture::default();
    let w1 = fx.w1.0.to_string();
    let mut app = App::launch(
        "locator_filter_demo_window",
        MockAdapter::with_multi_window(&fx),
    );
    app.wait_for_settle();
    let mut editor = app.window(&w1).expect("Editor is a live target");
    let scoped = editor
        .role_locator(Role::Group)
        .locator(&Selector::role_named(Role::Button, "Save"))
        .count()
        .expect("the scoped chain read");
    println!("editor: group >> Button \"Save\" → {scoped} match(es) (the pin scopes the walk)");
}
