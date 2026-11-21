//! The floem API **mirror** — the binding surface, scripted.
//!
//! This is the dependency-free stand-in for `lapce/floem` at the pinned
//! revision (`lapce/floem@1351ffb`): every name and signature here is the
//! real harness surface the `glue/floem` binding rides (its ride log is
//! the source of truth — see `guiwright-adapter-floem/src/contract.rs`),
//! reduced to the behavior the binding exercises:
//!
//! * **Boot** — [`TestRoot::new`] claims the thread's root view, the app
//!   factory builds the view tree against it, and
//!   [`HeadlessHarness::new_with_size`] wraps it (the real harness's
//!   `PaintState` "will never initialize" headless — the engine's
//!   tiny-skia [`guiwright_adapter_floem::paint`] module is what renders,
//!   exactly like the real glue).
//! * **Tree** — [`ViewId`] walks: [`ViewId::debug_name`] (the
//!   `"<test_id> - <TypeName>: <payload>"` decorator format the glue
//!   parses), [`ViewId::get_visual_rect`], [`ViewId::children`],
//!   [`ViewId::is_valid`].
//! * **Input** — [`HeadlessHarness::dispatch_event`] with the DOM-like
//!   [`Event`] payloads and real hit-testing: the event lands on the
//!   topmost view containing the point (the `element_id_at` behavior),
//!   clicks fire the view's behavior, key/IME edits write the focused
//!   editor buffer.
//! * **Focus** — clicking a `text_input` focuses it
//!   (`HeadlessHarness::is_focused`), mirroring the real
//!   focus-follows-click the walk reads.
//!
//! Deliberate divergences (each one a documented real-harness seam, not
//! an oversight): views are `Rc<RefCell<…>>`-backed so [`ViewId`] is
//! `Clone` rather than `Copy` (floem's slotmap key is Copy); there is no
//! capture/bubble routing (the mirror dispatches to the hit target only);
//! and no IME-request machinery (the REAL headless harness panics on the
//! `text_input` IME request — `contract.rs`'s fill blocker — which is why
//! the adapter's `SetValue` stays `Unsupported` on both sides).

use std::cell::RefCell;
use std::rc::Rc;

/// The thread's root-view claim (§4.7 boot: views created after the
/// claim attach to it). The mirror keeps the token shape; the real
/// `TestRoot` guards the global view context.
#[derive(Debug)]
pub struct TestRoot {
    _priv: (),
}

impl TestRoot {
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Default for TestRoot {
    fn default() -> Self {
        Self::new()
    }
}

/// `ViewId::get_visual_rect` output (origin + extents, logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// The DOM-like event payloads (floem `Event` subset, §4.7: pointer
/// down/up/move/leave, wheel, key down/up, IME commit).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    PointerDown(PointerDownEvent),
    PointerUp(PointerUpEvent),
    PointerMove(PointerMoveEvent),
    PointerLeave,
    Wheel(WheelEvent),
    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    ImeCommit(ImeCommitEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerDownEvent {
    pub x: f64,
    pub y: f64,
    pub button: PointerButton,
    pub click_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerUpEvent {
    pub x: f64,
    pub y: f64,
    pub button: PointerButton,
    pub click_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerMoveEvent {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WheelEvent {
    pub x: f64,
    pub y: f64,
    pub dx: f64,
    pub dy: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub key: String,
    pub modifiers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImeCommitEvent {
    pub text: String,
}

/// floem's pointer button vocabulary (the DOM-ish trio).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// One node of the retained mirror tree — the scripted equivalent of a
/// laid-out floem view: kind + optional test id (the `debug_name`
/// decorator), laid-out rect, the shared state the text/value facts read
/// (the real glue's `SemReader` seam: floem exposes no text getters), and
/// the click behavior.
#[derive(Clone)]
pub struct MirrorView {
    kind: &'static str,
    test_id: Option<String>,
    rect: Rect,
    text: Option<Rc<RefCell<String>>>,
    value: Option<Rc<RefCell<String>>>,
    on_click: Option<Rc<dyn Fn()>>,
    focusable: bool,
    children: Vec<MirrorView>,
}

impl std::fmt::Debug for MirrorView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorView")
            .field("kind", &self.kind)
            .field("test_id", &self.test_id)
            .field("rect", &self.rect)
            .field("focusable", &self.focusable)
            .field("children", &self.children.len())
            .finish_non_exhaustive()
    }
}

impl MirrorView {
    /// Scripted layout: the view's rect is declared, not computed — the
    /// real layout pass is the one floem surface the mirror does not
    /// model (the binding reads rects, never runs layout).
    pub fn at(mut self, x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        self.rect = Rect { x0, y0, x1, y1 };
        self
    }

    pub fn with_child(mut self, child: MirrorView) -> Self {
        self.children.push(child);
        self
    }

    /// The app factory's container (floem `v_stack`).
    pub fn v_stack() -> Self {
        Self {
            kind: "stack",
            test_id: None,
            rect: Rect::ORIGIN,
            text: None,
            value: None,
            on_click: None,
            focusable: false,
            children: Vec::new(),
        }
    }

    /// floem `button`: clickable, focusable on click.
    pub fn button(test_id: impl Into<String>, on_click: impl Fn() + 'static) -> Self {
        Self {
            kind: "button",
            test_id: Some(test_id.into()),
            rect: Rect::ORIGIN,
            text: None,
            value: None,
            on_click: Some(Rc::new(on_click)),
            focusable: true,
            children: Vec::new(),
        }
    }

    /// floem `label`: its text rides the app's shared state (the
    /// `SemReader` seam — the walk reads the CURRENT string every pass).
    pub fn label(test_id: impl Into<String>, text: Rc<RefCell<String>>) -> Self {
        Self {
            kind: "label",
            test_id: Some(test_id.into()),
            rect: Rect::ORIGIN,
            text: Some(text),
            value: None,
            on_click: None,
            focusable: false,
            children: Vec::new(),
        }
    }

    /// floem `text_input`: editable buffer (IME commits land here).
    pub fn text_input(test_id: impl Into<String>, value: Rc<RefCell<String>>) -> Self {
        Self {
            kind: "text_input",
            test_id: Some(test_id.into()),
            rect: Rect::ORIGIN,
            text: None,
            value: Some(value),
            on_click: None,
            focusable: true,
            children: Vec::new(),
        }
    }
}

impl Rect {
    const ORIGIN: Rect = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 0.0,
        y1: 0.0,
    };
}

#[derive(Debug, Default)]
struct Tree {
    nodes: Vec<NodeRecord>,
    next_id: u64,
    focused: Option<u64>,
}

/// The tree's stored node: the [`MirrorView`] fields the harness reads at
/// dispatch/walk time (the click behavior is held behind a separate
/// record so the tree stays `Debug`).
struct NodeRecord {
    id: u64,
    kind: &'static str,
    test_id: Option<String>,
    rect: Rect,
    text: Option<Rc<RefCell<String>>>,
    value: Option<Rc<RefCell<String>>>,
    on_click: Option<Rc<dyn Fn()>>,
    focusable: bool,
    children: Vec<u64>,
}

impl std::fmt::Debug for NodeRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeRecord")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("test_id", &self.test_id)
            .field("rect", &self.rect)
            .field("focusable", &self.focusable)
            .field("children", &self.children)
            .finish_non_exhaustive()
    }
}

impl Tree {
    /// Pre-order insert: the id doubles as the slot index, so the parent
    /// is pushed BEFORE its children recurse (ids stay index-aligned the
    /// way a slotmap's key↔slot pairing is).
    fn push(&mut self, view: MirrorView) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let MirrorView {
            kind,
            test_id,
            rect,
            text,
            value,
            on_click,
            focusable,
            children,
        } = view;
        self.nodes.push(NodeRecord {
            id,
            kind,
            test_id,
            rect,
            text,
            value,
            on_click,
            focusable,
            children: Vec::new(),
        });
        let child_ids = children.iter().map(|c| self.push(c.clone())).collect();
        if let Some(node) = self.node_mut(id) {
            node.children = child_ids;
        }
        id
    }

    fn node(&self, id: u64) -> Option<&NodeRecord> {
        self.nodes.get(id as usize)
    }

    fn node_mut(&mut self, id: u64) -> Option<&mut NodeRecord> {
        self.nodes.get_mut(id as usize)
    }
}

/// The harness-held view identity (floem `ViewId`): a slotmap-style index
/// plus the tree handle. `Clone`, not `Copy` — the mirror's tree is
/// `Rc<RefCell>`-backed (floem's key is Copy against its global context).
#[derive(Clone, Debug)]
pub struct ViewId {
    id: u64,
    tree: Rc<RefCell<Tree>>,
}

impl ViewId {
    /// The slotmap-style inner key (the `ViewReport::handle` the engine
    /// binds minted `NodeId`s to).
    pub(crate) fn inner(&self) -> u64 {
        self.id
    }

    /// The `debug_name` decorator string: `"<test_id> - <Kind>"` when a
    /// test id is set, `"<Kind>"` otherwise — the format the glue's
    /// `kind_key`/`test_id_of` parsers read.
    pub fn debug_name(&self) -> String {
        let tree = self.tree.borrow();
        match tree.node(self.id) {
            Some(node) => match &node.test_id {
                Some(test_id) => format!("{test_id} - {}", node.kind),
                None => node.kind.to_string(),
            },
            None => String::new(),
        }
    }

    /// The laid-out rect (public geometry, §4.7).
    pub fn get_visual_rect(&self) -> Rect {
        let tree = self.tree.borrow();
        tree.node(self.id).map_or(Rect::ORIGIN, |n| n.rect)
    }

    pub fn children(&self) -> Vec<ViewId> {
        let tree = self.tree.borrow();
        tree.node(self.id)
            .map(|n| {
                n.children
                    .iter()
                    .map(|child| ViewId {
                        id: *child,
                        tree: Rc::clone(&self.tree),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn is_valid(&self) -> bool {
        self.tree.borrow().node(self.id).is_some()
    }

    /// The shared state the facts read (the `SemReader` seam: the walk
    /// pulls the CURRENT text/value from the app's signals).
    pub(crate) fn text(&self) -> Option<String> {
        let tree = self.tree.borrow();
        tree.node(self.id)
            .and_then(|n| n.text.as_ref().map(|t| t.borrow().clone()))
    }

    pub(crate) fn value(&self) -> Option<String> {
        let tree = self.tree.borrow();
        tree.node(self.id)
            .and_then(|n| n.value.as_ref().map(|v| v.borrow().clone()))
    }

    fn on_click(&self) -> Option<Rc<dyn Fn()>> {
        self.tree
            .borrow()
            .node(self.id)
            .and_then(|n| n.on_click.as_ref().map(Rc::clone))
    }
}

/// The headless harness (floem `headless::HeadlessHarness`): tree + input
/// real, pixels none (the engine's tiny-skia `PaintState` renders — the
/// real glue's pixel story, unchanged).
#[derive(Debug)]
pub struct HeadlessHarness {
    tree: Rc<RefCell<Tree>>,
    root: u64,
    size: (f64, f64),
    pointer: (f64, f64),
}

impl HeadlessHarness {
    /// Wrap a freshly built view tree at the window size (§4.7 boot).
    pub fn new_with_size(root: TestRoot, view: MirrorView, w: f64, h: f64) -> Self {
        let _ = root;
        let mut tree = Tree::default();
        let root_id = tree.push(view);
        Self {
            tree: Rc::new(RefCell::new(tree)),
            root: root_id,
            size: (w, h),
            pointer: (w / 2.0, h / 2.0),
        }
    }

    pub fn root_id(&self) -> ViewId {
        ViewId {
            id: self.root,
            tree: Rc::clone(&self.tree),
        }
    }

    /// The focus read the walk folds in (focus-follows-click, mirroring
    /// the real harness's focus tracking).
    pub fn is_focused(&self, id: ViewId) -> bool {
        self.tree.borrow().focused == Some(id.id)
    }

    /// The window size the boot pinned (the adapter's coordinate space).
    pub fn size(&self) -> (f64, f64) {
        self.size
    }

    /// The DOM-like dispatch (§4.7 "headless = `dispatch_event`"): the
    /// event lands on the topmost view containing the pointer point —
    /// real hit-testing, no capture/bubble routing (the mirror's
    /// documented divergence).
    pub fn dispatch_event(&mut self, event: Event) {
        match event {
            Event::PointerMove(e) => {
                self.pointer = (e.x, e.y);
            }
            Event::PointerDown(e) => {
                self.pointer = (e.x, e.y);
                if let Some(hit) = self.hit_test(e.x, e.y) {
                    // Focus follows the press onto focusable views (the
                    // text_input IME request the REAL harness rejects —
                    // `contract.rs`'s fill blocker — happens at this
                    // point on floem; the mirror has no IME machinery to
                    // reject, and the adapter keeps `SetValue`
                    // `Unsupported` on both sides regardless).
                    if self.tree.borrow().node(hit).is_some_and(|n| n.focusable) {
                        self.tree.borrow_mut().focused = Some(hit);
                    }
                }
            }
            Event::PointerUp(e) => {
                self.pointer = (e.x, e.y);
                let hit = self.hit_test(e.x, e.y);
                let click = hit.and_then(|id| {
                    ViewId {
                        id,
                        tree: Rc::clone(&self.tree),
                    }
                    .on_click()
                });
                if let Some(click) = click {
                    click();
                }
            }
            Event::ImeCommit(e) => {
                // The editor buffer write: the focused text input's value.
                let focused = self.tree.borrow().focused;
                if let Some(id) = focused
                    && let Some(node) = self.tree.borrow_mut().node_mut(id)
                    && let Some(value) = &node.value
                {
                    *value.borrow_mut() = e.text;
                }
            }
            Event::PointerLeave | Event::Wheel(_) | Event::KeyDown(_) | Event::KeyUp(_) => {}
        }
    }

    /// The topmost (deepest, later-registered) view containing the point.
    fn hit_test(&self, x: f64, y: f64) -> Option<u64> {
        let tree = self.tree.borrow();
        let mut hit = None;
        for node in &tree.nodes {
            if node.rect.contains(x, y) {
                hit = Some(node.id);
            }
        }
        hit
    }

    // --- the harness convenience gestures the adapter's §9 synthesis uses
    // (the same helpers glue/floem calls; each routes through
    // `dispatch_event` — one pipeline, no bypass).

    pub fn pointer_move(&mut self, x: f64, y: f64) {
        self.dispatch_event(Event::PointerMove(PointerMoveEvent { x, y }));
    }

    pub fn pointer_down(&mut self, x: f64, y: f64) {
        self.dispatch_event(Event::PointerDown(PointerDownEvent {
            x,
            y,
            button: PointerButton::Left,
            click_count: 1,
        }));
    }

    pub fn pointer_up(&mut self, x: f64, y: f64) {
        self.dispatch_event(Event::PointerUp(PointerUpEvent {
            x,
            y,
            button: PointerButton::Left,
            click_count: 1,
        }));
    }

    pub fn pointer_leave(&mut self) {
        self.dispatch_event(Event::PointerLeave);
    }

    pub fn double_click(&mut self, x: f64, y: f64) {
        self.pointer_move(x, y);
        self.pointer_down(x, y);
        self.pointer_up(x, y);
        self.pointer_down(x, y);
        self.pointer_up(x, y);
    }

    pub fn scroll(&mut self, x: f64, y: f64, dx: f64, dy: f64) {
        self.dispatch_event(Event::Wheel(WheelEvent { x, y, dx, dy }));
    }

    /// The layout/rebuild pass (floem `rebuild`): the mirror's rects are
    /// scripted, so the pass is a marker the pump ride keeps honest.
    pub fn rebuild(&mut self) {}
}
