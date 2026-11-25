//! The REPL filter grammar (m93): the m91/m92 records' named standing
//! gap — "the REPL's get-args vocabulary speaks no filter rows (`has_not`
//! included)" — paid. The §8.1 narrowing words ride the shared get-args
//! parse as postfix words (`has_text <text…>`, `has_not <get-args…>`,
//! `has <get-args…>`, `first` / `last` / `nth <N>`), composed in the kit
//! locator DSL's order, so every get-args command speaks them — the set
//! reads, the strict `get`, the assertion rows — no wire change (the
//! `Filter`/`Has` rows shipped with M34/m91; the resolver owns the
//! semantics, the REPL composes the rows).
//!
//! ```text
//! cargo run -p guiwright-cli --example repl_filter_rows
//! # prints the narrowed set reads (`has_text`/`has_not`/`has` + the nth
//! # forms), the leaf rule at the prompt, the strict get + click loop
//! # aiming the filtered resolve, a count assertion riding the words,
//! # and a probe that stays 0 until a scripted "Later" button joins —
//! # the filter read re-answered by the event-driven session, no polling
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::TreeUpdate;
use guiwright_cli::{ConnectTarget, Repl};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the update onto the next
    // pump): a fifth button joins under the fixture's group, and the
    // narrowed set read below re-answers on the tree event.
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        scripts_for_push
            .lock()
            .expect("script lock")
            .push(Fixture::buttons().add_button_update("Later"));
    });

    let dir = tempfile::tempdir().expect("demo tmpdir");
    let path = dir.path().join("repl-filter.sock");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let handle = guiwright_driver::spawn_session_server_with(
            path.clone(),
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
            guiwright_driver::ServerOptions::default(),
        )
        .expect("demo server binds");

        let mut repl = Repl::connect(&ConnectTarget::Uds(path))
            .await
            .expect("repl attaches");

        // The budget the waitable rows ride (the `timeout <ms>` row,
        // m43's rule): the count assertion below re-reads on the tree
        // event, no polling.
        step(&mut repl, "timeout 4000").await;

        // The positive text probe: the fuzzy default (substring,
        // case-insensitive) is the TextMatch default the kit sugar and
        // the m92 TS helper speak.
        step(&mut repl, "count role Button has_text Save").await;
        step(&mut repl, "count role Button has_text Ghost").await;

        // The m91 positive selector filter: the ancestor keeps itself —
        // `group.has(button)` answers the group, the exact chain
        // distinction the m91 record pinned.
        step(&mut repl, "count role Group has role Button").await;

        // The m34 inverted filter + the leaf rule at the prompt: a
        // button's subtree is itself, so `has_not role Button` excludes
        // every button.
        step(&mut repl, "count role Button has_not text Confirm").await;
        step(&mut repl, "count role Button has_not role Button").await;

        // The nth forms in document order; the fold composes in order
        // (the kit's `.has_text(..).first()` composition).
        step(&mut repl, "count role Button first").await;
        step(&mut repl, "count role Button nth 1").await;
        step(&mut repl, "all role Button has_text Save first").await;

        // The strict get rides the grammar: the filtered resolve stores
        // the current node and the action vocabulary aims it — the full
        // click-what-you-see loop speaks filters.
        step(&mut repl, "get role Button has_text Save").await;
        step(&mut repl, "click").await;

        // The assertion rows ride the same grammar (the m61 family).
        step(&mut repl, "assert count role Button has_text Save is 1").await;

        // The probe stays 0 until the scripted update lands: the
        // narrowed read re-answers on the tree event — the session's
        // event-driven loop, not a poll.
        step(&mut repl, "count role Button has_text Later").await;
        step(&mut repl, "assert count role Button has_text Later is 1").await;

        // Error honesty: every filter word demands its argument, and the
        // grammar survives its user's mistakes (the session continues).
        step(&mut repl, "count role Button has_text").await;
        step(&mut repl, "count role Button nth soon").await;

        // The conversation's end (the §7 lifecycle).
        step(&mut repl, "bye").await;

        handle.shutdown();
    });
}

/// One REPL step, rendered the way the interactive front-end prints it
/// (errors carry the `error:` prefix, events flush ahead of the answer).
async fn step(repl: &mut Repl, line: &str) {
    let outcome = repl.step(line).await;
    println!("> {line}");
    for line in &outcome.lines {
        println!("  {}", line.render());
    }
}
