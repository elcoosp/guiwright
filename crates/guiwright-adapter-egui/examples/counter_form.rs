//! A real egui app driven end-to-end through a Guiwright [`Session`] —
//! the consumer example (architecture §4.1 "Embed"), the same
//! counter+form family the other adapters ship (iced §4.6, freya, the
//! floem/gpui/slint glue).
//!
//! Run it:
//!
//! ```text
//! cargo run -p guiwright-adapter-egui --example counter_form
//! ```
//!
//! The app is a stock egui closure (counter buttons + a name field, no
//! Guiwright code inside it). The example boots the §4.1 windowless
//! adapter, resolves locators against the AccessKit tree egui
//! generates, clicks and fills through the semantic pipeline, and
//! prints what the tree reflects after every step — the same flow the
//! embedded tests pin, as a runnable walkthrough.

use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_adapter_egui::EguiAdapter;
use guiwright_core::Selector;
use guiwright_driver::session::Session;

fn increment() -> Selector {
    Selector::role_named(Role::Button, "Increment")
}

fn decrement() -> Selector {
    Selector::role_named(Role::Button, "Decrement")
}

fn name_input() -> Selector {
    Selector::role(Role::TextInput)
}

fn main() {
    let mut count: i64 = 0;
    let mut name = String::new();
    let adapter = EguiAdapter::new(move |ctx| {
        ctx.root_ui(|ui| {
            ui.label(format!("Count: {count}"));
            if ui.button("Increment").clicked() {
                count += 1;
            }
            if ui.button("Decrement").clicked() {
                count -= 1;
            }
            ui.add(egui::TextEdit::singleline(&mut name).id_salt("name"));
            ui.label(format!("Welcome, {name}"));
        });
    });
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));

    println!("== egui counter/form, driven end-to-end (§4.1 embed) ==");

    let resolved = session
        .resolve(&increment())
        .expect("Increment resolves by role+name");
    println!(
        "resolve  role=button name=Increment  → bounds {:?}",
        resolved.bounds
    );

    session.click(&increment()).expect("click Increment");
    session
        .resolve(&Selector::text("Count: 1").first())
        .expect("tree reflects the click");
    println!("click    Increment               → tree shows \"Count: 1\"");

    session.fill(&name_input(), "Ada").expect("fill the name");
    session
        .resolve(&Selector::text("Welcome, Ada").first())
        .expect("tree reflects the typed name");
    println!("fill     name=Ada                → tree shows \"Welcome, Ada\"");

    session.click(&decrement()).expect("click Decrement");
    session
        .resolve(&Selector::text("Count: 0").first())
        .expect("tree reflects the second click");
    println!("click    Decrement               → tree shows \"Count: 0\"");
    println!("== done: every step ran through egui's real widget pipeline ==");
}
