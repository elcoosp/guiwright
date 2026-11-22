//! The §13 remote codegen end-to-end (m87): the wire path — a recording
//! started on the session (`codegen_start`, the `Inspect::StartRecording`
//! arm), actions applied through the pre-resolved GWP path
//! (`apply_action`, the M17 surface the server's Action domain rides),
//! and `codegen_stop` handing back the exact kit code the wire sequence
//! performed. The embedded kit surface has its own E2E (the kit's
//! `codegen_e2e.rs`); this is the remote-mode half — no double
//! recording, one recorder per surface.

use std::time::Duration;

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::test_util::Fixture;
use guiwright_driver::Session;
use guiwright_driver::adapter::SemAction;
use guiwright_driver::mock::MockAdapter;

fn settle(session: &mut Session<MockAdapter>) {
    session.settle(std::time::Instant::now() + Duration::from_secs(5));
}

/// The wire sequence records the same §13 lines the REPL's codegen
/// session emits — role+name selectors from the cost model, the fill
/// argument carried, no warnings.
#[test]
fn remote_codegen_records_the_wire_action_sequence() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);

    session.codegen_start().expect("recording starts");

    // Resolve + apply — the wire Action path (§7/M17): the caller holds
    // the handle, the session re-checks actionability at act time.
    let save = session
        .resolve(&Selector::role_named(Role::Button, "Save"))
        .expect("the save button resolves");
    session
        .apply_action(save.key, SemAction::Click, false, 2_000)
        .expect("click applies");

    let email = session
        .resolve(&Selector::role_named(Role::TextInput, "Email"))
        .expect("the email field resolves");
    session
        .apply_action(
            email.key,
            SemAction::SetValue {
                text: "ada@lovelace.dev".to_string(),
            },
            false,
            2_000,
        )
        .expect("fill applies");

    let (code, warnings) = session.codegen_stop().expect("recording stops");
    assert_eq!(
        code,
        "app.get_by_role(Role::Button, \"Save\").click();\n\
         app.get_by_role(Role::TextInput, \"Email\").fill(\"ada@lovelace.dev\");\n",
        "the wire sequence's recorded program"
    );
    assert!(
        warnings.is_empty(),
        "strict mode stayed clean: {warnings:?}"
    );

    // Recording off: a further action records nothing (and stopping
    // again is the documented clean error).
    let save = session
        .resolve(&Selector::role_named(Role::Button, "Cancel"))
        .expect("cancel resolves");
    session
        .apply_action(save.key, SemAction::Click, false, 2_000)
        .expect("the post-stop click still applies");
    let err = session
        .codegen_stop()
        .expect_err("stopping twice is the clean error");
    assert!(
        err.to_string().contains("no codegen recording is active"),
        "{err}"
    );
}

/// Double-start is the documented clean error (the video domain's
/// idempotence rules, §10).
#[test]
fn double_codegen_start_is_rejected() {
    let mut session = Session::new(MockAdapter::with_fixture(&Fixture::buttons()));
    settle(&mut session);
    session.codegen_start().expect("first start");
    let err = session
        .codegen_start()
        .expect_err("the second start is rejected");
    assert!(err.to_string().contains("already active"), "{err}");
    session.codegen_stop().expect("stop drains");
}
