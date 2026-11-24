//! The machine-readable output mode (the m11/m13 records' named CLI
//! milestone — docs/m11-full-tree-snapshot.md §5: *"The REPL prints text;
//! a `--json` output mode for machine consumers rides the CLI's
//! output-formatting milestones"*): `guiwright repl --json` prints **one
//! JSON document per step** — the lines carry their kind word
//! (`ok`/`info`/`warn`/`err`) and raw text beside the text mode's
//! `error: ` prefixes, the exit flag rides the document, and the dump
//! rows stay machine-readable end to end.
//!
//! This demo drives the same [`Repl`] core the front-end runs and prints
//! each step through [`StepOutcome::to_json_string`] — the exact shape
//! `guiwright repl --json` puts on stdout.
//!
//! ```text
//! cargo run -p guiwright-cli --example repl_json_rows
//! # prints one JSON document per step: a dump answering its summary
//! # rows, a strict-mode failure as a kind:"err" line (no `error: `
//! # decoration in the machine channel), and the bye step flipping the
//! # document's exit flag
//! ```

use guiwright_cli::{ConnectTarget, Repl};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;

fn main() {
    let dir = tempfile::tempdir().expect("demo tmpdir");
    let path = dir.path().join("repl-json.sock");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let handle = guiwright_driver::spawn_session_server(path.clone(), || {
            MockAdapter::with_fixture(&Fixture::buttons())
        })
        .expect("demo server binds");

        let mut repl = Repl::connect(&ConnectTarget::Uds(path))
            .await
            .expect("repl attaches");

        // The machine channel: every step is one document, rows included.
        step(&mut repl, "snapshot full").await;

        // A failure stays in the document (kind:"err", raw text — here
        // the typed timeout answering the absent "Ghost" button) — the
        // machine consumer parses the conversation, never a stderr side
        // channel.
        step(&mut repl, "get role button Ghost").await;

        // The conversation's end rides the same document shape: the exit
        // flag flips so a driver loop knows to stop feeding lines.
        step(&mut repl, "bye").await;

        handle.shutdown();
    });
}

/// One REPL step, printed as the `--json` front-end prints it: one JSON
/// document per step on stdout.
async fn step(repl: &mut Repl, line: &str) {
    let outcome = repl.step(line).await;
    println!("> {line}");
    println!("  {}", outcome.to_json_string());
}
