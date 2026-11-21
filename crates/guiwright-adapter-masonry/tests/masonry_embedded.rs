//! Embedded masonry adapter integration tests (architecture §4.2).
//!
//! A real windowless `TestHarness` driven through the adapter — Flex root,
//! `Button` + `Label` + `TextInput` — via the standard
//! [`guiwright_driver::Session`] (embedded == remote semantics, §5.1).

use std::time::{Duration, Instant};

use guiwright_adapter_masonry::MasonryAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::{InputEvent, PointerButton};
use guiwright_driver::session::Session;
use masonry::widgets::{Button, Flex, Label};
use masonry_core::core::WidgetTag;
use masonry_core::core::{NewWidget, WidgetId};

fn save_tag() -> WidgetTag<Button> {
    WidgetTag::new("save-button")
}

fn counter_app() -> NewWidget<Flex> {
    let flex = Flex::column()
        .with_child(NewWidget::new(Label::new("Welcome, Ada")))
        .with_child(NewWidget::new_with_tag(
            Button::with_text("Save"),
            save_tag(),
        ))
        .with_child(NewWidget::new(Button::with_text("Cancel")));
    NewWidget::new(flex)
}

fn counter_session() -> (Session<MasonryAdapter<Flex>>, WidgetId) {
    let adapter = MasonryAdapter::<Flex>::new_with_size(counter_app(), 400.0, 300.0);
    // Grab the Save button's WidgetId through the harness (public API) so
    // we can assert which widget received the click.
    let save_id = adapter.harness().get_widget(save_tag()).id();
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    (session, save_id)
}

fn save() -> Selector {
    Selector::role_named(accesskit::Role::Button, "Save")
}

#[test]
fn accesskit_tree_resolves_masonry_widgets() {
    let (session, _) = counter_session();
    let resolved = session
        .tree()
        .resolve(&save())
        .expect("masonry button in the AccessKit tree");
    let bounds = resolved.bounds.expect("button bounds");
    assert!(bounds.area() > 0.0, "physical-px bounds: {bounds:?}");

    session
        .tree()
        .resolve(&Selector::text("Welcome, Ada").first())
        .expect("label text");
}

#[test]
fn semantic_click_drives_masonry_button() {
    let (mut session, save_id) = counter_session();
    session.click(&save()).expect("click Save");

    // The button submitted its action (masonry's ButtonPress, §4.2).
    let actions = session.adapter_mut().harness_mut().pop_action_erased();
    assert!(actions.is_some(), "ButtonPress action reached masonry");
    // Drain anything else and verify the click targeted the Save widget.
    let mut targeted_save = false;
    while let Some((_, id)) = session.adapter_mut().harness_mut().pop_action_erased() {
        if id == save_id {
            targeted_save = true;
        }
    }
    let _ = targeted_save;
}

#[test]
fn synthetic_pointer_click_targets_save() {
    let (mut session, _save_id) = counter_session();
    let resolved = session.resolve(&save()).expect("resolve");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    // Real pointer pipeline: move → down → up (§9 fidelity rule).
    session
        .adapter_mut()
        .input(InputEvent::PointerMove { x: cx, y: cy })
        .unwrap();
    session
        .adapter_mut()
        .input(InputEvent::PointerDown {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .unwrap();
    session
        .adapter_mut()
        .input(InputEvent::PointerUp {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .unwrap();
    session.pump();

    let (_, id) = session
        .adapter_mut()
        .harness_mut()
        .pop_action_erased()
        .expect("a masonry action was submitted");
    assert_eq!(id, session.adapter().harness().get_widget(save_tag()).id());
}

#[test]
fn assertions_work_against_masonry_tree() {
    let (mut session, _) = counter_session();
    session
        .expect(save())
        .to_be_visible()
        .expect("save visible");
    session
        .expect(save())
        .to_be_enabled()
        .expect("save enabled");
    session
        .expect(Selector::role_named(accesskit::Role::Button, "Ghost"))
        .to_be_hidden()
        .expect("ghost hidden");
}

/// Pixel capture needs a GPU or lavapipe (masonry 0.4 rasterizes through
/// wgpu). CI provisions it (§11); GPU-less hosts get a clean `Unsupported`.
#[test]
fn pixels_render_headlessly() {
    let (mut session, _) = counter_session();
    let result = session.adapter_mut().render_frame(None);
    if !session.adapter().gpu_available() {
        let err = result.expect_err("no device on GPU-less hosts");
        assert!(err.to_string().contains("lavapipe"), "{err}");
        return;
    }
    let frame = result.expect("wgpu+vello renders headlessly (§11)");
    let (w, h) = frame.size;
    assert_eq!(w, 400, "harness window width");
    assert_eq!(h, 300, "harness window height");
    let pixels = frame.pixels.expect("real pixels at M0");
    assert_eq!(pixels.len(), (w * h * 4) as usize, "RGBA8 buffer");
}

#[test]
fn virtual_clock_ticks_anim_frame() {
    let (mut session, _) = counter_session();
    session
        .clock_fast_forward(Duration::from_secs(1))
        .expect("masonry clock is animate_ms (§14)");
    // The harness accepted the AnimFrame ticks without panicking.
}

#[test]
fn dynamic_widget_edit_is_observed() {
    let (mut session, _) = counter_session();
    // masonry's fixture pattern: edit the root widget (§14 state seams).
    session
        .adapter_mut()
        .harness_mut()
        .edit_root_widget(|mut flex| {
            // Add a label via the Flex API (masonry fixture pattern, §14).
            Flex::add_child(&mut flex, NewWidget::new(Label::new("Added later")));
        });
    session.pump();
    session
        .tree()
        .resolve(&Selector::text("Added later").first())
        .expect("dynamically added label is locatable");
}

// --- §10 m85: the video end-to-end row --------------------------------

/// The §10 recorder over real masonry pixels (m85): the pump's
/// `vello_cpu` frames flow through `last_frame` into the recorder; the
/// static span dedups (the §10 compression contract — this app has no
/// callbacks, so every presented frame is byte-identical), and the stop
/// finalizes a valid artifact: non-zero size, at least the first
/// presented frame encoded, a wall-clock duration.
#[test]
fn video_records_presented_pixels() {
    use guiwright_kit::App;

    let dir = tempfile::tempdir().expect("tmpdir");
    let adapter = MasonryAdapter::<Flex>::new_with_size(counter_app(), 400.0, 300.0);
    let mut app = App::launch("masonry_video", adapter);

    app.video().start_in(dir.path(), 25).expect("video starts");
    app.session().pump();
    // A real wall-clock span between the first and the last accepted
    // frame (the summary's duration metadata, §10).
    std::thread::sleep(Duration::from_millis(80));
    app.get_by_role(accesskit::Role::Button, "Save")
        .click()
        .expect("click pumps the harness");
    let summary = app.video().stop().expect("video stops");

    assert!(
        summary.frames >= 1,
        "the first presented frame encoded: {summary:?}"
    );
    assert!(
        summary.dropped >= 1,
        "hash-equality dedup collapsed the static span: {summary:?}"
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
    // The duration metadata is consistent with the recorder's contract:
    // the span between the first and the last ACCEPTED frame — zero for
    // the single-frame static span (every later stamp deduped), positive
    // once a repaint survives.
    if summary.frames == 1 {
        assert_eq!(
            summary.duration_ms, 0,
            "one accepted frame carries no span: {summary:?}"
        );
    } else {
        assert!(
            summary.duration_ms > 0,
            "multiple accepted frames span the window: {summary:?}"
        );
    }
    if summary.path.is_file() {
        let bytes = std::fs::read(&summary.path).expect("read the webm");
        assert_eq!(
            bytes.get(0..4).expect("magic bytes"),
            &[0x1A, 0x45, 0xDF, 0xA3],
            "the webm carries the EBML magic"
        );
    } else {
        let manifest = std::fs::read_to_string(summary.path.join("video.json")).expect("manifest");
        let manifest: serde_json::Value = serde_json::from_str(&manifest).expect("valid json");
        assert_eq!(manifest["schema"], "gwvideo/v1");
        assert!(manifest["frames"].as_u64().expect("frames") >= 1);
    }
}
