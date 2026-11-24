//! The REPL assertion rows (m61): the m54/m59 client-seam assertions'
//! interactive half — `assert text|value|count <get-args…> is <expected>`
//! riding the [`guiwright_proto::client::ClientConn`] seam rows with the
//! `timeout <ms>` budget, event-driven re-reads (no polling), the typed
//! timeout naming the seam's state word, and the `events on` filter
//! restored after the assertion's subscription window.
//!
//! ```text
//! cargo run -p guiwright-cli --example repl_assert_rows
//! # prints the satisfied count assertion answering the found count, an
//! # assertion that stays pending until a tree update joins a fifth
//! # button (the scripted "Later" update — the wake-and-re-read loop),
//! # the typed timeout naming `have count` and the last evaluation, and
//! # the surviving session (the debugging tool outlives its failures)
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
    // pending count assertion below wakes on the tree event.
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
    let path = dir.path().join("repl-assert.sock");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let handle = guiwright_driver::spawn_session_server_with(
            path.clone(),
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
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

        // Satisfied on entry: the fixture's four buttons answer their
        // count, the found number is the reply (the m59 answer shape).
        step(&mut repl, "assert count role Button is 4").await;

        // Pending until the scripted update lands: the assertion rides
        // the seam's wake loop — tree event, re-read, answered.
        step(&mut repl, "assert count role Button is 5").await;

        // The content pair: the satisfying row renders as the m37 facts,
        // directly aimable by the action vocabulary.
        step(&mut repl, r#"assert text role Button Later is "Later""#).await;
        step(
            &mut repl,
            r#"assert value role TextInput Email is "ada@example.dev""#,
        )
        .await;

        // A spent budget prints the typed timeout — `have count`, the
        // locator, the count the last evaluation found — and the session
        // survives it (the debugging tool outlives its failures).
        step(&mut repl, "timeout 150").await;
        step(&mut repl, "assert count role Button is < 5").await;
        // The inclusive comparators (m25's honest-list widening): the
        // bound itself holds, the grammar speaking `>=`/`<=` — the set
        // stands at 5 here, so the inclusive bounds 4 and 5 both answer.
        step(&mut repl, "timeout 2000").await;
        step(&mut repl, "assert count role Button is >= 4").await;
        step(&mut repl, "assert count role Button is <= 5").await;
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
