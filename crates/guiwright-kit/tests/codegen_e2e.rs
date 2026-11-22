//! The §13 codegen end-to-end (m87): one recording session over the kit —
//! the §13 interactive shape (overlay on, recorder on), then a full
//! sequence of locator actions (click, fill, press, hover) driven
//! through the kit surface — asserting the generated Rust code matches
//! the expected output **exactly** (the cost-model tiers, the fill/press
//! arguments, no warnings), and that reads record nothing.

use accesskit::Role;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn fixture_app() -> App<MockAdapter> {
    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("codegen_e2e", adapter);
    app.wait_for_settle();
    app
}

/// The §13 recording loop, end to end: the overlay rides (the §13
/// interactive session shows what it records), every action lands in the
/// recorder against its minimal stable selector, and `codegen_finish`
/// hands back the exact kit program the session performed.
#[test]
fn a_recorded_session_emits_the_exact_kit_code() {
    let mut app = fixture_app();
    // The §13 interactive shape: the inspector overlay on, then the
    // recorder on — actions flow through the locator taps below.
    app.inspect().enable().expect("overlay on");
    app.enable_codegen();

    app.get_by_role(Role::Button, "Save")
        .click()
        .expect("click Save");
    app.get_by_role(Role::TextInput, "Email")
        .fill("ada@lovelace.dev")
        .expect("fill the email");
    app.get_by_role(Role::TextInput, "Email")
        .press("Enter")
        .expect("press Enter");
    app.get_by_text("Welcome, Ada")
        .hover()
        .expect("hover the label");

    let (code, warnings) = app.codegen_finish().expect("recorder active");
    assert_eq!(
        code,
        "app.get_by_role(Role::Button, \"Save\").click();\n\
         app.get_by_role(Role::TextInput, \"Email\").fill(\"ada@lovelace.dev\");\n\
         app.get_by_role(Role::TextInput, \"Email\").press(\"Enter\");\n\
         app.get_by_role(Role::Label, \"Welcome, Ada\").hover();\n",
        "the recorded program matches the session exactly"
    );
    assert!(
        warnings.is_empty(),
        "strict mode stayed clean: {warnings:?}"
    );
}

/// The cost model's tier order rides the same sequence: role+name wins
/// over text even when a text selector would also resolve (the label
/// carries both facts — the §13 "minimal stable selector" rule).
#[test]
fn the_cost_model_picks_the_tier_the_session_asserts() {
    let mut app = fixture_app();
    app.enable_codegen();

    // The label node resolves by role+name AND by text; the recorder
    // emits the role+name form (tier 1 beats tier 3).
    app.get_by_role(Role::Label, "Welcome, Ada")
        .hover()
        .expect("hover");
    let (code, _) = app.codegen_finish().expect("recorder active");
    assert_eq!(
        code, "app.get_by_role(Role::Label, \"Welcome, Ada\").hover();\n",
        "role+name outranks text"
    );
}

/// Reads and assertions are not actions: nothing records (the §13 rule
/// the locator reads ride — codegen captures interactions, not reads).
#[test]
fn reads_record_no_lines() {
    let mut app = fixture_app();
    app.enable_codegen();

    // Reads are not interactions: the set read and the bulk read run
    // against the same selector and record nothing.
    let _ = app
        .get_by_role(Role::Button, "Save")
        .count()
        .expect("the set read");
    let _ = app
        .get_by_role(Role::Button, "Save")
        .all_text_contents()
        .expect("the bulk read");

    let (code, warnings) = app.codegen_code().expect("recorder active");
    assert_eq!(code, "", "reads record no codegen lines");
    assert!(warnings.is_empty(), "no warnings either: {warnings:?}");
}
