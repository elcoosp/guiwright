//! A real iced app driven end-to-end through a Guiwright [`Session`] —
//! the m76 consumer example (architecture §4.6, §17 MVP trio).
//!
//! Run it:
//!
//! ```text
//! cargo run -p guiwright-adapter-iced --features iced --example counter_form
//! ```
//!
//! The app is a stock iced `Program` (counter buttons + a name field).
//! The example boots the m76 harness ([`EmulatorHarness`] over
//! `iced_test::Emulator`), resolves locators against the logical tree,
//! clicks and fills through the real event pipeline, and prints what the
//! tree reflects after every step — the same flow the m76 tests pin, as a
//! runnable walkthrough.

#![cfg(feature = "iced")]

use std::time::{Duration, Instant};

use guiwright_adapter_iced::RoleRegistry;
use guiwright_adapter_iced::emulator::{EmulatorHarness, HarnessApp, SharedHarness, TreeDumpTx};
use guiwright_adapter_iced::harness::IcedAdapter;
use guiwright_core::Selector;
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

/// The demo app: counter + form, stock widgets.
struct CounterForm;

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
            // Interactive widgets are Id-carrying containers around stock
            // buttons — the §4.6 semantic contribution (see kind_of).
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

    // kind_of: the app's semantic contribution (§4.6 role registry); iced
    // 0.14 passes a button's Id to no operation hook, so the wrapper
    // container's Id carries the semantics.
    fn kind_of(id: &Id) -> Option<&'static str> {
        for name in ["increment", "decrement"] {
            if *id == Id::new(name) {
                return Some("button");
            }
        }
        None
    }
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

fn main() {
    let harness =
        EmulatorHarness::new(CounterForm, (480.0, 320.0)).expect("iced harness boots headlessly");
    let adapter = guiwright_adapter_iced::emulator::embedded_adapter(harness, RoleRegistry::new());
    let mut session: Session<IcedAdapter<SharedHarness<CounterForm>>> = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));

    println!("== iced counter/form, driven end-to-end (m76) ==");

    let resolved = session
        .resolve(&increment())
        .expect("Increment resolves by role+name");
    println!(
        "resolve  role=button name=Increment  → bounds {:?}",
        resolved.bounds
    );

    session.click(&increment()).expect("click Increment");
    session
        .resolve(&Selector::text("Count: 1"))
        .expect("tree reflects the click");
    println!("click    Increment               → tree shows \"Count: 1\"");

    session.fill(&name_input(), "Ada").expect("fill the name");
    session
        .resolve(&Selector::text("Welcome, Ada"))
        .expect("tree reflects the typed name");
    println!("fill     name=Ada                → tree shows \"Welcome, Ada\"");

    session.click(&decrement()).expect("click Decrement");
    session
        .resolve(&Selector::text("Count: 0"))
        .expect("tree reflects the second click");
    println!("click    Decrement               → tree shows \"Count: 0\"");

    let dump = session.tree_dump();
    println!(
        "tree     {} nodes under the window root — logical snapshot stable",
        dump.nodes.len()
    );
    println!("== done: every step ran through iced's real event pipeline ==");
}
