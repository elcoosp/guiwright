//! The §12 "open on failure" deep link at the kit surface — the failure
//! report's own words. The architecture doc's §12 viewer paragraph
//! promises *"open on failure" deep-links from assertion errors
//! (`guiwright show-trace …`)*, and the §16 error-context row claims the
//! pair (`error embeds tree snapshot + trace link`): the tree snapshot
//! rides the typed timeout error, and the trace link rides the runner's
//! failure line once `App::finish` has finalized the archive. This demo
//! drives the exact sequence `#[guiwright_kit::test]` generates — a body
//! that fails strict mode on a missing node, then `finish` before the
//! report — and prints the two lines a failing run carries:
//!
//! ```text
//! guiwright test `trace_deep_link` failed: no node matches …
//! trace: /tmp/…/guiwright-trace_deep_link-….zip (guiwright show-trace /tmp/…/….zip)
//! ```
//!
//! ```text
//! cargo run -p guiwright-kit --example trace_deep_link
//! ```

use accesskit::Role;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = guiwright_core::test_util::Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("trace_deep_link", adapter);
    app.wait_for_settle();

    // The archive arms where `GW_TRACE=1` would arm it at launch (the
    // env-driven path) — here with an explicit path so the demo names it.
    let dir = std::env::temp_dir().join(format!("gw-deep-link-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    app.enable_trace(dir.join("guiwright-trace_deep_link.zip"));

    // The body fails (§12's deep-link subject — an assertion error): the
    // fixture's tree carries no Button named "Never".
    let outcome = app
        .get_by_role(Role::Button, "Never")
        .resolve()
        .map(|_| ())
        .map_err(|e| e.to_string());

    // The macro's order: `finish` finalizes the archive BEFORE the
    // outcome reports — a failing test's archive is the valuable one.
    let trace = app.finish();

    // The report: the legacy line plus the deep link, the shape
    // `report_with_trace` panics with in a real run — printed here so the
    // demo stays a demo.
    match (outcome, trace) {
        (Err(err), Some(path)) => {
            println!("guiwright test `trace_deep_link` failed: {err}");
            println!(
                "trace: {} (guiwright show-trace {})",
                path.display(),
                path.display()
            );
        }
        (Err(err), None) => {
            println!("guiwright test `trace_deep_link` failed: {err}");
            println!("(no archive recorded — the legacy line stands alone)");
        }
        (Ok(()), trace) => {
            println!("the drive passed (trace: {trace:?}) — point the locator at a real node");
        }
    }
}
