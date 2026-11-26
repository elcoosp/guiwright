//! The REPL chain tokens (m94): the m93 record's honest-list gap —
//! *"the REPL renders a pasted chain row (`a >> b`) but speaks no syntax
//! for building one — the narrowing words are the prompt's vocabulary,
//! the chain is the DSL's"* — paid. The `>>` binder joins full locator
//! segments into the wire's `All` chain, in composition order (source
//! first, the descendants after — the kit `locator(child)` shape, M34),
//! every segment speaking the m93 filter words. The split lives in the
//! shared get-args parse, so every get-args command speaks it — no wire
//! change (`SelectorV1::All` shipped with M34 and the resolver walks it
//! already; the composite-link fix at the resolver keeps the walk finite).
//!
//! ```text
//! cargo run -p guiwright-cli --example repl_chain_rows
//! # prints the chain reads (the descendant answers beside the ancestor
//! # `has` read, the leaf rule at chain width), filters inside segments,
//! # the strict get + click loop aiming the chained resolve, a count
//! # assertion riding the chain, and a probe that stays 0 until a
//! # scripted "Later" button joins the group — the chained read
//! # re-answered by the event-driven session, no polling
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
    // chained read below re-answers on the tree event.
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
    let path = dir.path().join("repl-chain.sock");

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

        // The m91 record's distinction at the prompt: the ancestor `has`
        // keeps the group, the `>>` chain answers the buttons inside its
        // subtree — same prefix, different rows.
        step(&mut repl, "count role Group has role Button").await;
        step(&mut repl, "count role Group >> role Button").await;

        // Subtree scope at chain width — the leaf rule: a label's subtree
        // is itself, so the chain into it answers nothing (an honest 0,
        // never an error — the m35 read rule).
        step(&mut repl, "count role Label >> role Button").await;

        // Segments speak the m93 narrowing words (the wire nests `Filter`
        // inside `All` already), and `>>` binds at chain level: the `has`
        // filter folds inside its segment first, the chain rides the
        // filtered source.
        step(&mut repl, "count role Group >> role Button has_text Save").await;
        step(&mut repl, "count role Group has role Button >> role Label").await;

        // The strict get rides the grammar: the chained resolve stores
        // the current node and the action vocabulary aims it — the full
        // click-what-you-see loop speaks chains.
        step(&mut repl, "get role Group >> role Button has_text Save").await;
        step(&mut repl, "click").await;

        // The assertion rows ride the same grammar (the m61 family).
        step(&mut repl, "assert count role Group >> role Button is 4").await;

        // The probe stays 0 until the scripted update lands: the chained
        // read re-answers on the tree event — the session's event-driven
        // loop, not a poll.
        step(&mut repl, "count role Group >> role Button has_text Later").await;
        step(
            &mut repl,
            "assert count role Group >> role Button has_text Later is 1",
        )
        .await;

        // Error honesty: the binder demands a locator on both sides, and
        // the grammar survives its user's mistakes (the session
        // continues).
        step(&mut repl, "count role Button >>").await;
        step(&mut repl, "count >> role Button").await;

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
