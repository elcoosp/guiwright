//! The m80 glue benchmark (§15.7): one full introspection pass —
//! harness rebuild + real view-tree walk + engine fold — over the real
//! floem counter+form app. The design-target row rides the
//! `floem_introspection` entry (`m0_measurable = false`: the glue
//! workspace owns the sample, the main workspace carries no floem
//! stack).

use std::rc::Rc;
use std::time::Instant;

use floem::view::IntoView;
use floem::views::{Button, Decorators, Label, Stack, TextInput};
use floem_reactive::{RwSignal, SignalGet, SignalUpdate};

use guiwright_adapter_floem_glue::harness::{FloemAdapter, SemReader};

struct BenchApp {
    count: RwSignal<i32>,
    name: RwSignal<String>,
}

fn bench_view(app: &Rc<BenchApp>) -> impl IntoView {
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

fn bench_readers(app: &Rc<BenchApp>) -> Vec<SemReader> {
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

fn boot_adapter() -> FloemAdapter {
    let app = Rc::new(BenchApp {
        count: RwSignal::new(0),
        name: RwSignal::new(String::new()),
    });
    FloemAdapter::boot(
        (480.0, 320.0),
        {
            let app = Rc::clone(&app);
            move || bench_view(&app).into_any()
        },
        bench_readers(&app),
    )
    .expect("harness boots")
}

criterion::criterion_group! {
    name = benches;
    config = criterion::Criterion::default().sample_size(10);
    targets = floem_introspection
}
criterion::criterion_main!(benches);

fn floem_introspection(c: &mut criterion::Criterion) {
    c.bench_function("floem_introspection", |b| {
        let mut adapter = boot_adapter();
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = adapter.introspect_once();
            }
            start.elapsed()
        });
    });
}
