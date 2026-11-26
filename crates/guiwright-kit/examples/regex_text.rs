//! The regex text probes (§16 locators row — the m34 record's second
//! standing note, cashed; the m93/m94 honest lists' first item): the
//! `TextPattern::Regex` arm on the kit's text-matching vocabulary —
//! `getByText(/re/)`'s shape. The pattern speaks its own case rule
//! (inline flags, the JS convention), the probe is partial by default
//! (a regex `test`), and `exact` anchors it — every surface the string
//! probes spoke (`get_by_*`, `has_text`, the `to_have_text` /
//! `to_have_value` waitable pair) answers the pattern through the same
//! zero-polling session loop, embedded or over the door.
//!
//! ```text
//! cargo run -p guiwright-kit --example regex_text
//! ```
//!
//! Filters the button fixture by pattern probes, pins the case rule,
//! composes the `has_text` narrowing, and rides the strict gate through
//! an anchored probe.

use accesskit::Role;
use guiwright_core::selector::TextMatch;
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_kit::App;

fn main() {
    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("regex_text_demo", adapter);
    app.wait_for_settle();

    // The probe is partial by default: /Save|Cancel/ answers the two
    // buttons, the label and input stay out (their text never carries
    // the pattern).
    let pattern = TextMatch::regex("(?i)save|cancel");
    let set = app.get_by_text(pattern).count().expect("the pattern set");
    println!("getByText(/(?i)save|cancel/) → {set} match(es) — the inline flag decides the case");

    // Case-sensitive until the pattern says otherwise: the lowercase
    // anchor never meets `Save` — the honest empty read.
    let anchored = TextMatch::regex_exact("^save$");
    let none = app.get_by_text(anchored).count().expect("the empty set");
    println!("getByText(/^save$/) → {none} match(es) — case-sensitive without (?i)");

    // The role name is a text probe too: the role bucket narrows by the
    // pattern (getByRole's `name: /re/` form).
    let named = app
        .get_by_role(Role::Button, TextMatch::regex("^(Save|Cancel)$"))
        .count()
        .expect("the role-name probe");
    println!("getByRole(Button, {{ name: /^(Save|Cancel)$/ }}) → {named} match(es)");

    // The `has_text` narrowing speaks the pattern (the filter's subtree
    // text walk): Save alone fits the anchored form.
    let filtered = app
        .role_locator(Role::Button)
        .has_text(TextMatch::regex("^Sav"))
        .count()
        .expect("the filtered set");
    println!("Button.filter(hasText: /^Sav/) → {filtered} match(es) — the subtree walk");

    // The waitable pair evaluates client-side: the label's text fits
    // the pattern (the m54 wake loop, zero polling).
    guiwright_kit::expect(app.get_by_role(Role::Label, TextMatch::regex("^Wel.*")))
        .to_have_text(TextMatch::regex("Welcome, Ada"))
        .expect("the label's text fits the pattern");
    println!("expect(Label /^Wel.*/).toHaveText(/Welcome, Ada/) → satisfied");

    // Actions ride the same strict-mode resolve + actionability gate
    // through the probe.
    let outcome = app
        .get_by_role(Role::Button, TextMatch::regex("^Save$"))
        .click()
        .expect("the anchored probe clicks");
    println!(
        "getByRole(Button, {{ name: /^Save$/ }}).click() → {}",
        outcome.detail
    );

    // The composed selector renders the Playwright shape.
    let rendered = guiwright_core::Selector::text(TextMatch::regex("(?i)save|cancel"));
    println!("selector render → {}", rendered.render());
}
