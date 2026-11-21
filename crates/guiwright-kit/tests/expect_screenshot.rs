//! `expect(locator).to_have_screenshot` — the §16 retry waiter over the
//! §10 baseline flow (Playwright `toHaveScreenshot` parity). The
//! single-shot `Locator::screenshot` decides exactly one diff; this
//! surface polls resolve → render → crop → diff on every presented
//! frame until the pixel budget is satisfied or the `expect` timeout
//! expires — the `to_have_text` retry semantics over pixels. The mock
//! adapter scripts a settling animation (`with_frame_pixel_steps`:
//! every render advances one step, the last sticks), so "retries and
//! eventually passes" is byte-checkable against the §10 baselines and
//! artifacts.

use std::time::Duration;

use accesskit::Role;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::{App, BlessMode};

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

/// The settled gradient with two forced-white pixels inside the Save
/// crop — beyond the §10 YIQ threshold (mid-luma pairs pass the
/// perceptual metric by design, so the tamper is pure white).
fn tampered_frame(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = gradient_frame(width, height);
    for (x, y) in [(20u32, 20u32), (30, 25)] {
        let i = ((y * width + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
    }
    rgba
}

/// An app whose adapter renders the settled gradient (the baseline
/// author) — artifacts and baselines live in a shared temp dir.
fn settled_app(tag: &str, dir: &std::path::Path) -> App<MockAdapter> {
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let adapter = adapter.with_frame_pixels(480, 240, gradient_frame(480, 240));
    let mut app = App::launch(tag, adapter).with_baseline_dir(dir);
    app.wait_for_settle();
    app
}

/// An app whose adapter plays `steps` — render one, the last sticks
/// (an animation that settles). Pumps never render, so `wait_for_settle`
/// leaves the sequence intact for the assertion under test.
fn animated_app(tag: &str, dir: &std::path::Path, steps: Vec<Vec<u8>>) -> App<MockAdapter> {
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let adapter = adapter.with_frame_pixel_steps(480, 240, steps);
    let mut app = App::launch(tag, adapter).with_baseline_dir(dir);
    app.wait_for_settle();
    app
}

#[test]
fn delayed_screenshot_settles_through_the_retry_loop() {
    let (mut baseline, dir) = {
        let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
        adapter.push_frame(1);
        adapter.push_frame(1);
        let adapter = adapter.with_frame_pixels(480, 240, gradient_frame(480, 240));
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut app = App::launch("expect_shot_settle_base", adapter).with_baseline_dir(dir.path());
        app.wait_for_settle();
        (app, dir)
    };
    baseline
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("the settled frame blesses the baseline");

    // The delayed state change: the first rendered frame still carries
    // the mid-animation tamper, the next render sees the settled UI. A
    // single-shot snapshot/diff fails on the first frame — the retry
    // waiter keeps polling presented frames until the budget holds.
    let mut app = animated_app(
        "expect_shot_settle_anim",
        dir.path(),
        vec![tampered_frame(480, 240), gradient_frame(480, 240)],
    );
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_have_screenshot("save", BlessMode::Off)
        .expect("the animation settled inside the retry loop");
}

#[test]
fn screenshot_retry_times_out_with_the_last_report_and_artifacts() {
    let (mut baseline, dir) = {
        let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
        adapter.push_frame(1);
        adapter.push_frame(1);
        let adapter = adapter.with_frame_pixels(480, 240, gradient_frame(480, 240));
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut app =
            App::launch("expect_shot_timeout_base", adapter).with_baseline_dir(dir.path());
        app.wait_for_settle();
        (app, dir)
    };
    baseline
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline");

    // The animation never settles: the tamper sticks. The retry loop
    // polls to the timeout and fails with the assertion header, the
    // last mismatch report and the artifact links (§10 deep links).
    let mut app = animated_app(
        "expect_shot_timeout_anim",
        dir.path(),
        vec![tampered_frame(480, 240)],
    );
    let err = guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .with_timeout(Duration::from_millis(300))
        .to_have_screenshot("save", BlessMode::Off)
        .expect_err("the pixels never settle inside the timeout");
    let msg = err.to_string();
    assert!(msg.contains("screenshot"), "assertion header, got: {msg}");
    assert!(
        msg.contains("get_by_role") || msg.contains("Save"),
        "the header names the locator, got: {msg}"
    );
    assert!(
        msg.contains(".new.png") && msg.contains(".diff.png"),
        "the artifact links ride the timeout report (§10), got: {msg}"
    );
    // The artifacts landed next to the baseline — the LAST attempt's.
    let names: Vec<String> = std::fs::read_dir(dir.path().join("save"))
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
fn screenshot_retry_blesses_a_missing_baseline_through_the_store_flow() {
    // No baseline yet: with `BlessMode::Failing` the store's missing
    // baseline path writes it on the first poll and the budget counts
    // as satisfied — the same §10 flow the single-shot rides.
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut app = settled_app("expect_shot_bless", dir.path());
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_have_screenshot("fresh", BlessMode::Failing)
        .expect("the first poll blesses the missing baseline");
    let baseline_path =
        guiwright_shot::baseline::BaselineStore::new(dir.path()).baseline_path("fresh", "fixture");
    assert!(
        baseline_path.exists(),
        "baseline written: {baseline_path:?}"
    );
}

#[test]
fn negated_screenshot_retry_passes_while_the_pixels_differ() {
    let (mut baseline, dir) = {
        let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
        adapter.push_frame(1);
        adapter.push_frame(1);
        let adapter = adapter.with_frame_pixels(480, 240, gradient_frame(480, 240));
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut app =
            App::launch("expect_shot_negated_base", adapter).with_baseline_dir(dir.path());
        app.wait_for_settle();
        (app, dir)
    };
    baseline
        .get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline");

    // `not().to_have_screenshot` (the M24 modifier over the §10 flow):
    // satisfied exactly while the positive condition does not hold —
    // the tampered app never matches, so the inversion passes on the
    // first poll.
    let mut app = animated_app(
        "expect_shot_negated_anim",
        dir.path(),
        vec![tampered_frame(480, 240)],
    );
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .not()
        .to_have_screenshot("save", BlessMode::Off)
        .expect("the pixels differ — the negated assertion holds");
}

#[test]
fn screenshot_retry_never_polls_when_the_first_frame_matches() {
    // Zero-polling (§15.3): the initial evaluation settles the budget
    // before any retry pump. The one pump that runs is the session
    // resolve's own consistency pump (`Session::resolve` pumps first by
    // contract) — the retry loop itself adds nothing: no second render,
    // no mismatch artifacts written next to the baseline.
    let dir = tempfile::tempdir().expect("tmpdir");
    let mut app = settled_app("expect_shot_nopoll", dir.path());
    app.get_by_role(Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline");
    let frames_before = app.session().frame_seq();
    guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
        .to_have_screenshot("save", BlessMode::Off)
        .expect("first-frame match");
    assert_eq!(
        app.session().frame_seq(),
        frames_before + 1,
        "exactly the resolve consistency pump — the retry loop never pumped"
    );
    let names: Vec<String> = std::fs::read_dir(dir.path().join("save"))
        .expect("baseline dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !names.iter().any(|n| n.ends_with(".new.png")),
        "a first-frame match writes no mismatch artifacts, have {names:?}"
    );
}
