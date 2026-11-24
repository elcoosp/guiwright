//! The replayable §13 overlay (the deep-inspect mode, m8's named viewer
//! gaps): the live session composites interactive-only badges at present
//! time, and the **viewer replays the full-tree plan from the archived
//! facts** — every bounds-carrying node badged, the recorded focus
//! outlined — through the same `OverlayPlan::from_facts` cost model.
//!
//! ```text
//! cargo run -p guiwright-viewer --example deep_inspect
//! ```
//!
//! Records a small `gwtrace/v1` archive, opens it back, replays the
//! deep-inspect plans for every snapshot and renders the self-contained
//! `trace.html` carrying the overlay layer (an SVG over the archive's
//! resolution, scrubbed with the timeline). Prints the paths and the
//! badge counts.

use guiwright_trace::TraceOptions;
use guiwright_trace::TraceRecorder;
use guiwright_trace::schema::{AdapterRecord, ClockCommand, ClockJump, TreeNode, TreeSnapshot};
use guiwright_viewer::{Timeline, TraceArchive, overlay};

fn main() {
    let dir = std::env::temp_dir().join("gw-deep-inspect-demo");
    std::fs::create_dir_all(&dir).expect("demo dir");
    let archive_path = dir.join("deep_inspect_demo.zip");

    // Record: a window with three interactive controls and a label, two
    // snapshots around a click, one deduped frame pair — the e2e shape.
    let options = TraceOptions::new(
        archive_path.clone(),
        "deep_inspect_demo",
        AdapterRecord {
            name: "mock".into(),
            framework: "fixture".into(),
            framework_version: "0.1.0".into(),
        },
    );
    let mut recorder = TraceRecorder::start(options);
    record_snapshots(&mut recorder);
    recorder.stop().expect("archive written");

    // Open it back and replay the deep-inspect plans per snapshot.
    let archive = TraceArchive::open(&archive_path).expect("archive opens");
    let timeline = Timeline::from_archive(&archive);
    let plans = overlay::snapshot_replay_plans(&timeline);
    println!(
        "recorded {} — {} snapshots, {} replayed plans",
        archive_path.display(),
        archive.snapshots.len(),
        plans.len(),
    );
    for (frame, plan) in &plans {
        let focus = plan.hover.map_or_else(
            || "(none)".into(),
            |r| format!("({:.0},{:.0})→({:.0},{:.0})", r.x0, r.y0, r.x1, r.y1),
        );
        println!(
            "  frame {frame}: {} full-tree badges, focus outline {focus}",
            plan.badges.len(),
        );
        for badge in &plan.badges {
            println!("    {} @ ({:.0},{:.0})", badge.text, badge.at.0, badge.at.1);
        }
    }

    // Render the self-contained viewer page: the layer rides the payload
    // and scrubs with the timeline.
    let html = guiwright_viewer::render_html(&archive, &timeline);
    let html_path = dir.join("trace.html");
    std::fs::write(&html_path, &html).expect("html written");
    println!("viewer page — open {} in a browser", html_path.display());
    println!(
        "  the Overlay section carries {} per-snapshot layers on a {}×{} canvas",
        plans.len(),
        archive.meta.resolution.0,
        archive.meta.resolution.1,
    );
}

/// One archived row (the replay's fact source: role, label, bounds).
fn node(
    id: u64,
    parent: Option<u64>,
    role: &str,
    label: Option<&str>,
    bounds: [f64; 4],
) -> TreeNode {
    TreeNode {
        id,
        parent,
        role: role.into(),
        label: label.map(str::to_string),
        value: None,
        bounds: Some(bounds),
    }
}

/// The demo session: two snapshots around a click (the label restates and
/// the focus moves), one deduped frame pair, one virtual-clock jump.
fn record_snapshots(recorder: &mut TraceRecorder) {
    recorder.tree_snapshot(TreeSnapshot {
        frame_seq: 1,
        nodes: vec![
            node(1, None, "Window", Some("App"), [0.0, 0.0, 800.0, 600.0]),
            node(
                2,
                Some(1),
                "Button",
                Some("Save"),
                [16.0, 16.0, 136.0, 48.0],
            ),
            node(
                3,
                Some(1),
                "Button",
                Some("Export"),
                [16.0, 56.0, 136.0, 88.0],
            ),
            node(
                4,
                Some(1),
                "Label",
                Some("Ready"),
                [16.0, 96.0, 136.0, 120.0],
            ),
        ],
        focus: Some(2),
    });
    recorder.event(1, "click", "click @ (76, 32)");
    recorder.frame(1, 0xAAAA, vec![1, 2, 3, 4]);
    recorder.frame(2, 0xAAAA, vec![]); // deduped (same hash)
    recorder.tree_snapshot(TreeSnapshot {
        frame_seq: 3,
        nodes: vec![
            node(1, None, "Window", Some("App"), [0.0, 0.0, 800.0, 600.0]),
            node(
                2,
                Some(1),
                "Button",
                Some("Save"),
                [16.0, 16.0, 136.0, 48.0],
            ),
            node(
                3,
                Some(1),
                "Button",
                Some("Export"),
                [16.0, 56.0, 136.0, 88.0],
            ),
            node(
                4,
                Some(1),
                "Label",
                Some("Saved."),
                [16.0, 96.0, 136.0, 120.0],
            ),
        ],
        focus: Some(3),
    });
    recorder.clock_jump(ClockJump {
        frame_seq: 4,
        command: ClockCommand::FastForward { micros: 30_000_000 },
        virtual_time_us: 30_000_000,
    });
}
