//! Embedded freya adapter integration tests (architecture §4.3, §17 row 3,
//! m79).
//!
//! A real `freya_testing::TestingRunner` running a real freya app —
//! counter + form with stock `rect` press targets, `Label` texts and the
//! freya `Input` editor, all carrying explicit
//! `.a11y_id(AccessibilityId(n))` selectors (§4.3: "stable selectors
//! require apps to set `.a11y_id`") — driven through the standard
//! [`guiwright_driver::Session`]. The committed AccessKit `TreeUpdate`s
//! are the Level-1 diffs; input rides the frame-quantized
//! press→sync→release pipeline (`send_event` + `sync_and_update`).
//!
//! The freya stack (software Skia) compiles only where a C++ toolchain
//! exists — locally the default build stays framework-free; this leg is
//! runtime-tested by the CI `adapter-freya` job (m79 upgraded it from
//! `cargo check`).

#![cfg(feature = "freya")]

use std::time::{Duration, Instant};

use freya_components::input::Input;
use freya_testing::prelude::*;
use guiwright_adapter_freya::runner::{FreyaAdapter, FreyaTestingHarness};
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::InputEvent;
use guiwright_driver::session::Session;
// freya 0.5: element sizes ride torin's `Size` (`Size::px` — floats no
// longer convert implicitly).
use torin::prelude::Size;

/// The app state the hook provides and the test observes. `mounted` is
/// the mount gate (§9 quirk): freya 0.5's `TestingRunner` commits the
/// boot tree internally — the consumer would never see it — so the app
/// mounts its content one pass after boot (the harness flips `mounted`
/// before the session's first sync), which turns the first post-boot
/// diff into the full tree.
#[derive(Clone)]
struct AppState {
    count: State<i64>,
    name: State<String>,
    mounted: State<bool>,
}

/// The §4.3 app contract: explicit `.a11y_id`s (stable selectors),
/// `a11y_role` on the press targets, texts as `Label`s — root id 0
/// (the AccessKit root the commit declares).
fn counter_form() -> Element {
    let state = use_consume::<AppState>();
    if !*state.mounted.read() {
        return rect()
            .width(Size::px(480.))
            .height(Size::px(320.))
            .a11y_id(AccessibilityId(0))
            .into();
    }
    let mut count = state.count.clone();
    let name = state.name.clone();
    let name_input = name.clone();

    rect()
        .width(Size::px(480.))
        .height(Size::px(320.))
        .a11y_id(AccessibilityId(0))
        .child(
            // freya 0.5 aggregates the first value-carrying child's text
            // into the container's a11y name — this marker keeps the root
            // from colliding with the dynamic count/greeting texts (§9).
            label().text("Guiwright demo").a11y_id(AccessibilityId(100)),
        )
        .child(
            label()
                .text(format!("Count: {}", count.read()))
                .a11y_id(AccessibilityId(101)),
        )
        .child(
            rect()
                .width(Size::px(140.))
                .height(Size::px(40.))
                .background(Color::from_rgb(200, 220, 255))
                .child("Increment")
                .on_press(move |_| *count.write() += 1)
                .a11y_role(AccessibilityRole::Button)
                .a11y_id(AccessibilityId(102)),
        )
        .child(
            rect()
                .width(Size::px(140.))
                .height(Size::px(40.))
                .background(Color::from_rgb(255, 220, 200))
                .child("Decrement")
                .on_press(move |_| *count.write() -= 1)
                .a11y_role(AccessibilityRole::Button)
                .a11y_id(AccessibilityId(103)),
        )
        .child(
            Input::new(name_input)
                .placeholder("Type a name")
                .a11y_id(AccessibilityId(104)),
        )
        .child(
            label()
                .text(if name.read().is_empty() {
                    "Welcome, nobody".to_string()
                } else {
                    format!("Welcome, {}", name.read())
                })
                .a11y_id(AccessibilityId(105)),
        )
        .into()
}

fn counter_session() -> Session<FreyaAdapter<FreyaTestingHarness<AppState>>> {
    let harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
        // The context the app consumes IS the AppState (freya 0.5
        // registers contexts by type — `use_consume::<AppState>()`).
        runner.provide_root_context(|| AppState {
            count: State::create(0i64),
            name: State::create(String::new()),
            mounted: State::create(false),
        })
    });
    // The mount flip lands after freya's internal boot commit and before
    // the session's first sync — the next diff IS the full tree (§9).
    let mut mounted = harness.state().mounted.clone();
    *mounted.write() = true;
    let adapter = FreyaAdapter::new(harness);
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    // freya is Level-1: the app's `.a11y_id(n)` numbers ARE the AccessKit
    // NodeIds — the §8.1 test-id registry rides an explicit registration
    // (there is no auto test-id path for Level-1 trees).
    for n in [100u64, 101, 102, 103, 104, 105] {
        session.register_test_id(
            guiwright_core::NodeKey::AccessKit {
                tree: accesskit::TreeId::ROOT,
                node: accesskit::NodeId(n),
            },
            n.to_string(),
        );
    }
    session
}

fn increment() -> Selector {
    Selector::test_id("102")
}

fn name_field() -> Selector {
    Selector::test_id("104")
}

#[test]
fn tree_update_commits_the_a11y_ids() {
    let mut session = counter_session();
    // The committed diff carries the app-assigned ids (§4.3 a11y_id
    // contract) with laid-out bounds.
    session.resolve(&increment()).expect("increment button");
    session.resolve(&name_field()).expect("name input");
    session
        .resolve(&Selector::text("Count: 0"))
        .expect("counter label reflects boot state");
}

#[test]
fn semantic_click_drives_the_real_freya_counter() {
    let mut session = counter_session();

    session.click(&increment()).expect("click Increment");

    // The on_press handler ran through the real ragnarok pipeline.
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");
}

#[test]
fn fill_types_through_the_real_input() {
    let mut session = counter_session();

    session
        .fill(&name_field(), "Ada")
        .expect("fill the name input");

    // The two-way state binding updated the greeting label.
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("greeting reflects the typed name");
}

/// The §13 inspector overlay (m8 promise): with the session on, the
/// adapter composites the hover outline + role badges onto the runner's
/// own software-Skia rendering — the rasterized pixels differ from the
/// bare app's baseline (M83).
#[test]
fn overlay_pixel_diff_is_non_zero() {
    let mut session = counter_session();

    // Baseline: the bare app, no overlay session (§10 capture path).
    let baseline = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render");
    let base = baseline.pixels.expect("software Skia pixels");

    // The overlay session: badges on interactive widgets + the hover
    // outline on the resolved Increment button.
    session
        .inspect_enable_overlay()
        .expect("overlay session on");
    session
        .inspect_highlight(&increment())
        .expect("highlight resolves");

    let overlaid = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render with the overlay");
    let over = overlaid.pixels.expect("software Skia pixels");

    assert_eq!(base.len(), over.len(), "same viewport");
    let diff = base.iter().zip(over.iter()).filter(|(a, b)| a != b).count();
    assert!(diff > 0, "the overlay painted onto the scene: {diff} bytes");
    assert!(
        overlaid.stamp.hash != baseline.stamp.hash,
        "frame digest moved"
    );
}

#[test]
fn raw_input_moves_the_pointer_and_clicks() {
    let mut session = counter_session();

    // The §9 raw-injection path: the InputEvent → PlatformEvent
    // translation rides the same frame-quantized pipeline.
    session
        .adapter_mut()
        .input(InputEvent::PointerMove { x: 70.0, y: 38.0 })
        .expect("move");
    session
        .adapter_mut()
        .input(InputEvent::PointerDown {
            x: 70.0,
            y: 38.0,
            button: guiwright_driver::adapter::PointerButton::Primary,
        })
        .expect("down");
    session
        .adapter_mut()
        .input(InputEvent::PointerUp {
            x: 70.0,
            y: 38.0,
            button: guiwright_driver::adapter::PointerButton::Primary,
        })
        .expect("up");
    session.settle(Instant::now() + Duration::from_secs(1));

    session
        .resolve(&Selector::text("Count: 1"))
        .expect("raw pointer pipeline incremented the counter");
}

#[test]
fn render_frame_produces_real_pixels() {
    let mut session = counter_session();

    let frame = session.adapter_mut().render_frame(None).expect("render");
    assert_eq!(frame.size, (480, 320), "the boot viewport");
    let pixels = frame.pixels.expect("the skia rasterizer produced pixels");
    assert!(!pixels.is_empty());
    assert!(
        pixels.chunks_exact(4).any(|px| px[3] != 0),
        "the snapshot has painted pixels"
    );
}

#[test]
fn key_events_carry_the_full_sequence() {
    let mut session = counter_session();

    // §4.3 gap: modifiers AND KeyUp ride the raw Keyboard pipeline.
    session
        .adapter_mut()
        .input(InputEvent::KeyDown {
            key: "Escape".into(),
            modifiers: vec!["Shift".into()],
        })
        .expect("key down with modifier");
    session
        .adapter_mut()
        .input(InputEvent::KeyUp {
            key: "Escape".into(),
            modifiers: vec!["Shift".into()],
        })
        .expect("key up with modifier");
}

/// The §14 state-seam end-to-end (the gpui glue's `run_in` shape): a
/// compiled `SeamTask` runs inside the runner context — the hook's
/// `AppState` — writes the count through the state handle, and the next
/// committed diff carries the re-rendered label. State mutation with no
/// input event at all.
#[test]
fn run_in_mutates_app_state_end_to_end() {
    let mut session = counter_session();

    session
        .adapter_mut()
        .run_in(Box::new(|ctx: &mut dyn std::any::Any| {
            let state = ctx
                .downcast_mut::<AppState>()
                .ok_or("wrong runner context type")?;
            let mut count = state.count.clone();
            *count.write() = 42;
            Ok("count set to 42".to_string())
        }))
        .expect("seam executed in the runner context");

    // The write re-rendered on the next committed diff.
    session.settle(Instant::now() + Duration::from_secs(1));
    session
        .resolve(&Selector::text("Count: 42"))
        .expect("run_in mutation re-rendered");
}

#[test]
fn probe_mount_update() {
    let mut harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
        runner.provide_root_context(|| AppState {
            count: State::create(0i64),
            name: State::create(String::new()),
            mounted: State::create(false),
        })
    });
    let mut mounted = harness.state().mounted.clone();
    *mounted.write() = true;
    let u1 = guiwright_adapter_freya::runner::TestingSeam::sync_and_update(&mut harness);
    eprintln!("MOUNT nodes={}", u1.nodes.len());
    for (id, node) in &u1.nodes {
        let kids: Vec<i64> = node.children().iter().map(|c| c.0 as i64).collect();
        eprintln!("  id={} role={:?} kids={:?}", id.0, node.role(), kids);
    }
    let u2 = guiwright_adapter_freya::runner::TestingSeam::sync_and_update(&mut harness);
    eprintln!("POST nodes={}", u2.nodes.len());
    for (id, node) in &u2.nodes {
        eprintln!("  id={} role={:?}", id.0, node.role());
    }
}

// --- §10 m86: the video + visual-regression rows -----------------------

/// The §4.3 test-id registration the kit-driven sessions need too: freya
/// is Level-1 — "there is no auto test-id path for Level-1 trees" — so
/// the App-launched sessions register the app's `.a11y_id` numbers the
/// same way `counter_session` does.
fn register_app_test_ids(
    app: &mut guiwright_kit::App<FreyaAdapter<FreyaTestingHarness<AppState>>>,
) {
    for n in [100u64, 101, 102, 103, 104, 105] {
        app.session().register_test_id(
            guiwright_core::NodeKey::AccessKit {
                tree: accesskit::TreeId::ROOT,
                node: accesskit::NodeId(n),
            },
            n.to_string(),
        );
    }
}

/// The §10 recorder over real freya pixels (m86): the pump's frames flow
/// through `last_frame` into the recorder — the TestingRunner's software
/// Skia raster is the pixel path (§6.2 "last_frame ✅ post_render_callback"),
/// the fill repaint records, and the stop finalizes a valid artifact:
/// non-zero size, ≥2 distinct frames around the repaint, a wall-clock
/// duration, and a real container (ffmpeg EBML when the binary is
/// present, the PNG frame folder + `video.json` manifest otherwise).
///
/// The recorder's hash-equality dedup rides the pump's frame stamps;
/// freya's stamps carry the pass sequence as their digest (runner.rs —
/// "TestingRunner renders deterministically; the pixel digest rides
/// render_frame"), so no static-span dedup is asserted here — every pump
/// presents exactly one stamp and the two pumps around the repaint are
/// the two distinct frames.
#[test]
fn video_records_presented_pixels() {
    use guiwright_kit::App;

    let dir = tempfile::tempdir().expect("tmpdir");
    let harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
        runner.provide_root_context(|| AppState {
            count: State::create(0i64),
            name: State::create(String::new()),
            mounted: State::create(false),
        })
    });
    let mut mounted = harness.state().mounted.clone();
    *mounted.write() = true;
    let adapter = FreyaAdapter::new(harness);
    let mut app = App::launch("freya_video", adapter);
    register_app_test_ids(&mut app);

    app.video().start_in(dir.path(), 25).expect("video starts");
    app.session().pump();
    // A real wall-clock span between the first and the last accepted
    // frame (the summary's duration metadata, §10).
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.get_by_test_id("104")
        .fill("Ada")
        .expect("fill repaints");
    let summary = app.video().stop().expect("video stops");

    assert!(
        summary.frames >= 2,
        "the static span and the repaint both recorded: {summary:?}"
    );
    assert!(
        summary.path.exists(),
        "finalized at {}",
        summary.path.display()
    );
    let size = std::fs::metadata(&summary.path)
        .expect("artifact exists")
        .len();
    assert!(size > 0, "the artifact is not empty ({size} bytes)");
    assert!(
        summary.duration_ms >= 40,
        "the duration metadata spans the recorded window: {summary:?}"
    );
    is_valid_video(&summary.path);
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

/// The §10 visual-regression pipeline over real freya pixels (m86): the
/// settled app blesses an element baseline (`BlessMode::Failing` — the
/// `GUIWRIGHT_BLESS=1` form), and the unchanged re-render diffs clean
/// (`BlessMode::Off`) — the §10 bless → diff round trip end to end, from
/// the TestingRunner's software-Skia raster through the crop-aware
/// element screenshot into the per-platform baseline store.
#[test]
fn screenshot_baseline_bless_and_diff_roundtrip() {
    use guiwright_kit::{App, BlessMode, SnapshotOutcome};

    let dir = tempfile::tempdir().expect("tmpdir");

    let boot = || {
        let harness = FreyaTestingHarness::boot(counter_form, (480.0, 320.0), |runner| {
            runner.provide_root_context(|| AppState {
                count: State::create(0i64),
                name: State::create(String::new()),
                mounted: State::create(false),
            })
        });
        let mut mounted = harness.state().mounted.clone();
        *mounted.write() = true;
        FreyaAdapter::new(harness)
    };

    // Bless: the first run writes the baseline (the missing-baseline
    // branch of the §10 flow).
    let blessed = {
        let mut app = App::launch("freya_bless", boot()).with_baseline_dir(dir.path());
        register_app_test_ids(&mut app);
        app.get_by_test_id("102")
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

    // Diff: the unchanged re-render matches within the pixel budget.
    let mut app = App::launch("freya_diff", boot()).with_baseline_dir(dir.path());
    register_app_test_ids(&mut app);
    let matched = app
        .get_by_test_id("102")
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
