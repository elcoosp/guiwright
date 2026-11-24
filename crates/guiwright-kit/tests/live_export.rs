//! The kit's live capture (the m18 record's remaining-gaps list: *"The
//! kit's `export_trace_viewer` still archives first. Wiring `LiveFeed`
//! into the kit's trace flow (live export during a test run) is a
//! kit-side follow-on; M18 ships the viewer-side capability"*).
//!
//! The acceptance this pins: **a kit test can capture the live scrubber
//! while it runs** — the capture attaches the M18 `LiveFeed` over the
//! remote-to-self door, pumps on its own thread while the test body
//! acts over the same door, and `finish` writes M23's `render_live_html`
//! output stamped `gwtrace/v1 (live)` — the m87 CLI shape, now at kit
//! width, where the run itself is the capture window.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_core::selector::{Selector, TextMatch, TextPattern};
use guiwright_kit::remote::RemoteToSelf;
use guiwright_proto::schema::SemActionV1;

/// The scripted rename (the events test's shape): the queued update the
/// idle tick publishes as a `TreeChanged` — the fact the feed's
/// auto-sync rides.
fn rename_save_update() -> accesskit::TreeUpdate {
    let mut save = accesskit::Node::new(Role::Button);
    save.set_label("Save (renamed)");
    save.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 16.0,
        x1: 136.0,
        y1: 48.0,
    });
    accesskit::TreeUpdate {
        nodes: vec![(accesskit::NodeId(2), save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    }
}

/// A Role selector by exact name (the kit's core vocabulary over the
/// door bridge).
fn role_named(role: accesskit::Role, name: &str) -> Selector {
    Selector::Role {
        role,
        name: Some(TextMatch {
            pattern: TextPattern::Str(name.into()),
            exact: true,
            ignore_case: false,
        }),
    }
}

/// Wait for the fact, with a deadline — the capture pumps on its own
/// thread, so the test polls the shared counter instead of sleeping a
/// fixed guess (the event-driven shape: the deadline is the budget, the
/// fact is the release).
fn wait_for_dumps(capture: &guiwright_kit::LiveCapture, want: usize, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while capture.snapshot_count() < want && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        capture.snapshot_count() >= want,
        "{what}: {} dumps after the deadline",
        capture.snapshot_count()
    );
}

/// The m18 acceptance: the capture rides while the test body acts —
/// the attach dump lands first, the test's click over the door
/// triggers a `TreeChanged` auto-sync (a second dump), and `finish`
/// writes the scrubber HTML stamped `gwtrace/v1 (live)` with the
/// adapter header.
#[test]
fn live_capture_rides_the_door_during_the_run() {
    let out = tempfile::tempdir().expect("scratch dir");
    let path = out.path().join("run.live.html");

    // The adapter is built on the session thread (the factory shape);
    // the shared script queue is how the test body advances the tree
    // mid-run (the events test's harness-style scripting).
    let scripts: Arc<Mutex<Vec<accesskit::TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&scripts);
    let mut door = RemoteToSelf::launch(move || {
        let mut adapter = guiwright_driver::mock::MockAdapter::with_fixture(
            &guiwright_core::test_util::Fixture::buttons(),
        );
        adapter.push_frame(1);
        adapter.push_frame(1);
        adapter.with_shared_scripts(shared)
    })
    .expect("the door comes up");
    let capture = door
        .start_live("live_export_demo")
        .expect("the feed attaches");
    assert_eq!(capture.adapter(), "mock", "the attach names the adapter");
    wait_for_dumps(&capture, 1, "the attach dump");

    // The test body acts over the same door while the capture pumps.
    let save = door
        .resolve(role_named(Role::Button, "Save"), 2_000)
        .expect("the strict resolve over the door");
    door.act(save.node, save.generation, SemActionV1::Click, false, 2_000)
        .expect("the click lands");

    // The tree moves (the scripted rename): the idle tick publishes the
    // diff as `TreeChanged` and the feed auto-syncs a fresh dump.
    scripts
        .lock()
        .expect("script queue")
        .push(rename_save_update());
    wait_for_dumps(&capture, 2, "the tree change auto-syncs");

    let export = capture.finish(&path).expect("the capture lands");
    assert_eq!(export.path, path);
    assert_eq!(export.adapter, "mock");
    assert!(export.dumps >= 2, "the attach dump plus the change's sync");

    let html = std::fs::read_to_string(&export.path).expect("the scrubber HTML");
    assert!(html.contains("gwtrace/v1 (live)"), "the live stamp");
    assert!(
        html.contains("mock"),
        "the adapter header names the fixture"
    );

    door.finish().expect("the door shuts down");
}

/// The m87 rule at kit width — the session's end never eats the
/// capture: a quiet door (no actions after attach) still writes the
/// entry-state scrubber, the attach dump carrying the tree.
#[test]
fn a_quiet_door_still_captures_the_entry_state() {
    let out = tempfile::tempdir().expect("scratch dir");
    let path = out.path().join("quiet.live.html");

    let door = RemoteToSelf::launch_fixture();
    let capture = door.start_live("quiet_export").expect("the feed attaches");
    wait_for_dumps(&capture, 1, "the attach dump");

    let export = capture.finish(&path).expect("the quiet capture lands");
    assert_eq!(export.dumps, 1, "no tree changes — the attach dump only");

    let html = std::fs::read_to_string(&export.path).expect("the scrubber HTML");
    assert!(html.contains("gwtrace/v1 (live)"), "the live stamp");

    door.finish().expect("the door shuts down");
}

/// Dropping the capture without `finish` detaches the pump (the stop
/// half drops, the thread exits, the door keeps serving) — the kit's
/// `finish` still joins the door cleanly afterwards.
#[test]
fn dropping_the_capture_detaches_the_pump() {
    let mut door = RemoteToSelf::launch_fixture();
    let capture = door
        .start_live("dropped_capture")
        .expect("the feed attaches");
    wait_for_dumps(&capture, 1, "the attach dump");
    drop(capture);

    // The door still answers for the test body after the detach.
    door.resolve(role_named(Role::Button, "Save"), 2_000)
        .expect("the door serves after the capture detaches");
    door.finish().expect("shutdown joins with the pump gone");
}
