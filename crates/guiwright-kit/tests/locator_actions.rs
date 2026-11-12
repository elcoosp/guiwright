//! Locator action parity (§16 actions row, M18-follow-on): the wire's
//! semantic vocabulary (`SemAction::Press/Check/Uncheck/Focus/Blur`)
//! lands on the kit's locator surface — the same auto-waiting,
//! codegen-recorded path every other locator action takes. The mock
//! adapter records what it is handed, so the tests pin the action
//! vocabulary end-to-end: locator method → session gate → adapter act.

use accesskit::Role;
use guiwright_driver::SemAction;
use guiwright_kit::App;

#[test]
fn locator_press_reaches_the_adapter() {
    let mut app = App::launch_fixture("locator_press");
    let outcome = app
        .get_by_role(Role::Button, "Save")
        .press("Enter")
        .expect("press");
    assert_eq!(outcome.detail, "press(Enter)");
    let actions = app.session().adapter().actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].1,
        SemAction::Press {
            key: "Enter".into(),
            modifiers: Vec::new(),
        }
    );
}

#[test]
fn locator_check_uncheck_and_set_checked_reach_the_adapter() {
    let mut app = App::launch_fixture("locator_check");
    app.get_by_role(Role::Button, "Save")
        .check()
        .expect("check");
    app.get_by_role(Role::Button, "Save")
        .uncheck()
        .expect("uncheck");
    app.get_by_role(Role::Button, "Save")
        .set_checked(true)
        .expect("set_checked(true)");
    app.get_by_role(Role::Button, "Save")
        .set_checked(false)
        .expect("set_checked(false)");

    let actions = app.session().adapter().actions();
    let kinds: Vec<&SemAction> = actions.iter().map(|(_, action)| action).collect();
    assert_eq!(
        kinds,
        vec![
            &SemAction::Check,
            &SemAction::Uncheck,
            &SemAction::Check,
            &SemAction::Uncheck,
        ],
        "set_checked routes through the same Check/Uncheck vocabulary"
    );
}

#[test]
fn locator_focus_and_blur_reach_the_adapter() {
    let mut app = App::launch_fixture("locator_focus");
    app.get_by_role(Role::TextInput, "Email")
        .focus()
        .expect("focus");
    app.get_by_role(Role::TextInput, "Email")
        .blur()
        .expect("blur");

    let actions = app.session().adapter().actions();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].1, SemAction::Focus);
    assert_eq!(actions[1].1, SemAction::Blur);
}

#[test]
fn locator_parity_actions_run_the_actionability_gate() {
    // The hidden button is in the fixture tree but has no bounds: the
    // same gate every other locator action runs refuses it (§8.3) — the
    // typed timeout names the selector and the check vocabulary it was
    // stuck on (the embedded path's wording; the M17 wire gate adds the
    // "stuck on <check>" report over the wire).
    let mut app = App::launch_fixture("locator_gate");
    let err = app
        .get_by_role(Role::Button, "Ghost")
        .check()
        .expect_err("a hidden node is refused");
    let message = err.to_string();
    assert!(
        message.contains("visible"),
        "the refusal names the check vocabulary: {message}"
    );
    assert!(
        message.contains("Ghost"),
        "the refusal names the selector: {message}"
    );
}

#[test]
fn locator_parity_actions_record_codegen_lines() {
    let mut app = App::launch_fixture("locator_codegen");
    app.enable_codegen();
    app.get_by_role(Role::Button, "Save")
        .press("Enter")
        .expect("press");
    app.get_by_role(Role::Button, "Save")
        .check()
        .expect("check");
    app.get_by_role(Role::Button, "Save")
        .focus()
        .expect("focus");

    let (code, warnings) = app.codegen_code().expect("codegen recorded");
    assert!(
        warnings.is_empty(),
        "the fixture resolves unambiguously: {warnings:?}"
    );
    assert!(code.contains(".press(\"Enter\");"), "{code}");
    assert!(code.contains(".check();"), "{code}");
    assert!(code.contains(".focus();"), "{code}");
}
