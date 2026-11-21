//! Embedded iced adapter integration tests (architecture §4.6, §17 MVP trio).
//!
//! A real `iced_test::Emulator` running a real iced `Program` — counter
//! buttons + a text input, all stock iced widgets — driven through the
//! standard [`guiwright_driver::Session`] (embedded == remote semantics,
//! §5.1). The tree comes from the §4.6 read-back (`Task::widget` operation
//! shipping the widget walk over an mpsc channel); clicks and fills ride
//! the real `UserInterface::update` event pipeline the `Emulator`
//! executes per instruction.

#![cfg(feature = "iced")]

use std::time::{Duration, Instant};

use guiwright_adapter_iced::RoleRegistry;
use guiwright_adapter_iced::emulator::{EmulatorHarness, HarnessApp, SharedHarness, TreeDumpTx};
use guiwright_adapter_iced::harness::IcedAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::{InputEvent, PointerButton};
use guiwright_driver::session::Session;
use iced::widget::{button, column, container, text, text_input};
use iced_test::core::widget::Id;
use iced_test::core::{Element, Settings, Theme};
use iced_test::program::Program;
use iced_test::renderer;
use iced_test::runtime::Task;

/// The app state (`Program::State` — the shell [`CounterForm`] is the
/// immutable program config).
struct FormState {
    count: i64,
    name: String,
    greeting: String,
}

/// The demo app: the §17 MVP-trio shape (counter + form), stock widgets.
struct CounterForm;

/// The app's message vocabulary, including the guiwright read-back hook.
#[derive(Debug, Clone)]
enum Msg {
    Increment,
    Decrement,
    NameChanged(String),
    Hook(TreeDumpTx),
}

impl Program for CounterForm {
    type State = FormState;
    type Message = Msg;
    type Theme = Theme;
    type Renderer = renderer::Renderer;
    type Executor = iced_test::futures::backend::default::Executor;

    fn name() -> &'static str {
        "guiwright_counter_form"
    }

    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn window(&self) -> Option<iced_test::core::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            FormState {
                count: 0,
                name: String::new(),
                greeting: "Welcome, nobody".into(),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            Msg::Increment => state.count += 1,
            Msg::Decrement => state.count -= 1,
            Msg::NameChanged(name) => {
                state.name = name;
                state.greeting = format!("Welcome, {}", state.name);
            }
            // The one guiwright hook arm (§4.6 read-back): the default
            // run_hook returns Task::widget(TreeDumpOp) — the app only
            // routes the hook message.
            Msg::Hook(tx) => {
                return <CounterForm as HarnessApp>::run_hook(state, tx);
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced_test::core::window::Id,
    ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        column![
            text(format!("Count: {}", state.count)),
            container(button("Increment").on_press(Msg::Increment)).id(Id::new("increment")),
            container(button("Decrement").on_press(Msg::Decrement)).id(Id::new("decrement")),
            text_input("Type a name", &state.name)
                .on_input(Msg::NameChanged)
                .id(Id::new("name")),
            text(&state.greeting),
        ]
        .into()
    }
}

impl HarnessApp for CounterForm {
    fn hook(tx: TreeDumpTx) -> Self::Message {
        Msg::Hook(tx)
    }

    // run_hook: the trait default — Task::widget(TreeDumpOp::<CounterForm>).

    // kind_of: the app's semantic contribution (§4.6 role registry). The
    // interactive widgets are Id-carrying containers around stock iced
    // buttons (iced 0.14 passes a button's Id to no operation hook).
    fn kind_of(id: &Id) -> Option<&'static str> {
        for name in ["increment", "decrement"] {
            if *id == Id::new(name) {
                return Some("button");
            }
        }
        None
    }
}

fn counter_session() -> Session<IcedAdapter<SharedHarness<CounterForm>>> {
    let harness = EmulatorHarness::new(CounterForm, (480.0, 320.0)).expect("harness boots");
    let adapter = guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    session
}

fn increment() -> Selector {
    Selector::role_named(accesskit::Role::Button, "Increment")
}

fn decrement() -> Selector {
    Selector::role_named(accesskit::Role::Button, "Decrement")
}

fn name_input() -> Selector {
    Selector::role(accesskit::Role::TextInput)
}

#[test]
fn logical_tree_resolves_iced_widgets_with_bounds() {
    let mut session = counter_session();

    let resolved = session
        .resolve(&increment())
        .expect("iced button in the logical tree");
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
fn semantic_click_drives_the_real_iced_counter() {
    let mut session = counter_session();

    session.click(&increment()).expect("click Increment");

    // The message flowed through the real update: the tree reflects it.
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
fn fill_types_through_the_real_text_input() {
    let mut session = counter_session();

    session
        .fill(&name_input(), "Ada")
        .expect("fill the name input");

    // on_input fired per keystroke: the greeting label follows the state.
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("greeting reflects the typed name");
}

#[test]
fn synthetic_pointer_click_targets_the_button() {
    let mut session = counter_session();

    let resolved = session.resolve(&increment()).expect("resolve");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    // Real pointer pipeline: move → down → up (§9 fidelity rule).
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
    assert!(first > 5, "the full widget tree is in the snapshot");
}

/// The §13 inspector overlay (m8 promise): with the session on, the
/// adapter composites the hover outline + role badges onto the Emulator's
/// own headless rendering — the screenshot pixels differ from the bare
/// app's baseline (M83).
#[test]
fn overlay_pixel_diff_is_non_zero() {
    let mut session = counter_session();

    // Baseline: the bare app, no overlay session (§10 capture path).
    let baseline = session
        .adapter_mut()
        .render_frame(None)
        .expect("headless render");
    let base = baseline.pixels.expect("software raster pixels");

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
    let over = overlaid.pixels.expect("software raster pixels");

    assert_eq!(base.len(), over.len(), "same viewport");
    let diff = base.iter().zip(over.iter()).filter(|(a, b)| a != b).count();
    assert!(diff > 0, "the overlay painted onto the scene: {diff} bytes");
    assert!(
        overlaid.stamp.hash != baseline.stamp.hash,
        "frame digest moved"
    );
}

/// A timer-driven app (§14): the counter label advances on a real
/// `subscription::time::every` tick — the proof surface for the clock
/// binding.
struct TickerApp;

#[derive(Debug, Clone)]
enum TickMsg {
    Tick,
    Hook(TreeDumpTx),
}

impl Program for TickerApp {
    type State = u64;
    type Message = TickMsg;
    type Theme = Theme;
    type Renderer = renderer::Renderer;
    type Executor = iced_test::futures::backend::default::Executor;

    fn name() -> &'static str {
        "guiwright_ticker"
    }

    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn window(&self) -> Option<iced_test::core::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (0, Task::none())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            TickMsg::Tick => *state += 1,
            TickMsg::Hook(tx) => return <TickerApp as HarnessApp>::run_hook(state, tx),
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced_test::core::window::Id,
    ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        column![text(format!("Ticks: {state}"))].into()
    }

    fn subscription(
        &self,
        _state: &Self::State,
    ) -> iced_test::futures::Subscription<Self::Message> {
        iced_test::futures::backend::default::time::every(Duration::from_millis(50))
            .map(|_| TickMsg::Tick)
    }
}

impl HarnessApp for TickerApp {
    fn hook(tx: TreeDumpTx) -> Self::Message {
        TickMsg::Hook(tx)
    }
}

/// The §14 M2 proof: `clock_fast_forward` advances the framework's own
/// timers — the subscription ticks landed and the label moved off zero.
#[test]
fn clock_fast_forward_completes_subscription_timers() {
    let harness = EmulatorHarness::new(TickerApp, (300.0, 200.0)).expect("harness boots");
    let adapter = guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));

    // Boot state: zero ticks (the label is exactly "Ticks: 0").
    assert!(
        session.resolve(&Selector::text("Ticks: 0")).is_ok(),
        "boot state shows zero ticks"
    );

    session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::InstallVirtual)
        .expect("install virtual clock");
    let state = session
        .adapter_mut()
        .clock(guiwright_driver::adapter::ClockCmd::FastForward(
            Duration::from_millis(300),
        ))
        .expect("fast forward drains due timers");
    assert!(
        state.virtual_time.expect("ledger") >= Duration::from_millis(300),
        "the ledger advanced: {state:?}"
    );

    // The subscription fired through the real pipeline: the label moved.
    assert!(
        !session.resolve(&Selector::text("Ticks: 0")).is_ok(),
        "clock_fast_forward completed the subscription timer"
    );
}

// --- §12 m85: the trace end-to-end row --------------------------------

/// The §12 pipeline over a real iced session (m85): the kit records the
/// App's actions into a `gwtrace/v1` archive and `guiwright show-trace`
/// renders the scrubber HTML — the iced adapter header and the recorded
/// event vocabulary included.
#[test]
fn trace_archive_renders_through_show_trace() {
    use guiwright_kit::App;

    let dir = tempfile::tempdir().expect("tmpdir");
    let archive_path = dir.path().join("guiwright-iced-trace.zip");
    let harness = EmulatorHarness::new(CounterForm, (480.0, 320.0)).expect("harness boots");
    let adapter = guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
    let mut app = App::launch("iced_trace", adapter);
    app.enable_trace(&archive_path);
    app.get_by_role(accesskit::Role::Button, "Increment")
        .click()
        .expect("click Increment");

    let archive = app.finish().expect("trace finalized");
    assert_eq!(archive, archive_path, "the explicit path is honored");
    let written = std::fs::metadata(&archive).expect("archive exists").len();
    assert!(written > 0, "the archive is not empty ({written} bytes)");

    let html = guiwright_cli::show_trace(&archive, None, false).expect("html rendered");
    let rendered = std::fs::read_to_string(&html).expect("html written next to the archive");
    assert!(rendered.contains("gwtrace"), "schema marker present");
    assert!(rendered.contains("iced"), "adapter name in the header");
    assert!(rendered.contains("click"), "the click event is in the HTML");
}

// --- §10 m86: the video + visual-regression rows -----------------------

/// The §10 recorder over real iced pixels (m86): the pump's frames flow
/// through `last_frame` into the recorder — the static span dedups (the
/// pump emits the settled pass twice, §4.6 Zen), the click repaint
/// records, and the stop finalizes a valid artifact: non-zero size, ≥2
/// distinct frames around the repaint, a wall-clock duration, and a real
/// container (ffmpeg EBML when the binary is present, the PNG frame
/// folder + `video.json` manifest otherwise).
#[test]
fn video_records_presented_pixels() {
    use guiwright_kit::App;

    let dir = tempfile::tempdir().expect("tmpdir");
    let harness = EmulatorHarness::new(CounterForm, (480.0, 320.0)).expect("harness boots");
    let adapter = guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
    let mut app = App::launch("iced_video", adapter);

    app.video().start_in(dir.path(), 25).expect("video starts");
    app.session().pump();
    // A real wall-clock span between the first and the last accepted
    // frame (the summary's duration metadata, §10).
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.get_by_role(accesskit::Role::Button, "Increment")
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

/// The §10 visual-regression pipeline over real iced pixels (m86): the
/// settled app blesses an element baseline (`BlessMode::Failing` — the
/// `GUIWRIGHT_BLESS=1` form), and the unchanged re-render diffs clean
/// (`BlessMode::Off`) — the §10 bless → diff round trip end to end, from
/// the Emulator's tiny-skia raster through the crop-aware element
/// screenshot into the per-platform baseline store.
#[test]
fn screenshot_baseline_bless_and_diff_roundtrip() {
    use guiwright_kit::{App, BlessMode, SnapshotOutcome};

    let dir = tempfile::tempdir().expect("tmpdir");

    // Bless: the first run writes the baseline (the missing-baseline
    // branch of the §10 flow).
    let blessed = {
        let harness = EmulatorHarness::new(CounterForm, (480.0, 320.0)).expect("harness boots");
        let adapter =
            guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
        let mut app = App::launch("iced_bless", adapter).with_baseline_dir(dir.path());
        app.get_by_role(accesskit::Role::Button, "Increment")
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
    let harness = EmulatorHarness::new(CounterForm, (480.0, 320.0)).expect("harness boots");
    let adapter = guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
    let mut app = App::launch("iced_diff", adapter).with_baseline_dir(dir.path());
    let matched = app
        .get_by_role(accesskit::Role::Button, "Increment")
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
