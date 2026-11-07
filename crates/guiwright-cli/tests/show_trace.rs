//! `guiwright show-trace` (§12): record a `gwtrace/v1` archive with the
//! trace recorder (the viewer e2e's helper pattern), render it through
//! the CLI, assert the self-contained HTML carries the timeline payloads.

use guiwright_trace::TraceOptions;
use guiwright_trace::schema::{AdapterRecord, TreeNode, TreeSnapshot};

/// Record a small but complete archive: two snapshots, a diff, an event,
/// a clock jump and a deduped frame pair.
fn record_archive(path: std::path::PathBuf) -> guiwright_trace::TraceFile {
    let options = TraceOptions::new(
        path,
        "show_trace_e2e",
        AdapterRecord {
            name: "mock".into(),
            framework: "fixture".into(),
            framework_version: "0.1.0".into(),
        },
    );
    let mut recorder = guiwright_trace::TraceRecorder::start(options);
    recorder.tree_snapshot(TreeSnapshot {
        frame_seq: 1,
        nodes: vec![
            TreeNode {
                id: 1,
                parent: None,
                role: "Window".into(),
                label: Some("App".into()),
                value: None,
                bounds: Some([0.0, 0.0, 800.0, 600.0]),
            },
            TreeNode {
                id: 2,
                parent: Some(1),
                role: "Button".into(),
                label: Some("Save".into()),
                value: None,
                bounds: Some([16.0, 16.0, 136.0, 48.0]),
            },
        ],
        focus: Some(2),
    });
    recorder.event(1, "click", "click @ (76, 32)");
    recorder.tree_diff(2, vec![], vec![2], vec![], false);
    recorder.frame(1, 0xAAAA, vec![1, 2, 3, 4]);
    recorder.frame(2, 0xAAAA, vec![]); // deduped (same hash)
    recorder.stop().expect("trace finalized")
}

#[test]
fn show_trace_renders_the_html_next_to_the_archive() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let archive_path = dir.path().join("guiwright-show_trace_e2e-1.zip");
    let file = record_archive(archive_path.clone());
    assert!(archive_path.exists(), "archive recorded at {:?}", file.path);

    let out = guiwright_cli::show_trace(&archive_path, None, false).expect("rendered");
    assert_eq!(out, dir.path().join("guiwright-show_trace_e2e-1.html"));
    let html = std::fs::read_to_string(&out).expect("html written");
    // The self-contained viewer payloads (§12): timeline, tree, events.
    assert!(html.contains("gwtrace"), "schema marker present");
    assert!(html.contains("Save"), "tree node label present");
    assert!(html.contains("click @ (76, 32)"), "event present");
}

#[test]
fn show_trace_honors_an_explicit_out_path() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let out_dir = tempfile::tempdir().expect("tmpdir");
    let archive_path = dir.path().join("trace.zip");
    record_archive(archive_path.clone());

    let out = out_dir.path().join("custom.html");
    let written =
        guiwright_cli::show_trace(&archive_path, Some(out.clone()), false).expect("rendered");
    assert_eq!(written, out);
    assert!(out.exists());
}
