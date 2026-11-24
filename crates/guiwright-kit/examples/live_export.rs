//! The kit's live capture (the m18 record's remaining-gaps list: *"The
//! kit's `export_trace_viewer` still archives first. Wiring `LiveFeed`
//! into the kit's trace flow (live export during a test run) is a
//! kit-side follow-on; M18 ships the viewer-side capability"*): the
//! M18 feed attaches over the remote-to-self door, pumps on its own
//! thread while the run acts over the same door, and `finish` writes
//! M23's `render_live_html` output stamped `gwtrace/v1 (live)` — the
//! m87 CLI shape, at kit width, where the run itself is the capture
//! window.
//!
//! ```text
//! cargo run -p guiwright-kit --example live_export
//! ```
//!
//! Prints the attach facts, the dump count as the tree moves and the
//! finished export's stats; the written file replays offline — no
//! server, no assets.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::Role;
use guiwright_core::selector::{Selector, TextMatch, TextPattern};
use guiwright_kit::RemoteToSelf;
use guiwright_proto::schema::SemActionV1;

fn main() {
    // The adapter rides the session thread; the shared script queue
    // moves the tree mid-run (the harness-style scripting).
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

    // The capture attaches beside the run (a second door connection)
    // and pumps while the body below acts.
    let capture = door
        .start_live("live_export_demo")
        .expect("the feed attaches");
    println!(
        "capturing {} over the door (adapter {})",
        capture.adapter(),
        door.adapter().name
    );

    let save = door
        .resolve(
            Selector::Role {
                role: Role::Button,
                name: Some(TextMatch {
                    pattern: TextPattern::Str("Save".into()),
                    exact: true,
                    ignore_case: false,
                }),
            },
            2_000,
        )
        .expect("strict resolve over the door");
    door.act(save.node, save.generation, SemActionV1::Click, false, 2_000)
        .expect("the click lands");

    // The tree moves; the feed auto-syncs a fresh dump per change.
    scripts
        .lock()
        .expect("script queue")
        .push(accesskit_tree_update());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while capture.snapshot_count() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("dumps so far: {}", capture.snapshot_count());

    let export = capture
        .finish("live_export_demo.live.html")
        .expect("the capture lands");
    println!(
        "export: {} ({} dumps, {} frames, {} logs)",
        export.path.display(),
        export.dumps,
        export.frames,
        export.logs
    );

    door.finish().expect("the door shuts down");
}

/// The scripted rename (the events test's shape): the queued update the
/// idle tick publishes as a `TreeChanged` — the fact the feed's
/// auto-sync rides.
fn accesskit_tree_update() -> accesskit::TreeUpdate {
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
