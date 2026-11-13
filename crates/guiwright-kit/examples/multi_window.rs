//! The kit's multi-window handle surface (§16 v0.4 "targets per window",
//! the embedded half of M6): `app.windows()` enumerates the adapter's
//! window targets, `app.window(handle)` aims a borrow-and-act handle at
//! one target, and every locator born from that handle resolves only
//! inside the window's subtree — the same semantics the wire's
//! `Window::Use` gives a remote connection, with no new session or wire
//! code.
//!
//! ```text
//! cargo run -p guiwright-kit --example multi_window
//! ```
//!
//! Walks the discriminator shape on the two-window fixture — both windows
//! carry a "Save" button under independent subtree namespaces, so the
//! unscoped locator is ambiguous and the scope is what tells them apart —
//! then prints the window-scoped codegen lines the run records
//! (`app.window("…").get_by_role(…).click();`).

use accesskit::Role;
use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, expect};

fn main() {
    // The two-window fixture (M6): a root tree grafting an "Editor" and a
    // "Preview" subtree, each with its own "Save" button. Any real
    // multi-window adapter drops in unchanged — the handles are the
    // AccessKit subtree TreeIds the adapter already pushes.
    let fx = MultiWindowFixture::default();
    let mut app = App::launch("multi_window_demo", MockAdapter::with_multi_window(&fx));
    app.enable_codegen();
    app.wait_for_settle();

    // The adapter's window targets: handle / title / focus, the same
    // vocabulary `window list` prints in the REPL.
    println!("window targets ({}):", app.windows().len());
    for w in app.windows() {
        println!(
            "  {} {}{}",
            w.handle,
            w.title,
            if w.focused { " focused" } else { "" }
        );
    }

    let w1 = fx.w1.0.to_string();
    let w2 = fx.w2.0.to_string();

    // Without a scope, two "Save" buttons is a strict-mode failure —
    // the ambiguity the scope exists to resolve:
    match app.get_by_role(Role::Button, "Save").resolve() {
        Ok(_) => unreachable!("the fixture's Save buttons are ambiguous unscoped"),
        Err(err) => println!("\nunscoped get_by_role(Button, \"Save\") → {err}"),
    }

    // Aim at one window; the locators born from the handle are scoped:
    let mut preview = app.window(&w2).expect("Preview is a live target");
    println!("\naimed at {} ({}):", preview.handle(), preview.title());
    let save = preview
        .get_by_role(Role::Button, "Save")
        .resolve()
        .expect("Save inside Preview");
    println!("  scoped resolve → key {:?}", save.key);
    expect(preview.get_by_text("Preview text"))
        .to_be_visible()
        .expect("Preview text is visible inside Preview");
    println!("  expect(get_by_text(\"Preview text\")).to_be_visible() → ok");
    preview
        .get_by_role(Role::Button, "Save")
        .click()
        .expect("scoped click");
    println!("  scoped click → Preview's Save");

    // The Editor window never saw the click, and its own Save still
    // resolves through its handle:
    let mut editor = app.window(&w1).expect("Editor is a live target");
    editor
        .get_by_role(Role::Button, "Save")
        .resolve()
        .expect("Save inside Editor");
    println!("  Editor's Save still resolves through its own handle");

    // A dead handle fails at aim time (the tree is the source of truth):
    println!(
        "aimed at a dead handle → {}",
        app.window("00000000-dead-beef-0000-000000000000")
            .expect_err("unknown window")
    );

    // The codegen lines replay the scoping — recorded at the same tap as
    // every other action, scoped facts and all:
    let (code, warnings) = app.codegen_code().expect("codegen was enabled");
    println!("\ncodegen lines:");
    print!("{code}");
    for warning in warnings {
        println!("warning: {warning}");
    }
}
