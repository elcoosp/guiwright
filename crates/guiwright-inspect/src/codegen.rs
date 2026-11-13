//! The interaction → kit-code recorder (§13 "Recording" + §13 codegen).
//!
//! Cost model (§13): each recorded action is emitted against the minimal
//! stable selector — `TestId` > role+accessible-name > label > text —
//! with `.first()` narrowing on ambiguity. Candidates are validated
//! against the live tree (`resolve_all`), so the strict-mode uniqueness
//! check runs while recording: ambiguous selectors become warnings, not
//! broken tests.

use std::fmt::Write;

use guiwright_core::NodeKey;
use guiwright_core::selector::{Selector, TextMatch};
use guiwright_core::tree::TreeService;

/// What the recorder needs to know about the acted-on node. The kit
/// collects this from the tree service at the action tap (§13 "captured
/// at the adapter tap").
#[derive(Clone, Debug, PartialEq)]
pub struct NodeFacts {
    pub key: NodeKey,
    pub role: Option<String>,
    /// Accessible name (role+name selector candidate).
    pub name: Option<String>,
    /// Labelled-by label (`getByLabel` candidate).
    pub label: Option<String>,
    /// Value (`getByValue` candidate).
    pub value: Option<String>,
    /// Registered test id (`getByTestId` candidate — cost 0, §13).
    pub test_id: Option<String>,
    pub bounds: Option<[f64; 4]>,
}

/// One recorded line: the emitted kit code plus the selector that backed
/// it (inspectors show why a locator was chosen).
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedLine {
    pub code: String,
    /// `testid` | `role_name` | `label` | `value` | `text` | `role_first`.
    pub selector_kind: &'static str,
    /// The candidate was ambiguous — narrowed with `.first()` (§13).
    pub narrowed: bool,
}

impl Default for NodeFacts {
    fn default() -> Self {
        Self {
            key: NodeKey::AccessKit {
                tree: accesskit::TreeId::ROOT,
                node: accesskit::NodeId(0),
            },
            role: None,
            name: None,
            label: None,
            value: None,
            test_id: None,
            bounds: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodegenError {
    #[error("no stable selector could be derived for {0}")]
    Unselectable(String),
}

/// The selector cost tiers (§13, lower wins).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CostTier {
    TestId,
    RoleName,
    Label,
    Value,
    Text,
    RoleFirst,
}

/// The interaction → code recorder (§13).
#[derive(Debug, Default)]
pub struct CodegenRecorder {
    lines: Vec<RecordedLine>,
    warnings: Vec<String>,
}

impl CodegenRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one interaction. `action` is the kit vocabulary
    /// (`click`/`fill`/`hover`/…); `argument` carries `fill` payloads.
    /// The candidate selector is validated against `tree` (uniqueness +
    /// strict-mode warning, §13) — scoped to `pinned`'s subtree when the
    /// action rode a window handle, so uniqueness is judged where the
    /// action actually resolved (§16 v0.4). The handle also shapes the
    /// emitted line: `app.window("…").get_by_…`.
    pub fn record(
        &mut self,
        action: &str,
        argument: Option<&str>,
        facts: &NodeFacts,
        tree: &TreeService,
        pinned: Option<(&str, &NodeKey)>,
    ) -> Result<RecordedLine, CodegenError> {
        let choice = choose_selector(tree, facts, pinned.map(|(_, root)| root))?;
        let narrowed = choice.narrowed;
        let kind = choice.tier.kind();
        if narrowed {
            self.warnings.push(format!(
                "selector `{}` resolved to more than one node — recorded with .first()",
                choice.selector.render()
            ));
        }
        let locator = match pinned {
            // The window-handle surface (§16 v0.4): aim through
            // `app.window(handle)` — the DSL shape the recorded code
            // replays. `locator_code` always emits an `app.`-rooted
            // expression; the prefix swaps the root for the handle chain.
            Some((handle, _)) => {
                let body = locator_code(&choice, facts);
                match body.strip_prefix("app.") {
                    Some(rest) => format!("app.window({handle:?}).{rest}"),
                    None => body,
                }
            }
            None => locator_code(&choice, facts),
        };
        // M4 (§13 remote codegen): `fill` and `press` carry their payload
        // as the recorder argument — the emitted call reads like the kit
        // API (`locator.fill("…")`, `locator.press("Enter")`).
        let call = match (action, argument) {
            ("fill", Some(arg)) => format!("{locator}.fill({arg:?});"),
            ("press", Some(arg)) => format!("{locator}.press({arg:?});"),
            _ => format!("{locator}.{action}();"),
        };
        let line = RecordedLine {
            code: call,
            selector_kind: kind,
            narrowed,
        };
        self.lines.push(line.clone());
        Ok(line)
    }

    /// Record one pre-rendered raw-input line (§13 over §9): the
    /// gesture vocabulary has no selector to validate — the driver's
    /// renderer composes the call (`app.mouse().click(10, 20);`), the
    /// recorder lands it verbatim with `selector_kind = "raw"`.
    pub fn record_raw(&mut self, line: impl Into<String>) -> RecordedLine {
        let line = RecordedLine {
            code: line.into(),
            selector_kind: "raw",
            narrowed: false,
        };
        self.lines.push(line.clone());
        line
    }

    /// Strict-mode warnings collected while recording (§13).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Push a warning (used by the kit when a recorded target no longer
    /// resolves).
    pub fn warn(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }

    /// The generated kit code (one action per line).
    pub fn code(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&line.code);
            out.push('\n');
        }
        out
    }

    /// Consuming finish (§7 `Inspect::StopRecording` shape).
    pub fn finish(self) -> (String, Vec<String>) {
        let code = self.code();
        (code, self.warnings)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// The chosen selector: minimal cost that is live in the tree. `tier`
/// is the §13 cost rank AND the recorded-line label source (`kind`).
#[derive(Clone, Debug)]
struct Choice {
    selector: Selector,
    narrowed: bool,
    tier: CostTier,
}

impl CostTier {
    /// The `RecordedLine::selector_kind` label for this tier.
    fn kind(self) -> &'static str {
        match self {
            CostTier::TestId => "testid",
            CostTier::RoleName => "role_name",
            CostTier::Label => "label",
            CostTier::Value => "value",
            CostTier::Text => "text",
            CostTier::RoleFirst => "role_first",
        }
    }
}

/// §13 cost model: `TestId` > role+accessible-name > label > text, with
/// `.first()` narrowing on ambiguity. Every candidate is verified against
/// the live tree — a candidate that does not resolve is skipped (the
/// stable selector must survive re-runs, not just look good once).
fn choose_selector(
    tree: &TreeService,
    facts: &NodeFacts,
    scope: Option<&NodeKey>,
) -> Result<Choice, CodegenError> {
    // Tier 0 — TestId (O(1) hash, survives UI copy changes, §13).
    if let Some(id) = &facts.test_id {
        let selector = Selector::test_id(id.clone());
        return Ok(Choice {
            selector,
            narrowed: false,
            tier: CostTier::TestId,
        });
    }
    // Tier 1 — role + accessible name.
    if let (Some(role), Some(name)) = (facts.role.as_deref(), facts.name.as_deref()) {
        if let Some(role) = parse_role(role) {
            let selector = Selector::role_named(role, TextMatch::exact(name));
            if let Some(choice) = checked(tree, selector, CostTier::RoleName, scope) {
                return Ok(choice);
            }
        }
    }
    // Tier 2 — label.
    if let Some(label) = facts.label.as_deref() {
        let selector = Selector::label(TextMatch::exact(label));
        if let Some(choice) = checked(tree, selector, CostTier::Label, scope) {
            return Ok(choice);
        }
    }
    // Tier 2.5 — value (inputs: the value often outlives labels).
    if let Some(value) = facts.value.as_deref() {
        let selector = Selector::value(TextMatch::exact(value));
        if let Some(choice) = checked(tree, selector, CostTier::Value, scope) {
            return Ok(choice);
        }
    }
    // Tier 3 — text content.
    if let Some(name) = facts.name.as_deref() {
        let selector = Selector::text(TextMatch::exact(name));
        if let Some(choice) = checked(tree, selector, CostTier::Text, scope) {
            return Ok(choice);
        }
    }
    // Tier 4 — bare role, narrowed with `.first()` (warning).
    if let Some(role) = facts.role.as_deref().and_then(parse_role) {
        let selector = Selector::role(role).first();
        return Ok(Choice {
            selector,
            narrowed: true,
            tier: CostTier::RoleFirst,
        });
    }
    Err(CodegenError::Unselectable(
        facts.role.clone().unwrap_or_else(|| "unknown node".into()),
    ))
}

/// Validate a candidate: resolvable against the live tree — within the
/// window scope when one is pinned (§16 v0.4, the same subtree the action
/// resolved in) — unique or narrowed explicitly (§13 strict-mode warnings
/// while recording).
fn checked(
    tree: &TreeService,
    selector: Selector,
    tier: CostTier,
    scope: Option<&NodeKey>,
) -> Option<Choice> {
    match tree.resolve_all_scoped(&selector, scope) {
        Ok(matches) if !matches.is_empty() => {
            let narrowed = matches.len() > 1;
            let selector = if narrowed { selector.first() } else { selector };
            Some(Choice {
                selector,
                narrowed,
                tier,
            })
        }
        _ => None,
    }
}

/// Kit code for the chosen locator: the cheapest reader-facing form.
/// Narrowing (`.first()`/`.nth(k)`) rides a `Filter` wrapper — unwrapped
/// here so the emitted code reads like the kit API (§13).
fn locator_code(choice: &Choice, _facts: &NodeFacts) -> String {
    let (base, nth) = match &choice.selector {
        Selector::Filter {
            source,
            nth: Some(nth),
            ..
        } => (source.as_ref(), Some(nth.clone())),
        s => (s, None),
    };
    let mut code = match base {
        Selector::TestId(id) => format!("app.get_by_test_id({id:?})"),
        Selector::Role {
            role,
            name: Some(matched),
        } => {
            let name = matched.pattern.as_str().to_string();
            let role_name = role_debug_name(*role);
            format!("app.get_by_role(Role::{role_name}, {name:?})")
        }
        Selector::Role { role, name: None } => {
            let role_name = role_debug_name(*role);
            format!("app.role_locator(Role::{role_name})")
        }
        Selector::Label(m) => format!("app.get_by_label({:?})", m.pattern.as_str()),
        Selector::Text(m) => format!("app.get_by_text({:?})", m.pattern.as_str()),
        Selector::Value(m) => format!("app.get_by_value({:?})", m.pattern.as_str()),
        _ => format!("app.locator({:?})", base.render()),
    };
    match nth {
        Some(guiwright_core::selector::Nth::First) => code.push_str(".first()"),
        Some(guiwright_core::selector::Nth::Last) => code.push_str(".last()"),
        Some(guiwright_core::selector::Nth::Index(i)) => {
            let _ = write!(code, ".nth({i})");
        }
        None => {}
    }
    code
}

/// `accesskit::Role` → kit `Role::` variant name (Debug names line up).
fn role_debug_name(role: accesskit::Role) -> String {
    format!("{role:?}")
}

/// Recreate the role from its Debug name (the recorder round-trips the
/// same vocabulary the tree reports).
fn parse_role(name: &str) -> Option<accesskit::Role> {
    // accesskit::Role has ~90 variants; the tree's Debug names are the
    // variant names. Parse via a tiny lookup over the most common
    // interactive roles — the full set rides the tree, codegen only ever
    // sees roles that were in it.
    const ROLES: &[(&str, accesskit::Role)] = &[
        ("Button", accesskit::Role::Button),
        ("CheckBox", accesskit::Role::CheckBox),
        ("RadioButton", accesskit::Role::RadioButton),
        ("TextInput", accesskit::Role::TextInput),
        ("SearchInput", accesskit::Role::SearchInput),
        ("MultilineTextInput", accesskit::Role::MultilineTextInput),
        ("Slider", accesskit::Role::Slider),
        ("Switch", accesskit::Role::Switch),
        ("Tab", accesskit::Role::Tab),
        ("MenuItem", accesskit::Role::MenuItem),
        ("MenuListOption", accesskit::Role::MenuListOption),
        ("ListBoxOption", accesskit::Role::ListBoxOption),
        ("ComboBox", accesskit::Role::ComboBox),
        ("Link", accesskit::Role::Link),
        ("Label", accesskit::Role::Label),
        ("Heading", accesskit::Role::Heading),
        ("List", accesskit::Role::List),
        ("ListItem", accesskit::Role::ListItem),
        ("Dialog", accesskit::Role::Dialog),
        ("Alert", accesskit::Role::Alert),
        ("Image", accesskit::Role::Image),
        ("Pane", accesskit::Role::Pane),
        ("Window", accesskit::Role::Window),
        ("Toolbar", accesskit::Role::Toolbar),
        ("TabList", accesskit::Role::TabList),
    ];
    ROLES.iter().find(|(n, _)| *n == name).map(|(_, r)| *r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
    use guiwright_core::test_util::Fixture;

    fn tree_with(update: TreeUpdate) -> TreeService {
        let mut svc = TreeService::new();
        svc.apply_update(update);
        svc
    }

    fn fixture_tree() -> TreeService {
        tree_with(Fixture::buttons().initial_update())
    }

    fn node(id: u64) -> NodeKey {
        NodeKey::AccessKit {
            tree: TreeId::ROOT,
            node: NodeId(id),
        }
    }

    /// A minimal tree: window → listed children (builder API).
    fn simple_tree(children: &[(u64, Node)]) -> TreeService {
        let ids: Vec<NodeId> = children.iter().map(|(id, _)| NodeId(*id)).collect();
        let mut root = Node::new(Role::Window);
        root.set_children(ids.clone());
        let mut update = TreeUpdate {
            nodes: vec![(NodeId(1), root)],
            tree: Some(TreeInfo {
                root: NodeId(1),
                toolkit_name: Some("codegen-test".into()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: NodeId(1),
        };
        update
            .nodes
            .extend(children.iter().map(|(id, n)| (NodeId(*id), n.clone())));
        tree_with(update)
    }

    fn named_node(role: Role, name: &str) -> Node {
        let mut n = Node::new(role);
        n.set_label(name);
        n
    }

    fn facts(id: u64, role: Role, name: &str) -> NodeFacts {
        NodeFacts {
            key: node(id),
            role: Some(format!("{role:?}")),
            name: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn role_name_selector_when_no_test_id() {
        let tree = fixture_tree();
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record("click", None, &facts(2, Role::Button, "Save"), &tree, None)
            .expect("recorded");
        assert_eq!(
            line.code,
            "app.get_by_role(Role::Button, \"Save\").click();"
        );
        assert_eq!(line.selector_kind, "role_name");
        assert!(!line.narrowed);
    }

    #[test]
    fn test_id_wins_the_cost_model() {
        let mut tree = fixture_tree();
        tree.register_test_id(node(2), "save-btn");
        let mut fact = facts(2, Role::Button, "Save");
        fact.test_id = Some("save-btn".into());
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record("click", None, &fact, &tree, None)
            .expect("recorded");
        assert_eq!(line.code, "app.get_by_test_id(\"save-btn\").click();");
        assert_eq!(line.selector_kind, "testid");
    }

    #[test]
    fn fill_emits_fill_with_argument() {
        let tree = fixture_tree();
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record(
                "fill",
                Some("ada@lovelace.dev"),
                &facts(5, Role::TextInput, "Email"),
                &tree,
                None,
            )
            .expect("recorded");
        assert_eq!(
            line.code,
            "app.get_by_role(Role::TextInput, \"Email\").fill(\"ada@lovelace.dev\");"
        );
    }

    /// M4 (§13 remote codegen): `press` carries the key as the recorder
    /// argument, same call shape as `fill`.
    #[test]
    fn press_emits_press_with_argument() {
        let tree = fixture_tree();
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record(
                "press",
                Some("Enter"),
                &facts(5, Role::TextInput, "Email"),
                &tree,
                None,
            )
            .expect("recorded");
        assert_eq!(
            line.code,
            "app.get_by_role(Role::TextInput, \"Email\").press(\"Enter\");"
        );
    }

    #[test]
    fn ambiguous_candidate_is_narrowed_with_warning() {
        // Two buttons share the accessible name "Save".
        let tree = simple_tree(&[
            (2, named_node(Role::Button, "Save")),
            (3, named_node(Role::Button, "Save")),
        ]);
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record("click", None, &facts(2, Role::Button, "Save"), &tree, None)
            .expect("recorded");
        assert!(line.narrowed, "ambiguous → narrowed");
        assert!(line.code.contains(".first()"), "{}", line.code);
        assert!(
            rec.warnings()
                .iter()
                .any(|w| w.contains("resolved to more than one node")),
            "{:?}",
            rec.warnings()
        );
    }

    #[test]
    fn text_fallback_for_non_interactive_roles() {
        // A TextRun node: not in the interactive-role table, so the cost
        // model falls through to `get_by_text` (§13 tier 3).
        let tree = simple_tree(&[(2, named_node(Role::TextRun, "Welcome, Ada"))]);
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record(
                "click",
                None,
                &facts(2, Role::TextRun, "Welcome, Ada"),
                &tree,
                None,
            )
            .expect("recorded");
        assert_eq!(line.selector_kind, "text");
        assert_eq!(line.code, "app.get_by_text(\"Welcome, Ada\").click();");
        assert!(!line.narrowed);
    }

    #[test]
    fn text_selector_when_role_not_interactive() {
        // A Label node whose role_name selector resolves: codegen uses it
        // directly (cost tier 1) even for non-interactive roles.
        let tree = simple_tree(&[(2, named_node(Role::Label, "Welcome, Ada"))]);
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record(
                "click",
                None,
                &facts(2, Role::Label, "Welcome, Ada"),
                &tree,
                None,
            )
            .expect("recorded");
        assert_eq!(line.selector_kind, "role_name", "{}", line.code);
    }

    /// Raw-input vocabulary (§13 over §9): pre-rendered lines land
    /// verbatim, `selector_kind` = `"raw"`, no narrowing and no tree
    /// validation (there is no selector to validate).
    #[test]
    fn record_raw_lands_the_line_verbatim() {
        let tree = fixture_tree();
        let mut rec = CodegenRecorder::new();
        rec.record("click", None, &facts(2, Role::Button, "Save"), &tree, None)
            .expect("locator line");
        let line = rec.record_raw("app.mouse().click(10, 20);");
        assert_eq!(line.code, "app.mouse().click(10, 20);");
        assert_eq!(line.selector_kind, "raw");
        assert!(!line.narrowed);
        let (code, warnings) = rec.finish();
        assert_eq!(
            code,
            "app.get_by_role(Role::Button, \"Save\").click();\napp.mouse().click(10, 20);\n"
        );
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn finish_returns_code_and_warnings() {
        let tree = fixture_tree();
        let mut rec = CodegenRecorder::new();
        rec.record("click", None, &facts(2, Role::Button, "Save"), &tree, None)
            .expect("1");
        rec.record(
            "hover",
            None,
            &facts(7, Role::Button, "Confirm"),
            &tree,
            None,
        )
        .expect("2");
        assert!(!rec.is_empty());
        let (code, warnings) = rec.finish();
        assert_eq!(warnings.len(), 0);
        assert_eq!(code.lines().count(), 2);
        assert!(code.contains("Save"));
        assert!(code.contains("Confirm"));
    }

    #[test]
    fn role_first_emits_narrowed_locator_when_name_unresolvable() {
        // A named role whose name does not resolve to any node (the app
        // renamed it) → falls to role + .first() narrowing with a warning.
        let tree = simple_tree(&[(2, named_node(Role::Button, "Renamed"))]);
        let mut rec = CodegenRecorder::new();
        let line = rec
            .record(
                "click",
                None,
                &facts(2, Role::Button, "Stale Name"),
                &tree,
                None,
            )
            .expect("recorded");
        assert_eq!(line.selector_kind, "role_first");
        assert!(line.narrowed);
        assert!(line.code.contains("role_locator"), "{}", line.code);
        assert_eq!(rec.warnings().len(), 1, "{:?}", rec.warnings());
    }

    #[test]
    fn unselectable_nodes_error_cleanly() {
        // No name, no test id, unknown role: nothing stable to emit.
        let tree = simple_tree(&[(2, Node::new(Role::Document))]);
        let mut rec = CodegenRecorder::new();
        let err = rec
            .record("click", None, &facts(2, Role::Document, ""), &tree, None)
            .expect_err("nothing to select with");
        assert!(err.to_string().contains("Document"), "{err}");
    }
}
