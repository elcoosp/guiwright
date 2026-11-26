//! The kit's regex text probes (the m34 record's second standing note,
//! cashed — the m93/m94 honest lists' first item: *"No regex
//! `TextPattern` … the REPL's `has_text` word speaks the fuzzy
//! `TextMatch` default only"*). The acceptance this pins: a
//! [`TextPattern::Regex`] probe rides the same surfaces the string
//! probes always spoke — the kit's `get_by_*` vocabulary, the
//! `has_text` narrowing, the `to_have_text`/`to_have_value` waitable
//! pair — embedded (the core resolver's `matches` arm) and over the
//! door (the appended `SelectorV1` regex rows, the M64 rule at selector
//! width), with the JS-convention semantics: partial by default,
//! case-sensitive until the pattern's inline `(?i)` flag, the
//! `exact` flag anchoring.

use accesskit::Role;
use guiwright_core::Selector;
use guiwright_core::selector::TextMatch;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn fixture_app(test_name: &str) -> App<MockAdapter> {
    let mut app = App::launch(test_name, MockAdapter::with_fixture(&Fixture::buttons()));
    app.wait_for_settle();
    app
}

fn regex(pattern: &str) -> TextMatch {
    TextMatch::regex(pattern)
}

#[test]
fn regex_text_probe_resolves_through_the_kit_surface() {
    let mut app = fixture_app("kit_regex_text");

    // getByText(/save|cancel/i): the pattern's inline flag decides the
    // case rule — the probe is partial by default.
    assert_eq!(
        app.get_by_text(regex("(?i)save|cancel")).count().unwrap(),
        2,
        "Save and Cancel answer; the label and input don't"
    );
    // The anchored form: full-match semantics composed from the source.
    assert_eq!(app.get_by_text(regex("^Save$")).count().unwrap(), 1);
    assert_eq!(
        app.get_by_text(regex("^save$")).count().unwrap(),
        0,
        "case-sensitive without the inline flag — the JS convention"
    );

    // Strict mode rides the probe unchanged: two matches refuse, the
    // narrowing saves the resolve — and the resolved node is the probe's
    // own answer, compared against the string form's key.
    let err = app
        .get_by_text(regex("(?i)save|cancel"))
        .resolve()
        .expect_err("two matches — strict mode refuses");
    assert!(err.to_string().contains("resolved to 2 nodes"), "{err}");
    let resolved = app
        .get_by_text(regex("(?i)^cancel$"))
        .resolve()
        .expect("the anchored probe resolves one");
    let cancel = app
        .key_of(&Selector::role_named(Role::Button, "Cancel"))
        .expect("Cancel resolves unscoped on the single-window fixture");
    assert_eq!(resolved.key, cancel, "the regex probe aims the same node");
}

#[test]
fn regex_role_name_probe_narrows_the_role() {
    let mut app = fixture_app("kit_regex_role_name");

    // getByRole(Button, { name: /^(Save|Cancel)$/ }): the name probe is
    // a regex — the role bucket narrows by the pattern, the strict-mode
    // refusal naming the two survivors.
    assert_eq!(
        app.get_by_role(Role::Button, regex("^(Save|Cancel)$"))
            .count()
            .unwrap(),
        2,
    );
    let err = app
        .get_by_role(Role::Button, regex("^(Save|Cancel)$"))
        .click()
        .expect_err("two buttons answer the pattern — strict mode refuses");
    assert!(err.to_string().contains("resolved to 2 nodes"), "{err}");

    // The action path rides the probe: one match clicks through the
    // same actionability gate as any locator.
    let outcome = app
        .get_by_role(Role::Button, regex("^Save$"))
        .click()
        .expect("the anchored probe clicks");
    assert_eq!(outcome.detail, "clicked", "{outcome:?}");
    assert_eq!(app.session().adapter().actions().len(), 1);
}

#[test]
fn regex_has_text_filter_narrows_the_locator_set() {
    let mut app = fixture_app("kit_regex_has_text");

    // filter({ hasText: /Sav/ }): the subtree text walk with the regex
    // probe — Save kept, the other buttons dropped.
    assert_eq!(
        app.role_locator(Role::Button)
            .has_text(regex("^Sav"))
            .count()
            .unwrap(),
        1,
    );
    // The fuzzy form beside it: unchanged bytes, unchanged semantics.
    assert_eq!(
        app.role_locator(Role::Button)
            .has_text("Save")
            .count()
            .unwrap(),
        1,
    );
}

#[test]
fn regex_rides_the_waitable_content_assertions() {
    let mut app = fixture_app("kit_regex_expect");

    // to_have_text(/Welcome, Ada/): the waiter evaluates the probe
    // client-side over the describe-all re-reads (the m54 wake loop) —
    // a regex expectation composes the same zero-polling wait.
    guiwright_kit::expect(app.get_by_role(Role::Label, regex("^Wel.*")))
        .to_have_text(regex("Welcome, Ada"))
        .expect("the label's text fits the pattern");
    // The value probe: the input's value speaks the pattern.
    guiwright_kit::expect(app.get_by_role(Role::TextInput, regex("^Email$")))
        .to_have_value(regex(r"^\S+@\S+\.\S+$"))
        .expect("the email value fits the pattern");
    // The negation composes (the M24 decorator): a pattern the value
    // never satisfies pends until the typed timeout names the fact.
    let err = guiwright_kit::expect(app.get_by_role(Role::TextInput, regex("^Email$")))
        .not()
        .to_have_value(regex("ada@example"))
        .expect_err("the value carries the address — the negated probe pends");
    assert!(
        err.to_string().contains("value"),
        "the timeout names the fact: {err}"
    );
}

/// The door half: the same regex probe over the wire — the kit
/// selector serializes to the appended `SelectorV1` regex rows (the
/// M64 rule), the resolver answers through the in-process door
/// byte-identically with the UDS/TCP/WS doors.
#[test]
fn regex_probes_cross_the_door() {
    use guiwright_kit::remote::RemoteToSelf;

    let mut rts = RemoteToSelf::launch_fixture();
    // The wire resolve: getByText(/(?i)^save$/) rides TextRegex.
    let resolved = rts
        .resolve(Selector::text(regex("(?i)^save$")), 2_000)
        .expect("the regex probe crosses the door");
    let guiwright_proto::schema::NodeKeyV1::AccessKit { node, .. } = resolved.node else {
        panic!("the mock fixture speaks AccessKit handles");
    };
    assert_eq!(node, 2, "the fixture's Save button");
    // The regex has_text filter rides HasTextRegex (the composed row):
    // the describe-all set read answers the filtered walk.
    let rows = rts
        .describe_all(Selector::role(Role::Button).has_text(regex("^Save$")))
        .expect("the regex filter crosses the door");
    assert_eq!(rows.len(), 1, "Save alone fits the anchored pattern");
    assert_eq!(rows[0].name.as_deref(), Some("Save"));
    rts.finish().expect("shutdown joins");
}
