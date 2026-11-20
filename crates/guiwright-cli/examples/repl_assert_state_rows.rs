//! The REPL state-assertion rows (m74): the m73 client-seam state
//! assertions' interactive half — `assert checked <get-args…> is
//! checked|unchecked|mixed|not-checkable` / `assert enabled <get-args…>
//! is enabled|disabled` / `assert focused <get-args…>` riding the
//! [`guiwright_proto::client::ClientConn`] seam rows with the `timeout
//! <ms>` budget, event-driven re-reads (no polling), the four-way
//! checked word demanded exactly, and the typed timeout naming the
//! seam's state word and the fact the last evaluation found.
//!
//! ```text
//! cargo run -p guiwright-cli --example repl_assert_state_rows
//! # prints the tri-state asserted on entry (checked, unchecked, mixed),
//! # the not-checkable fact answering as a fact, the focus arriving on a
//! # scripted update (the wake-and-re-read loop), and the typed timeout
//! # naming `be checked` and the word the last evaluation found
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{NodeId, TreeUpdate};
use guiwright_cli::{ConnectTarget, Repl};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;

fn main() {
    // The app's UI "changes" a moment after attach (a real app would be
    // painting; the scripted adapter pushes the update onto the next
    // pump): the fixture's "Dark mode" checkbox flips checked, and the
    // pending checked assertion below wakes on the tree event.
    let scripts: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts_for_push = Arc::clone(&scripts);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        if let Some(update) =
            Fixture::with_checkboxes().checkbox_toggle_update(NodeId(10), accesskit::Toggled::True)
        {
            scripts_for_push.lock().expect("script lock").push(update);
        }
    });

    let dir = tempfile::tempdir().expect("demo tmpdir");
    let path = dir.path().join("repl-assert-state.sock");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let handle = guiwright_driver::spawn_session_server_with(
            path.clone(),
            move || {
                MockAdapter::with_fixture(&Fixture::with_checkboxes()).with_shared_scripts(scripts)
            },
            guiwright_driver::ServerOptions {
                log_tail: true,
                ..guiwright_driver::ServerOptions::default()
            },
        )
        .expect("demo server binds");

        let mut repl = Repl::connect(&ConnectTarget::Uds(path))
            .await
            .expect("repl attaches");

        // The budget the assertions ride (the `timeout <ms>` row, m43's
        // rule): four seconds for the waitable conversation below.
        step(&mut repl, "timeout 4000").await;

        // The tri-state demanded exactly on entry: the checked,
        // unchecked and mixed words each answer their checkbox — the
        // satisfying m68 row renders the facts with the state three
        // named (the m74 render).
        step(
            &mut repl,
            "assert checked role CheckBox Notifications is checked",
        )
        .await;
        step(
            &mut repl,
            "assert checked role CheckBox \"Dark mode\" is unchecked",
        )
        .await;
        step(
            &mut repl,
            "assert checked role CheckBox \"Partially installed\" is mixed",
        )
        .await;

        // The four-way word's total fact: a not-checkable role answers
        // `not-checkable` — a fact, never an error.
        step(
            &mut repl,
            "assert checked role Button Save is not-checkable",
        )
        .await;

        // The enabled pair, both ways: the live checkbox answers
        // `enabled`, the disabled `Confirm` answers `disabled`.
        step(
            &mut repl,
            "assert enabled role CheckBox Notifications is enabled",
        )
        .await;
        step(&mut repl, "assert enabled role Button Confirm is disabled").await;

        // Pending until the scripted update lands: the assertion rides
        // the seam's wake loop — tree event, re-read, answered.
        step(
            &mut repl,
            "assert checked role CheckBox \"Dark mode\" is checked",
        )
        .await;

        // A spent budget prints the typed timeout — `be checked`, the
        // locator, the word the last evaluation found — and the session
        // survives it (the debugging tool outlives its failures).
        step(&mut repl, "timeout 150").await;
        step(
            &mut repl,
            "assert checked role CheckBox Notifications is unchecked",
        )
        .await;
        step(&mut repl, "ping").await;

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
