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
