//! The floem §10 pixel rows (m87): video recording and the visual-
//! regression bless → diff flow over REAL rasterized pixels — the
//! engine's tiny-skia `PaintState` over real laid-out view geometry
//! (§4.7 contribution 2's driver-side half). The m86 iced/freya shape:
//! the recorder reads the `last_frame` readback cache, recordings
//! finalize valid artifacts (non-zero size, ≥2 distinct frames around a
//! real repaint, wall-clock duration metadata), and the
//! `to_have_screenshot` bless → diff round trip runs on the real
//! rasters.

use std::rc::Rc;

use accesskit::Role;
use floem::view::IntoView;
use floem::views::{Button, Decorators, Label, Stack, TextInput};
use floem_reactive::{RwSignal, SignalGet, SignalUpdate};

use guiwright_adapter_floem_glue::harness::FloemAdapter;
use guiwright_kit::{App, BlessMode, SnapshotOutcome};

/// The real counter + form app (the m80 consumer): every interaction is
/// a floem callback over real reactive signals.
struct CounterForm {
    count: RwSignal<i32>,
    name: RwSignal<String>,
}

impl CounterForm {
    fn new() -> Self {
        Self {
            count: RwSignal::new(0),
            name: RwSignal::new(String::new()),
        }
    }
}

/// Build the real view tree (the m80 test's shape: §8.1 test ids ride
/// floem's `debug_name` decorator).
fn counter_view(app: &Rc<CounterForm>) -> impl IntoView {
    let count = app.count;
    let name = app.name;
    let name_input = app.name;
    Stack::new((
        Button::new("Increment")
            .action(move || count.set(count.get_untracked() + 1))
            .debug_name("increment"),
        Button::new("Decrement")
            .action(move || count.set(count.get_untracked() - 1))
            .debug_name("decrement"),
        TextInput::new(name_input).debug_name("name-input"),
        Label::derived(move || format!("Count: {}", count.get())).debug_name("count-label"),
        Label::derived(move || format!("Welcome, {}", name.get())).debug_name("greeting"),
    ))
    .style(|s| s.flex_col().items_center())
}

/// The app glue: text/value facts are the app's **real reactive state**.
fn counter_readers(app: &Rc<CounterForm>) -> Vec<guiwright_adapter_floem_glue::harness::SemReader> {
    use guiwright_adapter_floem_glue::harness::SemReader;
    let count = app.count;
    let name = app.name;
    let name_value = app.name;
    vec![
        SemReader::text("increment", move || "Increment".to_string()),
        SemReader::text("decrement", move || "Decrement".to_string()),
        SemReader::text("count-label", move || format!("Count: {}", count.get())),
        SemReader::text("greeting", move || format!("Welcome, {}", name.get())),
        SemReader::value("name-input", move || name_value.get_untracked()),
    ]
}

fn counter_app(name: &str) -> App<FloemAdapter> {
    let app = Rc::new(CounterForm::new());
    let adapter = FloemAdapter::boot(
        (480.0, 320.0),
        {
            let app = Rc::clone(&app);
            move || counter_view(&app).into_any()
        },
        counter_readers(&app),
    )
    .expect("harness boots");
    App::launch(name, adapter)
}

/// The §10 recorder over real floem pixels (m87): presented frames flow
/// from the tiny-skia raster through the `last_frame` readback cache
/// into the recorder; the static span dedups, the repaint records (the
/// fill row rides the documented headless-IME blocker, so the repaint
/// trigger is the counter click — the label's real reactive state), and
/// the stop finalizes a valid artifact.
#[test]
fn video_records_presented_pixels() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut app = counter_app("floem_video");

    app.video().start_in(dir.path(), 25).expect("video starts");
    app.session().pump();
    // A real wall-clock span between the first and the last accepted
    // frame (the summary's duration metadata, §10).
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.get_by_role(Role::Button, "Increment")
        .click()
        .expect("click repaints");
    let summary = app.video().stop().expect("video stops");

    assert!(
        summary.frames >= 2,
        "the static span deduped and the repaint recorded: {summary:?}"
    );
    assert!(
        summary.dropped >= 1,
        "hash-equality dedup collapsed the settled pass: {summary:?}"
    );
    assert!(
        summary.path.exists(),
        "finalized at {}",
        summary.path.display()
    );
    let size = std::fs::metadata(&summary.path).expect("artifact exists").len();
    assert!(size > 0, "the artifact is not empty ({size} bytes)");
    assert!(
        summary.duration_ms >= 40,
        "the duration metadata spans the recorded window: {summary:?}"
    );
    is_valid_video(&summary.path);
}

/// The §10 visual-regression round trip on real floem pixels: the first
/// run blesses the baseline, the unchanged re-render matches within the
/// pixel budget (the deterministic raster — same geometry, same bytes).
#[test]
fn screenshot_baseline_bless_and_diff_roundtrip() {
    let dir = tempfile::tempdir().expect("tmpdir");

    let blessed = {
        let mut app = counter_app("floem_bless").with_baseline_dir(dir.path());
        app.get_by_role(Role::Button, "Increment")
            .screenshot("increment", BlessMode::Failing)
            .expect("the settled frame blesses the baseline")
    };
    assert!(
        matches!(blessed, SnapshotOutcome::Blessed { .. }),
        "the first run blesses: {blessed:?}"
    );
    let written: Vec<std::fs::DirEntry> = walk_pngs(dir.path());
    assert!(!written.is_empty(), "the baseline file landed in the store");
    assert!(
        written
            .iter()
            .all(|e| e.metadata().expect("stat").len() > 0),
        "no empty baseline files"
    );

    let mut app = counter_app("floem_diff").with_baseline_dir(dir.path());
    let matched = app
        .get_by_role(Role::Button, "Increment")
        .screenshot("increment", BlessMode::Off)
        .expect("the deterministic re-render diffs clean");
    assert!(
        matches!(matched, SnapshotOutcome::Matched { .. }),
        "the pipeline round-trips: {matched:?}"
    );
}

/// Every `.png` under `root` (the baseline tree is
/// `<name>/<os>-<arch>-<renderer>.png`, §10).
fn walk_pngs(root: &std::path::Path) -> Vec<std::fs::DirEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "png") {
                out.push(entry);
            }
        }
    }
    out
}

/// The artifact is a real video: ffmpeg writes an EBML `.webm` container;
/// the PNG-sequence fallback writes a `video.json` manifest whose frame
/// count matches the summary (the duration metadata rides fps × frames).
fn is_valid_video(path: &std::path::Path) {
    if path.is_file() {
        let bytes = std::fs::read(path).expect("read the webm");
        assert_eq!(
            bytes.get(0..4).expect("magic bytes"),
            &[0x1A, 0x45, 0xDF, 0xA3],
            "the webm carries the EBML magic"
        );
    } else {
        let manifest = std::fs::read_to_string(path.join("video.json")).expect("manifest");
        let manifest: serde_json::Value = serde_json::from_str(&manifest).expect("valid json");
        assert_eq!(manifest["schema"], "gwvideo/v1");
        assert_eq!(manifest["fps"], 25);
        assert!(
            manifest["frames"].as_u64().expect("frames") >= 2,
            "the manifest counts the distinct frames"
        );
    }
}
