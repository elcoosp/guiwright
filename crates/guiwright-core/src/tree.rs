//! Tree service: one incremental [`accesskit_consumer`] tree plus per-frame
//! locator indexes, generation counters and the diff fan-out point
//! (architecture §15.2 "one diff, many consumers").
//!
//! Adapters feed [`accesskit::TreeUpdate`]s in (egui emits full rebuilds per
//! pass; masonry/freya/slint emit incremental ones) — the consumer's
//! [`accesskit_consumer::TreeChangeHandler`] turns every update into an
//! O(changes) diff which drives the indexes, waiters, traces and remote
//! subscribers alike.

use std::collections::HashMap;

use accesskit::{Point, Role, TreeUpdate};
use accesskit_consumer::{NodeRef, Tree, TreeChangeHandler, common_filter};

use crate::error::{CoreError, NodeDesc, TreeSummary};
use crate::key::NodeKey;
use crate::selector::{Nth, Selector, TextMatch, normalize};

/// Index entry captured per node when a diff is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
struct IndexEntry {
    role: Role,
    label: Option<String>,
    value: Option<String>,
}

impl IndexEntry {
    fn from_node(node: &NodeRef<'_>) -> Self {
        Self {
            role: node.role(),
            label: node.label().map(|l| normalize(&l)),
            value: node.value().map(|v| normalize(&v)),
        }
    }
}

#[derive(Default, Debug)]
struct Index {
    /// Document order of every live node (refreshed per diff).
    all: Vec<NodeKey>,
    by_role: HashMap<Role, Vec<NodeKey>>,
    /// Exact accessible-name lookup: normalized lowercase name → nodes.
    by_role_name: HashMap<(Role, String), Vec<NodeKey>>,
    /// Normalized accessible label → nodes (getByLabel / getByText).
    by_label: HashMap<String, Vec<NodeKey>>,
    /// Normalized value → nodes (getByValue / getByText).
    by_value: HashMap<String, Vec<NodeKey>>,
    /// Explicit test-id registry (`getByTestId`, O(1) hash, §8.1).
    test_ids: HashMap<String, NodeKey>,
}

impl Index {
    fn add(&mut self, key: NodeKey, entry: &IndexEntry) {
        self.all.push(key);
        self.by_role.entry(entry.role).or_default().push(key);
        if let Some(label) = &entry.label {
            self.by_role_name
                .entry((entry.role, label.to_lowercase()))
                .or_default()
                .push(key);
            self.by_label.entry(label.clone()).or_default().push(key);
        }
        if let Some(value) = &entry.value {
            self.by_value.entry(value.clone()).or_default().push(key);
        }
    }

    fn remove(&mut self, key: NodeKey, entry: &IndexEntry) {
        self.all.retain(|k| *k != key);
        if let Some(v) = self.by_role.get_mut(&entry.role) {
            v.retain(|k| *k != key);
        }
        if let Some(label) = &entry.label {
            if let Some(v) = self
                .by_role_name
                .get_mut(&(entry.role, label.to_lowercase()))
            {
                v.retain(|k| *k != key);
            }
            if let Some(v) = self.by_label.get_mut(label) {
                v.retain(|k| *k != key);
            }
        }
        if let Some(value) = &entry.value {
            if let Some(v) = self.by_value.get_mut(value) {
                v.retain(|k| *k != key);
            }
        }
        self.test_ids.retain(|_, k| *k != key);
    }
}

/// Summary of the last applied diff — the single fan-out payload that wakes
/// waiters, traces and remote subscribers (§15.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeDiffSummary {
    pub added: Vec<NodeKey>,
    pub updated: Vec<NodeKey>,
    pub removed: Vec<NodeKey>,
    pub focus_changed: bool,
}

impl TreeDiffSummary {
    /// Did anything change that could affect locator resolution?
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// A resolved locator: identity + generation + current bounds.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub key: NodeKey,
    pub generation: u32,
    /// Window-relative bounds in physical pixels (AccessKit convention,
    /// §9 "AccessKit bounds are already physical").
    pub bounds: Option<accesskit::Rect>,
}

/// Which update was presented last (§10 `FramePresented { seq, hash }`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub seq: u64,
    /// Cheap visual-change signal: xxh3 of the frame digest.
    pub hash: u64,
}

/// One window target (§16 v0.4 "multi-window / tabs — targets per window",
/// M6). AccessKit models multi-window apps as **grafted subtrees**: a window
/// node in the root tree carries a `tree_id` property naming the subtree
/// that holds the window's content. The handle is that subtree's [`TreeId`]
/// string — the same identity [`crate::NodeKey::AccessKit`] already carries,
/// so keys never collide across windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowScope {
    /// Stable handle: the subtree [`TreeId`] (uuid string), or `"main"` for
    /// a graft-free (single-window) tree.
    pub handle: String,
    /// The window node itself (the graft node in the root tree; the tree
    /// root for `"main"`). Scoped resolution matches this node and its
    /// descendants.
    pub root: NodeKey,
    /// The window title (the window node's accessible name).
    pub title: Option<String>,
}

/// The framework-agnostic tree brain (§5.2 `guiwright-core`).
#[derive(Debug)]
pub struct TreeService {
    tree: Option<Tree>,
    /// Updates received before the first update carrying `TreeInfo`
    /// (the consumer tree cannot be initialized without it).
    pending: Vec<TreeUpdate>,
    index: Index,
    /// ABA guard: increments every time an identity leaves the tree (§8.2).
    generations: HashMap<NodeKey, u32>,
    last_diff: TreeDiffSummary,
    frame_seq: u64,
    last_frame: Option<FrameInfo>,
    /// Active window scope (§16 v0.4, M6): when set, resolution only sees
    /// the scope subtree. Session-owned transient state — set around a
    /// scoped request, cleared after (the session is the only writer, the
    /// rendezvous model keeps this single-threaded).
    scope: Option<NodeKey>,
}

impl Default for TreeService {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeService {
    pub fn new() -> Self {
        Self {
            tree: None,
            pending: Vec::new(),
            index: Index::default(),
            generations: HashMap::new(),
            last_diff: TreeDiffSummary::default(),
            frame_seq: 0,
            last_frame: None,
            scope: None,
        }
    }

    // ------------------------------------------------------------------
    // Update path (one diff, many consumers, §15.2)

    /// Apply an adapter [`TreeUpdate`]; returns the diff summary for waiters
    /// and subscribers. Full rebuilds (egui) cost O(changes) here.
    pub fn apply_update(&mut self, update: TreeUpdate) -> TreeDiffSummary {
        if self.tree.is_none() {
            if update.tree.is_none() {
                self.pending.push(update);
                return TreeDiffSummary::default();
            }
            // A full update supersedes any diffs received before
            // initialization (incremental adapters always open with a full
            // update; egui-style adapters emit full rebuilds anyway).
            if !self.pending.is_empty() {
                tracing::debug!(
                    count = self.pending.len(),
                    "dropping pre-init diffs superseded by the full update"
                );
                self.pending.clear();
            }
            let tree = Tree::new(update, false);
            let mut summary = TreeDiffSummary::default();
            // Initial build: index the whole tree in document order.
            let root = tree.state().root();
            walk(root, &mut |key, node| {
                let entry = IndexEntry::from_node(node);
                self.index.add(key, &entry);
                summary.added.push(key);
            });
            self.tree = Some(tree);
            self.last_diff = summary.clone();
            return summary;
        }

        let mut handler = DiffCollector::default();
        self.tree
            .as_mut()
            .expect("tree exists")
            .update_and_process_changes(update, &mut handler);

        let mut summary = TreeDiffSummary::default();
        // Drain pending updates now that the tree exists — no: tree exists,
        // pending must have been empty.
        for (key, old, new) in std::mem::take(&mut handler.updated) {
            self.index.remove(key, &old);
            self.index.add(key, &new);
            summary.updated.push(key);
        }
        for (key, old) in std::mem::take(&mut handler.removed) {
            self.index.remove(key, &old);
            let slot = self.generations.entry(key).or_insert(0);
            *slot = slot.saturating_add(1);
            summary.removed.push(key);
        }
        for (key, entry) in std::mem::take(&mut handler.added) {
            self.index.add(key, &entry);
            summary.added.push(key);
        }
        if handler.focus_changed {
            summary.focus_changed = true;
        }
        self.last_diff = summary.clone();
        summary
    }

    /// The diff of the most recently applied update.
    pub fn last_diff(&self) -> &TreeDiffSummary {
        &self.last_diff
    }

    /// Present a frame (§10). Returns its seq/hash for waiters.
    pub fn frame_presented(&mut self, hash: u64) -> FrameInfo {
        self.frame_seq += 1;
        let info = FrameInfo {
            seq: self.frame_seq,
            hash,
        };
        self.last_frame = Some(info);
        info
    }

    pub fn last_frame(&self) -> Option<FrameInfo> {
        self.last_frame
    }

    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }

    // ------------------------------------------------------------------
    // Lookups

    /// Borrow the node for `key` (if live in the current tree).
    pub fn node(&self, key: &NodeKey) -> Option<NodeRef<'_>> {
        let NodeKey::AccessKit { tree, node } = *key else {
            return None;
        };
        self.tree
            .as_ref()?
            .state()
            .node_by_tree_local_id(node, tree)
    }

    /// Current generation of `key`.
    pub fn generation(&self, key: &NodeKey) -> u32 {
        self.generations.get(key).copied().unwrap_or(0)
    }

    /// Validate a [`Resolved`] against ABA recycling (§8.2 generation guard).
    pub fn check_generation(&self, resolved: &Resolved) -> Result<(), CoreError> {
        if self.generation(&resolved.key) != resolved.generation {
            return Err(CoreError::StaleNode {
                key: resolved.key,
                generation: resolved.generation,
            });
        }
        Ok(())
    }

    /// Hit-test the visible tree at a window-relative physical point (§8.3
    /// "receives events" mechanism).
    pub fn hit_test(&self, point: Point) -> Option<NodeKey> {
        let root = self.tree.as_ref()?.state().root();
        root.node_at_point(point, &common_filter)
            .map(|n| node_key(&n))
    }

    /// Current focus (tree-local; host focus state is `false` for headless
    /// sessions so this reports the adapter-reported focus).
    pub fn focus(&self) -> Option<NodeKey> {
        let tree = self.tree.as_ref()?;
        let focus = tree.state().focus_id_in_tree();
        tree.state().node_by_id(focus).map(|n| node_key(&n))
    }

    /// All live node keys in document order.
    pub fn all_nodes(&self) -> &[NodeKey] {
        &self.index.all
    }

    pub fn node_count(&self) -> usize {
        self.index.all.len()
    }

    /// Register an explicit test id for a node (`getByTestId`, §8.1).
    /// Dropped automatically when the node leaves the tree.
    pub fn register_test_id(&mut self, key: NodeKey, id: impl Into<String>) {
        self.index.test_ids.insert(id.into(), key);
    }

    /// The registered test id of a node, if any (§13 codegen cost model —
    /// `TestId` is the cheapest stable selector). Cold path: the registry
    /// is small and the lookup runs once per recorded interaction.
    pub fn test_id_of(&self, key: &NodeKey) -> Option<&str> {
        self.index
            .test_ids
            .iter()
            .find(|(_, k)| *k == key)
            .map(|(id, _)| id.as_str())
    }

    /// Visit every descendant (depth-first, excluding the node itself).
    pub fn descendants(&self, key: &NodeKey, visit: &mut impl FnMut(NodeKey, &NodeRef<'_>)) {
        let Some(node) = self.node(key) else {
            return;
        };
        walk_descendants(node, visit);
    }

    // ------------------------------------------------------------------
    // Windows (§16 v0.4 multi-window, M6)

    /// Enumerate the window targets of the current tree (§16 v0.4 "targets
    /// per window"). Grafted subtrees are the windows; a graft-free tree is
    /// one `main` window rooted at the tree root. An uninitialized tree has
    /// no windows yet.
    pub fn windows(&self) -> Vec<WindowScope> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let state = tree.state();
        let mut windows = Vec::new();
        walk(state.root(), &mut |key, node| {
            if !node.is_graft() {
                return;
            }
            if let Some(subtree) = node.data().tree_id() {
                windows.push(WindowScope {
                    handle: subtree.0.to_string(),
                    root: key,
                    title: node.label(),
                });
            }
        });
        if windows.is_empty() {
            // Graft-free (single-window) app: the whole tree is one window.
            let root = state.root();
            return vec![WindowScope {
                handle: crate::key::MAIN_WINDOW.to_string(),
                root: node_key(&root),
                title: root.label(),
            }];
        }
        windows
    }

    /// Map a window handle to its scope root (the GWP `Window::Use`
    /// validation, §7). `Ok(None)` = no filter (the `main` window of a
    /// graft-free tree IS the whole tree); `Err(UnknownWindow)` for handles
    /// the tree does not know.
    pub fn window_scope(&self, handle: &str) -> Result<Option<NodeKey>, CoreError> {
        if handle.is_empty() {
            return Ok(None);
        }
        for window in self.windows() {
            if window.handle == handle {
                return Ok(if window.handle == crate::key::MAIN_WINDOW {
                    None
                } else {
                    Some(window.root)
                });
            }
        }
        Err(CoreError::UnknownWindow {
            handle: handle.to_string(),
        })
    }

    /// Is `key` the scope root or one of its descendants?
    fn is_within(&self, key: &NodeKey, root: &NodeKey) -> bool {
        if key == root {
            return true;
        }
        match (self.node(key), self.node(root)) {
            (Some(node), Some(scope_root)) => node.is_descendant_of(&scope_root),
            _ => false,
        }
    }

    /// Narrow resolution to a window subtree: session-owned transient state
    /// set around a scoped request and cleared after (§16 v0.4). Every
    /// resolve path — including waiters and `expect`, which re-resolve on
    /// world events — honors the scope until it is cleared.
    pub fn set_scope(&mut self, scope: Option<NodeKey>) {
        self.scope = scope;
    }

    /// The active resolution scope, if one is set.
    pub fn scope(&self) -> Option<&NodeKey> {
        self.scope.as_ref()
    }

    /// Resolve with the strict-mode policy inside an explicit scope —
    /// `resolve` with the session's window filter applied for one call
    /// (§16 v0.4). `None` scope = unscoped.
    pub fn resolve_scoped(
        &self,
        selector: &Selector,
        scope: Option<&NodeKey>,
    ) -> Result<Resolved, CoreError> {
        let matches = self.resolve_all_scoped(selector, scope)?;
        self.strict(selector, matches)
    }

    /// [`Self::resolve_all`] restricted to an explicit scope.
    pub fn resolve_all_scoped(
        &self,
        selector: &Selector,
        scope: Option<&NodeKey>,
    ) -> Result<Vec<Resolved>, CoreError> {
        let keys = self.candidate_keys(selector)?;
        Ok(keys
            .into_iter()
            .filter(|key| scope.is_none_or(|root| self.is_within(key, root)))
            .map(|key| Resolved {
                key,
                generation: self.generation(&key),
                bounds: self.node(&key).and_then(|n| n.bounding_box()),
            })
            .collect())
    }

    /// The strict-mode uniqueness policy shared by [`Self::resolve`] and
    /// [`Self::resolve_scoped`].
    fn strict(&self, selector: &Selector, matches: Vec<Resolved>) -> Result<Resolved, CoreError> {
        match matches.len() {
            0 => Err(CoreError::NoMatch {
                selector: selector.render(),
                last: Box::new(self.summary()),
            }),
            1 => Ok(matches.into_iter().next().expect("len == 1")),
            _ => Err(CoreError::Ambiguous {
                selector: selector.render(),
                matches: matches
                    .iter()
                    .map(|m| self.describe(&m.key))
                    .collect::<Vec<_>>(),
            }),
        }
    }

    // ------------------------------------------------------------------
    // Resolution pipeline (§8.2)

    /// Resolve with the strict-mode uniqueness policy: exactly one match.
    /// Honors the session's window scope when one is set (§16 v0.4) — the
    /// same filter the waiters and `expect` see.
    pub fn resolve(&self, selector: &Selector) -> Result<Resolved, CoreError> {
        let matches = self.resolve_all(selector)?;
        self.strict(selector, matches)
    }

    /// Resolve returning *all* matches in document order (no strict check).
    /// Used by filters, waiters and the codegen uniqueness probe (§13).
    /// Honors the session's window scope when one is set (§16 v0.4, M6).
    pub fn resolve_all(&self, selector: &Selector) -> Result<Vec<Resolved>, CoreError> {
        let keys = self.candidate_keys(selector)?;
        Ok(keys
            .into_iter()
            .filter(|key| {
                self.scope
                    .as_ref()
                    .is_none_or(|root| self.is_within(key, root))
            })
            .map(|key| Resolved {
                key,
                generation: self.generation(&key),
                bounds: self.node(&key).and_then(|n| n.bounding_box()),
            })
            .collect())
    }

    /// Index-first candidate generation (§8.2 step 1): one arm per
    /// selector family, each extracted into its own `*_candidates` method
    /// so the dispatch stays readable and every index stays next to the
    /// query that owns it.
    fn candidate_keys(&self, selector: &Selector) -> Result<Vec<NodeKey>, CoreError> {
        if self.tree.is_none() {
            return Ok(Vec::new());
        }
        match selector {
            Selector::Role { role, name } => Ok(self.role_candidates(*role, name.as_ref())),
            Selector::Label(m) => Ok(self.label_candidates(m)),
            Selector::Text(m) => Ok(self.text_candidates(m)),
            Selector::TestId(id) => Ok(self
                .index
                .test_ids
                .get(id)
                .copied()
                .map(|k| vec![k])
                .unwrap_or_default()),
            Selector::Value(m) => Ok(self.value_candidates(m)),
            Selector::All(chain) => self.all_candidates(chain),
            Selector::Filter {
                source,
                has_text,
                has_not,
                nth,
            } => {
                self.filter_candidates(source, has_text.as_ref(), has_not.as_deref(), nth.as_ref())
            }
        }
    }

    /// `getByRole` candidates: the role bucket, narrowed by the name
    /// match (exact case-sensitive scans, the case-insensitive hash, or
    /// the pattern filter — §8.2).
    fn role_candidates(&self, role: Role, name: Option<&TextMatch>) -> Vec<NodeKey> {
        let Some(m) = name else {
            return self.index.by_role.get(&role).cloned().unwrap_or_default();
        };
        let mut out = match m {
            m if m.exact && !m.ignore_case => {
                // Exact case-sensitive: hash lookup misses case
                // variants, so scan the role bucket.
                self.index
                    .by_role
                    .get(&role)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|k| self.role_name_matches(k, m))
                    .collect()
            }
            m if m.exact && m.ignore_case => self
                .index
                .by_role_name
                .get(&(role, m.pattern.as_str().to_lowercase()))
                .cloned()
                .unwrap_or_default(),
            m => self
                .index
                .by_role
                .get(&role)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|k| self.role_name_matches(k, m))
                .collect(),
        };
        out.sort_dedup();
        out
    }

    /// `getByLabel` candidates: the label hash for the exact
    /// case-insensitive path, the whole label index otherwise (§8.2).
    fn label_candidates(&self, m: &TextMatch) -> Vec<NodeKey> {
        let mut out = if m.exact && m.ignore_case {
            self.index
                .by_label
                .get(&m.pattern.as_str().to_lowercase())
                .cloned()
                .unwrap_or_default()
        } else {
            self.index
                .by_label
                .values()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
        };
        if !(m.exact && m.ignore_case) {
            out.retain(|k| self.label_matches(k, m));
        }
        out.sort_dedup();
        out
    }

    /// `getByText` candidates: text matches label *or* value (Playwright
    /// matches visible text; AccessKit exposes text through labels and
    /// values).
    fn text_candidates(&self, m: &TextMatch) -> Vec<NodeKey> {
        let mut out: Vec<NodeKey> = Vec::new();
        if m.exact && m.ignore_case {
            let needle = m.pattern.as_str().to_lowercase();
            out.extend(
                self.index
                    .by_label
                    .get(&needle)
                    .cloned()
                    .unwrap_or_default(),
            );
            out.extend(
                self.index
                    .by_value
                    .get(&needle)
                    .cloned()
                    .unwrap_or_default(),
            );
        } else {
            out.extend(self.index.all.iter().copied());
        }
        out.retain(|k| self.text_matches(k, m));
        out.sort_dedup();
        out
    }

    /// `getByValue` candidates (the label path over the value index).
    fn value_candidates(&self, m: &TextMatch) -> Vec<NodeKey> {
        let mut out = if m.exact && m.ignore_case {
            self.index
                .by_value
                .get(&m.pattern.as_str().to_lowercase())
                .cloned()
                .unwrap_or_default()
        } else {
            self.index
                .by_value
                .values()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
        };
        if !(m.exact && m.ignore_case) {
            out.retain(|k| self.value_matches(k, m));
        }
        out.sort_dedup();
        out
    }

    /// Chained `All` candidates: resolve the head, then narrow through
    /// every link's subtree (§8.2 "filter chains").
    fn all_candidates(&self, chain: &[Selector]) -> Result<Vec<NodeKey>, CoreError> {
        let mut current = self.resolve_all(&chain[0])?;
        for link in &chain[1..] {
            let mut next = Vec::new();
            for parent in &current {
                self.descendants(&parent.key, &mut |key, _| {
                    if self.matches_selector(key, link) && !next.contains(&key) {
                        next.push(key);
                    }
                });
            }
            current = next
                .into_iter()
                .map(|key| Resolved {
                    key,
                    generation: self.generation(&key),
                    bounds: self.node(&key).and_then(|n| n.bounding_box()),
                })
                .collect();
        }
        Ok(current.into_iter().map(|r| r.key).collect())
    }

    /// `Filter` candidates: `has_text` / `has_not` narrowing over the source
    /// resolution, then the document-order `nth` pick (§8.1 filters).
    fn filter_candidates(
        &self,
        source: &Selector,
        has_text: Option<&TextMatch>,
        has_not: Option<&Selector>,
        nth: Option<&Nth>,
    ) -> Result<Vec<NodeKey>, CoreError> {
        let mut candidates = self.resolve_all(source)?;
        if let Some(t) = has_text {
            candidates.retain(|r| self.subtree_has_text(&r.key, t));
        }
        if let Some(inner) = has_not {
            candidates.retain(|r| {
                // Exclude nodes whose subtree matches the inner selector.
                !self.subtree_matches_selector(&r.key, inner)
            });
        }
        if let Some(nth) = nth {
            let mut ordered = candidates;
            ordered.sort_by_key(|r| self.document_order(&r.key));
            candidates = match nth {
                Nth::First => ordered.into_iter().take(1).collect(),
                Nth::Last => ordered
                    .into_iter()
                    .next_back()
                    .map(|r| vec![r])
                    .unwrap_or_default(),
                Nth::Index(i) => ordered
                    .into_iter()
                    .nth(*i)
                    .map(|r| vec![r])
                    .unwrap_or_default(),
            };
        }
        Ok(candidates.into_iter().map(|r| r.key).collect())
    }

    /// Does `key` itself satisfy `selector`? (single-node predicate,
    /// shared by chains and subtree filters).
    fn matches_selector(&self, key: NodeKey, selector: &Selector) -> bool {
        let Some(entry) = self.entry_of(key) else {
            return false;
        };
        match selector {
            Selector::Role { role, name } => {
                if entry.role != *role {
                    return false;
                }
                name.as_ref().is_none_or(|m| match &entry.label {
                    Some(l) => m.matches(l),
                    None => false,
                })
            }
            Selector::Label(m) => entry.label.as_deref().is_some_and(|l| m.matches(l)),
            Selector::Text(m) => {
                entry.label.as_deref().is_some_and(|l| m.matches(l))
                    || entry.value.as_deref().is_some_and(|v| m.matches(v))
            }
            Selector::Value(m) => entry.value.as_deref().is_some_and(|v| m.matches(v)),
            Selector::TestId(id) => self.index.test_ids.get(id) == Some(&key),
            Selector::All(_) | Selector::Filter { .. } => {
                // Full sub-resolution within the subtree of this node.
                self.subtree_matches_selector(&key, selector)
            }
        }
    }

    /// Whether any node in `key`'s subtree (including itself) matches the
    /// selector — used by `has_text`/`has_not`/chains.
    fn subtree_matches_selector(&self, key: &NodeKey, selector: &Selector) -> bool {
        if self.matches_selector(*key, selector) {
            return true;
        }
        let mut found = false;
        self.descendants(key, &mut |k, _| {
            if !found && self.matches_selector(k, selector) {
                found = true;
            }
        });
        found
    }

    fn subtree_has_text(&self, key: &NodeKey, m: &TextMatch) -> bool {
        if let Some(entry) = self.entry_of(*key) {
            if entry.label.as_deref().is_some_and(|l| m.matches(l))
                || entry.value.as_deref().is_some_and(|v| m.matches(v))
            {
                return true;
            }
        }
        let mut found = false;
        self.descendants(key, &mut |k, _| {
            if found {
                return;
            }
            if let Some(entry) = self.entry_of(k) {
                if entry.label.as_deref().is_some_and(|l| m.matches(l))
                    || entry.value.as_deref().is_some_and(|v| m.matches(v))
                {
                    found = true;
                }
            }
        });
        found
    }

    fn entry_of(&self, key: NodeKey) -> Option<IndexEntry> {
        let node = self.node(&key)?;
        Some(IndexEntry::from_node(&node))
    }

    fn role_name_matches(&self, key: &NodeKey, m: &TextMatch) -> bool {
        self.entry_of(*key)
            .and_then(|e| e.label)
            .is_some_and(|l| m.matches(&l))
    }

    fn label_matches(&self, key: &NodeKey, m: &TextMatch) -> bool {
        self.entry_of(*key)
            .and_then(|e| e.label)
            .is_some_and(|l| m.matches(&l))
    }

    fn value_matches(&self, key: &NodeKey, m: &TextMatch) -> bool {
        self.entry_of(*key)
            .and_then(|e| e.value)
            .is_some_and(|v| m.matches(&v))
    }

    fn text_matches(&self, key: &NodeKey, m: &TextMatch) -> bool {
        self.entry_of(*key).is_some_and(|e| {
            e.label.as_deref().is_some_and(|l| m.matches(l))
                || e.value.as_deref().is_some_and(|v| m.matches(v))
        })
    }

    /// Document (depth-first) order of a node, computed on demand.
    /// O(tree) — only used for `nth` narrowing and error reports.
    fn document_order(&self, key: &NodeKey) -> usize {
        let Some(tree) = &self.tree else {
            return usize::MAX;
        };
        let mut order = 0usize;
        let mut found = usize::MAX;
        let root = tree.state().root();
        walk(root, &mut |k, _| {
            if found == usize::MAX {
                if k == *key {
                    found = order;
                }
                order += 1;
            }
        });
        found
    }

    // ------------------------------------------------------------------
    // Reporting

    pub fn describe(&self, key: &NodeKey) -> NodeDesc {
        let node = self.node(key);
        NodeDesc {
            key: *key,
            role: node
                .as_ref()
                .map_or("?".into(), |n| format!("{:?}", n.role())),
            name: node.as_ref().and_then(NodeRef::label),
            bounds: node
                .as_ref()
                .and_then(NodeRef::bounding_box)
                .map(|r| format!("{r:?}")),
        }
    }

    /// Compact state summary embedded in errors (§8.3).
    pub fn summary(&self) -> TreeSummary {
        let mut sample = Vec::new();
        for key in self.index.all.iter().take(5) {
            sample.push(self.describe(key));
        }
        TreeSummary {
            node_count: self.index.all.len(),
            root_role: self
                .index
                .all
                .first()
                .map(|k| self.describe(k))
                .map(|d| d.role),
            focus: self.focus().map(|k| self.describe(&k)),
            sample,
        }
    }
}

/// Document-order dedup helper.
trait SortDedup {
    fn sort_dedup(&mut self);
}

impl SortDedup for Vec<NodeKey> {
    fn sort_dedup(&mut self) {
        self.sort_unstable();
        self.dedup();
    }
}

/// Depth-first walk of a live node (including itself).
fn node_key(n: &NodeRef<'_>) -> NodeKey {
    let (node, tree) = n.locate();
    NodeKey::AccessKit { tree, node }
}

fn walk(node: NodeRef<'_>, visit: &mut impl FnMut(NodeKey, &NodeRef<'_>)) {
    visit(node_key(&node), &node);
    for child in node.children() {
        walk(child, visit);
    }
}

fn walk_descendants(node: NodeRef<'_>, visit: &mut impl FnMut(NodeKey, &NodeRef<'_>)) {
    for child in node.children() {
        visit(node_key(&child), &child);
        walk_descendants(child, visit);
    }
}

/// Collects consumer change callbacks into plain data so they can be applied
/// to the indexes after the consumer tree has committed (borrow-split).
#[derive(Default)]
struct DiffCollector {
    added: Vec<(NodeKey, IndexEntry)>,
    updated: Vec<(NodeKey, IndexEntry, IndexEntry)>,
    removed: Vec<(NodeKey, IndexEntry)>,
    focus_changed: bool,
}

impl TreeChangeHandler for DiffCollector {
    fn node_added(&mut self, node: &NodeRef<'_>) {
        self.added
            .push((node_key(node), IndexEntry::from_node(node)));
    }

    fn node_updated(&mut self, old_node: &NodeRef<'_>, new_node: &NodeRef<'_>) {
        self.updated.push((
            node_key(new_node),
            IndexEntry::from_node(old_node),
            IndexEntry::from_node(new_node),
        ));
    }

    fn focus_moved(&mut self, _old_node: Option<&NodeRef<'_>>, _new_node: Option<&NodeRef<'_>>) {
        self.focus_changed = true;
    }

    fn node_removed(&mut self, node: &NodeRef<'_>) {
        self.removed
            .push((node_key(node), IndexEntry::from_node(node)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Fixture;

    fn svc_with(fixture: &Fixture) -> TreeService {
        let mut svc = TreeService::new();
        svc.apply_update(fixture.initial_update());
        svc
    }

    #[test]
    fn init_indexes_every_node() {
        let fixture = Fixture::buttons();
        let svc = svc_with(&fixture);
        assert_eq!(svc.node_count(), fixture.expected_nodes);
    }

    #[test]
    fn incremental_diff_updates_index() {
        let fixture = Fixture::buttons();
        let mut svc = svc_with(&fixture);

        let summary = svc.apply_update(fixture.add_button_update("Export"));
        assert_eq!(summary.added.len(), 1);
        assert_eq!(summary.removed.len(), 0);
        assert_eq!(svc.node_count(), fixture.expected_nodes + 1);

        let resolved = svc
            .resolve(&Selector::role_named(Role::Button, "Export"))
            .expect("export button resolves");
        assert_eq!(svc.generation(&resolved.key), 0);
    }

    #[test]
    fn removal_bumps_generation_and_cleans_index() {
        let fixture = Fixture::buttons();
        let mut svc = svc_with(&fixture);
        let before = svc
            .resolve(&Selector::role_named(Role::Button, "Save"))
            .expect("save resolves");

        let summary = svc.apply_update(fixture.remove_save_update());
        assert_eq!(summary.removed.len(), 1);
        assert!(
            svc.resolve(&Selector::role_named(Role::Button, "Save"))
                .is_err()
        );
        assert_eq!(svc.generation(&before.key), 1);
    }

    #[test]
    fn frames_are_sequenced() {
        let mut svc = TreeService::new();
        let f1 = svc.frame_presented(0xA);
        let f2 = svc.frame_presented(0xA);
        assert_eq!(f2.seq, f1.seq + 1);
        assert_eq!(svc.last_frame().unwrap().hash, 0xA);
    }

    #[test]
    fn update_without_tree_info_is_deferred() {
        let mut svc = TreeService::new();
        let fixture = Fixture::buttons();
        let no_info = fixture.initial_update();
        let no_info = accesskit::TreeUpdate {
            tree: None,
            ..no_info
        };
        let summary = svc.apply_update(no_info);
        assert!(summary.is_empty());
        assert_eq!(svc.node_count(), 0);
        // Now the real one initializes.
        svc.apply_update(fixture.initial_update());
        assert_eq!(svc.node_count(), fixture.expected_nodes);
    }
}
