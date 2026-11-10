//! Element screenshots through the locator surface (§10, M9): the frame
//! is cropped by the resolved node's clip-aware rect — never re-rendered
//! per element — and asserted against the same per-platform baselines
//! [`App::screenshot`] uses. The mock adapter scripts deterministic
//! gradient pixels, so every crop is byte-checkable.

use std::time::Instant;

use accesskit::Role;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, BlessMode, SnapshotOutcome};

/// Deterministic gradient frame: pixel (x, y) = [x, y, 99, 255]. The
/// fixture's Save button sits at [16,16 → 136,48] physical px.
fn gradient_frame(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.push((x & 0xff) as u8);
            rgba.push((y & 0xff) as u8);
            rgba.push(99);
            rgba.push(255);
        }
    }
    rgba
}

fn screenshot_app(
    tag: &str,
    frame: Option<(u32, u32, Vec<u8>)>,
) -> (App<MockAdapter>, tempfile::TempDir) {
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    if let Some((w, h, rgba)) = frame {
        adapter = adapter.with_frame_pixels(w, h, rgba);
    }
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut app = App::launch(tag, adapter).with_baseline_dir(dir.path());
    app.wait_for_settle();
    (app, dir)
}

#[test]
fn element_screenshot_crops_the_resolved_rect() {
    let (mut app, dir) = screenshot_app("element_crop", Some((480, 240, gradient_frame(480, 240))));

    // Fresh baseline.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("first run writes the baseline");
    assert_eq!(outcome, SnapshotOutcome::Blessed { forced: false });

    // The baseline is exactly the resolved node's rect (§10: crop from the
    // window frame — no re-render, no padding).
    let baseline_path =
        guiwright_shot::baseline::BaselineStore::new(dir.path()).baseline_path("save", "fixture");
    let baseline = std::fs::read(baseline_path).expect("baseline written");
    let image = guiwright_shot::codec::decode_png(&baseline).expect("png");
    assert_eq!(image.width, 120, "136 - 16");
    assert_eq!(image.height, 32, "48 - 16");
    // Byte-check the crop against the scripted gradient.
    for y in 0..image.height {
        for x in 0..image.width {
            assert_eq!(
                image.pixel(x, y),
                [((x + 16) & 0xff) as u8, ((y + 16) & 0xff) as u8, 99, 255],
                "crop carries the window-frame pixels at ({x},{y})"
            );
        }
    }

    // Second run: matched within budget.
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Off)
        .expect("same pixels");
    assert!(matches!(outcome, SnapshotOutcome::Matched { .. }));
}

#[test]
fn element_screenshot_mismatch_writes_artifacts() {
    let (mut app, dir) = screenshot_app(
        "element_mismatch",
        Some((480, 240, gradient_frame(480, 240))),
    );
    app.get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline");

    // A repaint that flips two pixels beyond the §10 YIQ threshold
    // (mid-luma pairs pass the perceptual metric by design — anti-flake
    // for AA fringes — so the tamper forces pure-white pixels). A
    // mismatch is a `ShotError` (the assertion failed, like
    // `App::screenshot`).
    let mut tampered = gradient_frame(480, 240);
    for (x, y) in [(20u32, 20u32), (30, 25)] {
        let i = ((y * 480 + x) * 4) as usize;
        tampered[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
    }
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter = adapter.with_frame_pixels(480, 240, tampered);
    let mut app2 = App::launch("element_mismatch_2", adapter).with_baseline_dir(dir.path());
    app2.wait_for_settle();

    let err = app2
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Off)
        .expect_err("pixels drifted");
    assert!(
        err.to_string().contains("mismatch"),
        "mismatch wording, got: {err}"
    );
    // Artifacts land next to the baseline (§10): <stem>.new.png/.diff.png
    // where the stem is the per-platform baseline name.
    let baseline_dir = dir.path().join("save");
    let names: Vec<String> = std::fs::read_dir(&baseline_dir)
        .expect("baseline dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with(".new.png")),
        "actual artifact, have {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with(".diff.png")),
        "diff artifact, have {names:?}"
    );
}

#[test]
fn element_screenshot_without_pixels_is_a_clean_error() {
    let (mut app, _dir) = screenshot_app("element_nopixels", None);
    let err = app
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Off)
        .expect_err("the adapter renders no pixels");
    assert!(
        err.to_string().contains("render_frame"),
        "the §6.2 honest row (render_frame unsupported), got: {err}"
    );
}

#[test]
fn element_screenshot_on_a_bounds_less_node_is_a_clean_error() {
    let (mut app, _dir) = screenshot_app(
        "element_nobounds",
        Some((480, 240, gradient_frame(480, 240))),
    );
    // The fixture's hidden button has no bounds — nothing to crop.
    let err = app
        .get_by_role(Role::Button, "Invisible")
        .screenshot("invisible", BlessMode::Off)
        .expect_err("no bounds");
    assert!(
        err.to_string().to_lowercase().contains("bounds"),
        "bounds wording, got: {err}"
    );
}

#[test]
fn element_screenshot_is_budget_shaped() {
    // The §15.1 element-crop row: resolve + render + crop + baseline IO
    // must stay in the hundreds of microseconds to low milliseconds —
    // this is a smoke bound (the enforced guard lives in guiwright-bench).
    let (mut app, _dir) =
        screenshot_app("element_budget", Some((480, 240, gradient_frame(480, 240))));
    app.get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline");
    let start = Instant::now();
    app.get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Off)
        .expect("matched");
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "one element screenshot in {elapsed:?} (budget-shaped)"
    );
}
