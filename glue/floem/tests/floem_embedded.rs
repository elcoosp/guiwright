//! Embedded floem adapter integration tests (architecture §4.7, §17 row
//! 6, m80).
//!
//! A real `lapce/floem` `HeadlessHarness` running a real floem counter
//! +form app — stock `button` + `text_input` + derived labels — driven
//! through the standard [`guiwright_driver::Session`] (embedded ==
//! remote semantics, §5.1). Interactions go through the harness's real
//! event pipeline (`dispatch_event`, `pointer_down/up/move`, real
//! `KeyboardEvent`s) with real hit-testing (`element_id_at`), and the
//! tree folds real `ViewId::get_visual_rect` geometry through the
//! engine's `FloemLogicalTree`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::Role;
use floem::view::IntoView;
use floem::views::{Button, Decorators, Label, Stack, TextInput};
use floem_reactive::{RwSignal, SignalGet, SignalUpdate};

use guiwright_adapter_floem_glue::harness::{FloemAdapter, SemReader};
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::{InputEvent, PointerButton};
use guiwright_driver::session::Session;

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

/// Build the real view tree. Views carry their §8.1 test ids through
/// floem's native `debug_name` decorator (the walk feeds the engine's
/// [`guiwright_adapter_floem::TestIdRegistry`] from it).
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
        // The two labels the §11 floem row's pixel/pump assertions read.
        Label::derived(move || format!("Count: {}", count.get())).debug_name("count-label"),
        Label::derived(move || format!("Welcome, {}", name.get())).debug_name("greeting"),
    ))
    .style(|s| s.flex_col().items_center())
}

/// The app glue: text/value facts are the app's **real reactive state**
/// (the same signals the views render from) — the documented §4.7
/// contribution seam (upstream patch 1b would add text getters on core
/// views; until it lands, the app declares its readers).
fn counter_readers(app: &Rc<CounterForm>) -> Vec<SemReader> {
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

fn counter_session() -> Session<FloemAdapter> {
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
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    session
}

fn increment() -> Selector {
    Selector::role_named(Role::Button, "Increment")
}

fn decrement() -> Selector {
    Selector::role_named(Role::Button, "Decrement")
}

fn name_input() -> Selector {
    Selector::role(Role::TextInput)
}

#[test]
fn logical_tree_resolves_floem_views_with_bounds() {
    let mut session = counter_session();

    let resolved = session
        .resolve(&increment())
        .expect("floem button in the logical tree");
    let bounds = resolved.bounds.expect("button bounds");
    assert!(bounds.area() > 0.0, "logical-px bounds: {bounds:?}");

    session
        .resolve(&decrement())
        .expect("second button resolves");
    session.resolve(&name_input()).expect("text input resolves");

    // The initial counter label reflects the boot state.
    session
        .resolve(&Selector::text("Count: 0"))
        .expect("counter label reflects boot state");
}

#[test]
fn semantic_click_drives_the_real_floem_counter() {
    let mut session = counter_session();

    session.click(&increment()).expect("click Increment");

    // The `action` closure flowed through the real harness event
    // pipeline (pointer down → up → click routing).
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");

    session.click(&decrement()).expect("click Decrement");
    session.click(&decrement()).expect("click Decrement again");
    session
        .resolve(&Selector::text("Count: -1"))
        .expect("counter decremented twice");
}

#[test]
fn fill_reports_the_upstream_headless_ime_blocker() {
    let mut session = counter_session();

    // The form's TextInput resolves with real bounds (tree/input are
    // real — §4.7's non-negotiable).
    let resolved = session.resolve(&name_input()).expect("text input resolves");
    let bounds = resolved.bounds.expect("input bounds");
    assert!(bounds.area() > 0.0, "logical-px bounds: {bounds:?}");

    // `fill` is the one documented upstream blocker (m80): floem's
    // text_input requests IME the moment it gains focus
    // (`src/views/text_input.rs:1403`), the headless `MockWindow`
    // rejects every IME request (`src/window/mock.rs:262`,
    // `Err(ImeRequestError::NotSupported)`) and
    // `src/window/handle.rs:1270` unwraps that into a panic — floem's
    // own 40-file harness suite never focuses a text_input. The
    // adapter gates before the focus click and reports the capability
    // row honestly instead of ever panicking or mocking.
    let err = session
        .fill(&name_input(), "Ada")
        .expect_err("fill is the documented upstream blocker");
    assert!(
        err.to_string().contains("NotSupported"),
        "the error carries the file-level evidence: {err}"
    );

    // The core loop keeps working around the blocked cell: the counter
    // still clicks through the real pipeline.
    session.click(&increment()).expect("click Increment");
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("counter incremented");
}

#[test]
fn synthetic_pointer_click_targets_the_button() {
    let mut session = counter_session();

    let resolved = session.resolve(&increment()).expect("resolve");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    // Real pointer pipeline: move → down → up (§9 fidelity rule) — the
    // harness's `pointer_move/pointer_down/pointer_up`, which route
    // `Event::Pointer` through the same dispatch the live window uses.
    session
        .adapter_mut()
        .input(InputEvent::PointerMove { x: cx, y: cy })
        .expect("move");
    session
        .adapter_mut()
        .input(InputEvent::PointerDown {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .expect("down");
    session
        .adapter_mut()
        .input(InputEvent::PointerUp {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .expect("up");
    session.settle(Instant::now() + Duration::from_secs(1));

    session
        .resolve(&Selector::text("Count: 1"))
        .expect("pointer pipeline incremented the counter");
}

#[test]
fn logical_snapshot_is_stable_across_pumps() {
    let mut session = counter_session();
    let first = session
        .adapter_mut()
        .logical_snapshot()
        .expect("logical diff")
        .update
        .nodes
        .len();
    let second = session
        .adapter_mut()
        .logical_snapshot()
        .expect("logical diff")
        .update
        .nodes
        .len();
    assert_eq!(first, second, "tree size stable across re-snapshots");
    assert!(first > 5, "the full view tree is in the snapshot");
}

#[test]
fn virtual_clock_ticks_the_harness_update_queue() {
    let mut session = counter_session();

    session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::InstallVirtual)
        .expect("install");
    let state = session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::FastForward(
            Duration::from_millis(1_500),
        ))
        .expect("fast forward");
    let virtual_time = state.virtual_time.expect("virtual clock installed");
    assert!(
        virtual_time >= Duration::from_millis(1_500),
        "harness ticks advanced virtual time: {virtual_time:?}"
    );
}

#[test]
fn render_frame_rasterizes_real_view_geometry() {
    let mut session = counter_session();

    let frame = session
        .adapter_mut()
        .render_frame(Some((240, 180)))
        .expect("headless render");
    assert_eq!(frame.size, (240, 180), "raster at the requested size");
    let pixels = frame
        .pixels
        .expect("the tiny-skia PaintState produced pixels");
    assert!(!pixels.is_empty());
    // Not a blank canvas: the walk painted the laid-out view rects.
    assert!(
        pixels.as_chunks::<4>().0.iter().any(|px| px[3] != 0),
        "the snapshot has painted pixels"
    );
}

#[test]
fn hit_test_maps_a_point_to_the_walked_node() {
    let mut session = counter_session();

    let resolved = session.resolve(&increment()).expect("resolve");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    // The framework's own hit test agrees with the walked geometry: the
    // point inside the button's visual rect resolves to the same view.
    session
        .adapter_mut()
        .assert_hit(cx, cy, "increment")
        .expect("element_id_at resolves to the increment view");
}
