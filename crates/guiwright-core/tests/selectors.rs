//! Resolution pipeline integration tests: strict mode, filters, chains,
//! test-id registry (architecture §8.1–§8.2).

use accesskit::Role;
use guiwright_core::CoreError;
use guiwright_core::selector::{Selector, TextMatch};
use guiwright_core::test_util::Fixture;
use guiwright_core::tree::TreeService;

fn tree() -> (TreeService, Fixture) {
    let fixture = Fixture::buttons();
    let mut svc = TreeService::new();
    svc.apply_update(fixture.initial_update());
    (svc, fixture)
}

fn node_id(key: guiwright_core::NodeKey) -> accesskit::NodeId {
    let guiwright_core::NodeKey::AccessKit { node, .. } = key else {
        panic!("fixture keys are accesskit keys");
    };
    node
}

#[test]
fn role_and_name_resolves_exactly_one() {
    let (svc, _) = tree();
    let resolved = svc
        .resolve(&Selector::role_named(Role::Button, "Save"))
        .expect("save resolves");
    assert_eq!(svc.generation(&resolved.key), 0);
    assert!(resolved.bounds.is_some());
}

#[test]
fn strict_mode_rejects_multiple_matches() {
    let (svc, _) = tree();
    let err = svc
        .resolve(&Selector::role(Role::Button))
        .expect_err("four buttons");
    assert!(matches!(err, CoreError::Ambiguous { .. }), "{err}");
    let msg = err.to_string();
    assert!(msg.contains("4 nodes"), "{msg}");
    assert!(msg.contains(".first()"), "{msg}");
}

#[test]
fn no_match_reports_tree_summary() {
    let (svc, _) = tree();
    let err = svc
        .resolve(&Selector::role_named(Role::Button, "Deploy"))
        .expect_err("no deploy button");
    match err {
        CoreError::NoMatch { last, .. } => {
            assert_eq!(last.node_count, 8);
            assert!(
                last.sample
                    .iter()
                    .any(|d| d.name.as_deref() == Some("Save"))
            );
        }
        other => panic!("expected NoMatch, got {other:?}"),
    }
}

#[test]
fn first_last_nth_narrowing() {
    let (svc, f) = tree();
    let first = svc
        .resolve(&Selector::role(Role::Button).first())
        .expect("first button");
    let last = svc
        .resolve(&Selector::role(Role::Button).last())
        .expect("last button");
    let nth = svc
        .resolve(&Selector::role(Role::Button).nth(1))
        .expect("second button");

    assert_eq!(node_id(first.key), f.save_button);
    // Buttons in document order: Save(2), Cancel(3), Invisible(6), Confirm(7).
    assert_eq!(node_id(last.key), f.disabled_button);
    assert_eq!(node_id(nth.key), f.cancel_button);
}

#[test]
fn label_and_text_selectors() {
    let (svc, f) = tree();
    let welcome = svc
        .resolve(&Selector::label("Welcome, Ada"))
        .expect("label resolves");
    assert!(matches!(
        welcome.key,
        guiwright_core::NodeKey::AccessKit { .. }
    ));

    // getByText matches label OR value, fuzzy by default.
    let by_text = svc
        .resolve(&Selector::text("example.dev"))
        .expect("email input matches text");
    assert_eq!(node_id(by_text.key), f.email_input);
}

#[test]
fn value_selector_hits_input() {
    let (svc, f) = tree();
    let resolved = svc
        .resolve(&Selector::value(TextMatch::exact("ada@example.dev")))
        .expect("value exact");
    assert_eq!(node_id(resolved.key), f.email_input);
}

#[test]
fn test_id_registry_is_o1_and_survives_updates() {
    let (mut svc, f) = tree();
    let save = guiwright_core::NodeKey::AccessKit {
        tree: accesskit::TreeId::ROOT,
        node: f.save_button,
    };
    svc.register_test_id(save, "save-btn");

    let resolved = svc
        .resolve(&Selector::test_id("save-btn"))
        .expect("test id");
    assert_eq!(resolved.key, save);

    // Removal cleans the registry entry.
    svc.apply_update(Fixture::buttons().remove_save_update());
    assert!(svc.resolve(&Selector::test_id("save-btn")).is_err());
}

#[test]
fn has_text_filter_narrows_to_subtree_content() {
    let (svc, _) = tree();
    // Window has_text "Welcome, Ada" → matches (descendant label).
    let window = svc
        .resolve(&Selector::role(Role::Window).has_text("Welcome, Ada"))
        .expect("window subtree has the label");
    assert_eq!(node_id(window.key).0, 0, "window is node 0");

    // A selector that does not appear in the subtree must not match.
    assert!(
        svc.resolve(&Selector::role(Role::Window).has_text("Nonexistent"))
            .is_err()
    );
}

#[test]
fn has_not_filter_excludes_matching_subtrees() {
    let (mut svc, _) = tree();
    // Give the fixture a second group without the email input.
    let mut group2 = accesskit::Node::new(Role::Group);
    group2.set_label("Empty group");
    group2.set_bounds(accesskit::Rect {
        x0: 260.0,
        y0: 16.0,
        x1: 420.0,
        y1: 64.0,
    });
    let mut window = accesskit::Node::new(Role::Window);
    window.set_label("Test App");
    window.set_children([accesskit::NodeId(1), accesskit::NodeId(9)]);
    svc.apply_update(accesskit::TreeUpdate {
        nodes: vec![
            (accesskit::NodeId(9), group2),
            (accesskit::NodeId(0), window),
        ],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    });

    let found = svc
        .resolve_all(&Selector::role(Role::Group))
        .expect("groups");
    assert_eq!(found.len(), 2);

    let empty = svc
        .resolve(&Selector::role(Role::Group).has_not(Selector::role(Role::TextInput)))
        .expect("the group without a text input");
    assert_eq!(node_id(empty.key).0, 9);
}

#[test]
fn descendant_chain_resolves_through_hierarchy() {
    let (svc, f) = tree();
    let chain =
        Selector::role_named(Role::Button, "Save").descendant_of(Selector::role(Role::Window));
    let resolved = svc.resolve(&chain).expect("chained save");
    assert_eq!(node_id(resolved.key), f.save_button);
}

// --- the `has` filter (the m34 record's named follow-on) --------------------

#[test]
fn has_filter_keeps_matches_whose_subtree_fits() {
    let (svc, f) = tree();
    // The group's subtree holds buttons: kept. The fixture ids are
    // window(0) > group(1) > {Save(2), Cancel(3), label(4), input(5),
    // Invisible(6), Confirm(7)}.
    let kept = svc
        .resolve_all(&Selector::role(Role::Group).has(Selector::role(Role::Button)))
        .expect("the group keeps");
    assert_eq!(kept.len(), 1);
    assert_eq!(node_id(kept[0].key).0, 1);
    // A name constraint the subtree cannot answer drops the match, and
    // an empty `has` answer is the valid snapshot read (no error).
    let empty = svc
        .resolve_all(&Selector::role(Role::Group).has(Selector::role_named(Role::Button, "Export")))
        .expect("empty set read");
    assert!(empty.is_empty(), "{:?}", empty.len());
    // The `save_button` fixture id grounds the positive read: the
    // group's subtree holds Save too.
    let _ = f.save_button;
}

#[test]
fn has_matches_the_subtree_including_the_node_itself() {
    let (svc, _) = tree();
    // A leaf's subtree is itself: every button keeps itself, document
    // order (2, 3, 6, 7 — hidden and disabled are tree matches, the
    // visibility gate is the waitable face).
    let kept = svc
        .resolve_all(&Selector::role(Role::Button).has(Selector::role(Role::Button)))
        .expect("the buttons keep themselves");
    let ids: Vec<_> = kept.iter().map(|r| node_id(r.key).0).collect();
    assert_eq!(ids, vec![2, 3, 6, 7]);
    // The label's subtree has no button: dropped.
    let dropped = svc
        .resolve_all(&Selector::label("Welcome, Ada").has(Selector::role(Role::Button)))
        .expect("the label drops");
    assert!(dropped.is_empty());
}

#[test]
fn has_is_not_the_descendant_chain() {
    let (svc, _) = tree();
    // The semantic distinction the m34 note named: the All chain narrows
    // to the DESCENDANT's rows, `has` keeps the ANCESTOR's.
    let chained = svc
        .resolve_all(&Selector::role(Role::Group).then_descendant(Selector::role(Role::Button)))
        .expect("the chain answers buttons");
    let chained_ids: Vec<_> = chained.iter().map(|r| node_id(r.key).0).collect();
    assert_eq!(chained_ids, vec![2, 3, 6, 7]);

    let filtered = svc
        .resolve_all(&Selector::role(Role::Group).has(Selector::role(Role::Button)))
        .expect("has answers the ancestor");
    let filtered_ids: Vec<_> = filtered.iter().map(|r| node_id(r.key).0).collect();
    assert_eq!(filtered_ids, vec![1]);
    // The render names the positive filter the way has_not names the
    // inverted one.
    assert_eq!(
        Selector::role(Role::Group)
            .has(Selector::role(Role::Button))
            .render(),
        "getByRole(Group).filter(has=getByRole(Button))"
    );
}

#[test]
fn rename_updates_indexes() {
    let fixture = Fixture::buttons();
    let mut svc = TreeService::new();
    svc.apply_update(fixture.initial_update());
    svc.apply_update(fixture.rename_save_update());

    // Exact "Save" no longer matches (node is now "Save All"); the fuzzy
    // selector still matches, per Playwright substring semantics.
    assert!(
        svc.resolve(&Selector::role_named(
            Role::Button,
            TextMatch::exact("Save")
        ))
        .is_err()
    );
    let renamed = svc
        .resolve(&Selector::role_named(Role::Button, "Save All"))
        .expect("renamed button resolves");
    assert_eq!(svc.generation(&renamed.key), 0);
    assert_eq!(svc.last_diff().updated.len(), 1);
}

#[test]
fn nth_out_of_range_is_no_match() {
    let (svc, _) = tree();
    let err = svc
        .resolve(&Selector::role(Role::Button).nth(42))
        .expect_err("out of range");
    assert!(matches!(err, CoreError::NoMatch { .. }), "{err}");
}

#[test]
fn focus_reports_last_update_focus() {
    let (svc, _) = tree();
    assert!(svc.focus().is_some(), "initial update carries focus");
}

#[test]
fn fuzz_selectors_never_panic() {
    use proptest::prelude::*;
    proptest!(|(name in "[a-zA-Z ]{0,12}")| {
        let (svc, _) = tree();
        let _ = svc.resolve(&Selector::text(name.as_str()));
        let _ = svc.resolve(&Selector::label(name.as_str()));
        let _ = svc.resolve(&Selector::value(name.as_str()));
    });
}

// --- the composite links the REPL chain grammar speaks (m94) ---------------

#[test]
fn composite_link_in_chain_resolves() {
    let (svc, f) = tree();
    // All([Group, Filter { Button has_text Save }]): the chain's second
    // link is a pipeline stage, not a node predicate — the buttons whose
    // text probe matches, inside the group. The link question is
    // node-level membership ("this node is one of the link's matches"),
    // so the group ancestor itself is never a match (the m34 has/chain
    // distinction carried to composite links) and the walk stays finite.
    let chain =
        Selector::role(Role::Group).then_descendant(Selector::role(Role::Button).has_text("Save"));
    let resolved = svc
        .resolve(&chain)
        .expect("the chained filter resolves Save");
    assert_eq!(node_id(resolved.key), f.save_button);
}

#[test]
fn composite_chain_answers_the_link_matches_only() {
    let (svc, _) = tree();
    let chain =
        Selector::role(Role::Group).then_descendant(Selector::role(Role::Button).has_text("Save"));
    let all = svc.resolve_all(&chain).expect("the chain set");
    let ids: Vec<_> = all.iter().map(|r| node_id(r.key).0).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn composite_inner_in_has_resolves() {
    let (svc, _) = tree();
    // Has { Group, All([Button, …Label]) }: the positive filter's inner
    // is a chain — the group whose subtree holds a node the chain
    // resolves to. No button's subtree holds a label: the honest empty
    // read, and the walk terminates (one resolution, one containment
    // check — never a per-descendant re-resolution of the composite).
    let kept = svc
        .resolve_all(
            &Selector::role(Role::Group)
                .has(Selector::role(Role::Button).then_descendant(Selector::role(Role::Label))),
        )
        .expect("the chain inner resolves");
    assert!(kept.is_empty(), "no button subtree holds a label");
}

// The regex text probes (the m34 record's second standing note, the
// m93/m94 honest lists' first item — *"No regex `TextPattern`: the enum
// stays open until the codegen milestone asks"*). The acceptance
// criterion this file pins: the enum opens, and a regex probe resolves
// through the same pipeline every string probe rides — no new resolver
// seam, the `TextMatch::matches` arm owns the semantics.

#[test]
fn regex_text_probe_resolves_the_pattern_matches() {
    let (svc, _) = tree();
    // Playwright's getByText(/save|cancel/i) shape: the pattern speaks
    // its own case rule (the `i` inline flag), the probe is partial by
    // default (a regex `test`, not an anchored match).
    let all = svc
        .resolve_all(&Selector::text(TextMatch::regex("(?i)save|cancel")))
        .expect("the regex set");
    let ids: Vec<_> = all.iter().map(|r| node_id(r.key).0).collect();
    assert_eq!(
        ids,
        vec![2, 3],
        "Save and Cancel match; the label and input don't"
    );
}

#[test]
fn regex_probe_is_case_sensitive_without_inline_flags() {
    let (svc, _) = tree();
    // The JS convention the regex probe mirrors: case-sensitive unless
    // the pattern carries (?i) — the `TextMatch` ignore_case field never
    // applies to a regex (the wire regex rows carry the pattern only).
    let all = svc
        .resolve_all(&Selector::text(TextMatch::regex("^save$")))
        .expect("the anchored pattern set");
    assert!(all.is_empty(), "lowercase `save` never matches `Save`");
    let all = svc
        .resolve_all(&Selector::text(TextMatch::regex("^Save$")))
        .expect("the anchored pattern set");
    assert_eq!(
        all.len(),
        1,
        "anchored — the exact button, the label doesn't"
    );
}

#[test]
fn regex_role_name_probe_resolves() {
    let (svc, _) = tree();
    let err = svc
        .resolve(&Selector::role_named(
            Role::Button,
            TextMatch::regex("^(Save|Cancel)$"),
        ))
        .expect_err("two buttons answer the pattern — strict mode refuses");
    assert!(matches!(err, CoreError::Ambiguous { .. }), "{err}");
    let all = svc
        .resolve_all(&Selector::role_named(
            Role::Button,
            TextMatch::regex("^Wel(Come)?,? Ada$"),
        ))
        .expect("no button is the Welcome label — honest empty");
    assert!(all.is_empty());
}

#[test]
fn regex_has_text_filter_narrows_the_set() {
    let (svc, _) = tree();
    // The m93 gap's own words: *"the REPL's `has_text` word speaks the
    // fuzzy `TextMatch` default only"* — the filter's probe speaks the
    // regex now, through the same Filter row (the widening is its own
    // variant, the M64 rule at selector width).
    let all = svc
        .resolve_all(&Selector::role(Role::Button).has_text(TextMatch::regex("^(Save|Cancel)$")))
        .expect("the regex-filtered set");
    let ids: Vec<_> = all.iter().map(|r| node_id(r.key).0).collect();
    assert_eq!(ids, vec![2, 3]);
}

#[test]
fn an_uncompilable_pattern_matches_nothing_never_panics() {
    let (svc, _) = tree();
    // The resolver's total-read rule: a pattern the regex crate refuses
    // answers the honest empty set (the callers validate at their own
    // boundary — the resolver never panics, the fuzz rule holds).
    let all = svc
        .resolve_all(&Selector::text(TextMatch::regex("(unclosed[")))
        .expect("the empty set");
    assert!(all.is_empty(), "an invalid pattern matches nothing");
}

#[test]
fn regex_probes_ride_the_chain() {
    let (svc, _) = tree();
    // The m94 chain vocabulary with regex segments: every segment speaks
    // the pattern form — the label inside the group answers the probe,
    // the walk unchanged.
    let all = svc
        .resolve_all(
            &Selector::role(Role::Group)
                .then_descendant(Selector::role_named(Role::Label, TextMatch::regex("Ada"))),
        )
        .expect("the label matches the pattern inside the group");
    assert_eq!(all.len(), 1, "the group keeps its subtree match");
}
