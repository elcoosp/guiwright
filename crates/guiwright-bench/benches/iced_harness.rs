//! The m76 iced harness bench (architecture §15.7, budget row
//! `iced_tree_readback`): one full §4.6 read-back pass on a real
//! `iced_test::Emulator` — the drain-then-dump cycle the adapter's pump
//! rides per frame.
//!
//! Renderer-bound by design: the Emulator owns the tiny-skia device, so
//! the number measures the harness path (hook round trip, walk, fold),
//! not the renderer. It is a design-target row (`m0_measurable = false`)
//! until the iced runtime matrix enforces it like the other framework
//! rows.
//!
//! Gated on the bench crate's `iced` feature (§18: the default build
//! carries no framework stack); runs under `--all-features`.

use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use guiwright_adapter_iced::dump::TreeDumpTx;
use guiwright_adapter_iced::emulator::{EmulatorHarness, HarnessApp};
use guiwright_adapter_iced::harness::SimulatorSeam;
use iced_test::core::widget::Id;
use iced_test::core::{Element, Settings, Theme};
use iced_test::program::Program;
use iced_test::renderer;
use iced_test::runtime::Task;
use iced_widget::{button, column, container, text};

/// The bench app: the m76 counter shape, minimal (one text + one button).
struct BenchApp;

#[derive(Debug, Clone)]
enum BenchMsg {
    Increment,
    Hook(TreeDumpTx),
}

struct BenchState {
    count: i64,
}

impl Program for BenchApp {
    type State = BenchState;
    type Message = BenchMsg;
    type Theme = Theme;
    type Renderer = renderer::Renderer;
    type Executor = iced_test::futures::backend::default::Executor;

    fn name() -> &'static str {
        "guiwright_bench_app"
    }

    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn window(&self) -> Option<iced_test::core::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (BenchState { count: 0 }, Task::none())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            BenchMsg::Increment => state.count += 1,
            BenchMsg::Hook(tx) => {
                return <BenchApp as HarnessApp>::run_hook(state, tx);
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
            // An `Id`-carrying container around a stock button — the §4.6
            // semantic contribution (see `dump::HarnessApp::kind_of`).
            container(button("Increment").on_press(BenchMsg::Increment)).id(Id::new("increment")),
        ]
        .into()
    }
}

impl HarnessApp for BenchApp {
    fn hook(tx: TreeDumpTx) -> Self::Message {
        BenchMsg::Hook(tx)
    }

    fn kind_of(id: &Id) -> Option<&'static str> {
        (*id == Id::new("increment")).then_some("button")
    }
}

fn iced_readback(c: &mut Criterion) {
    // Setup once: renderer init dominates and is not the measured op.
    let mut harness =
        EmulatorHarness::<BenchApp>::new(BenchApp, (320.0, 240.0)).expect("bench harness boots");

    let mut group = c.benchmark_group("iced_harness");
    group.bench_function("tree_readback_full_pass", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                harness.run_one_pass();
            }
            start.elapsed()
        });
    });
    group.finish();
}

criterion_group!(benches, iced_readback);
criterion_main!(benches);
