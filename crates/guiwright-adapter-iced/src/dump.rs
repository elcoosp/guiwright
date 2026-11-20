//! The §4.6 tree read-back: a [`widget::Operation`] that walks the laid-out
//! widget tree and ships the walk out over an mpsc channel.
//!
//! Iced has no AccessKit tree (§4.6 "nothing to piggyback — Guiwright builds
//! its logical tree from selector ops"). The read-back runs as a real
//! `Task::widget` operation **inside the Emulator's runtime context** —
//! operations execute against the laid-out UI on the runner thread, and "an
//! mpsc channel is the safe boundary" (§4.6 live-mode pattern). The app's
//! only obligation is one update arm that forwards the request (the
//! [`HarnessApp`] hook, in `crate::emulator`).
//!
//! Kind semantics per hook (iced 0.14 exposes no widget-type names — the
//! operation hooks are capability-shaped):
//!
//! | hook | kind | identity |
//! | --- | --- | --- |
//! | `text` | `"text"` | none (iced passes `None`) |
//! | `text_input` | `"text_input"` | the widget's `Id` |
//! | `focusable` | `"focusable"` | the widget's `Id` |
//! | `scrollable` | `"scrollable"` | the widget's `Id` |
//! | `container` | `"container"` | the widget's `Id`, when it has one |
//!
//! One iced widget may fire several hooks — `text_input` fires
//! `text_input` + `focusable` with the same `Id`; the walk refines such a
//! node instead of nesting it. Buttons and plain columns are
//! indistinguishable hooks (`container(None)` — iced 0.14 passes a button's
//! `Id` to no operation hook), so apps contribute the semantics of their
//! interactive widgets: wrap them in an `Id`-carrying container and answer
//! [`HarnessApp::kind_of`] — the §4.6 "role registry contribution", the
//! same contract masonry meets with `WidgetTag`.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::mpsc::Sender;

use iced_test::core::widget::Id;
use iced_test::core::widget::operation::{Focusable, Operation, Outcome, Scrollable, TextInput};
use iced_test::core::{Point, Rectangle, Size};
use iced_test::program::Program;
use iced_test::runtime::Task;

use crate::logical::WidgetReport;

/// Handle of the synthetic window root the harness wraps the walk in (the
/// single-window vocabulary, §16 v0.4 single-window row).
pub const WINDOW_HANDLE: u64 = 0;

/// The read-back channel: the operation ships [`DumpResult`]s through it,
/// once per dump request.
#[derive(Clone, Debug)]
pub struct TreeDumpTx(Sender<DumpResult>);

impl TreeDumpTx {
    pub fn new(sender: Sender<DumpResult>) -> Self {
        Self(sender)
    }

    fn send(&self, result: DumpResult) {
        // The receiver lives as long as the harness; a failed send means
        // the harness is gone and the walk result is moot.
        let _ = self.0.send(result);
    }
}

/// One completed walk: the laid-out widget tree, root-first.
#[derive(Debug)]
pub struct DumpResult {
    /// The root widget's node (the harness wraps it in a window root).
    pub root: WidgetReport,
}

/// Hook specificity — follow-up hooks on the same widget refine the kind
/// (`text_input` also fires `focusable` with the same `Id`).
fn specificity(kind: &str) -> u8 {
    match kind {
        "text_input" => 4,
        "text" => 3,
        "scrollable" => 2,
        "focusable" => 1,
        _ => 0, // "container"
    }
}

/// Hook bounds in the report's window-relative px vocabulary
/// ([`WidgetReport`] bounds are x0/y0/x1/y1).
fn rect_bounds(bounds: Rectangle) -> [f64; 4] {
    [
        f64::from(bounds.x),
        f64::from(bounds.y),
        f64::from(bounds.x + bounds.width),
        f64::from(bounds.y + bounds.height),
    ]
}

/// Does `outer` fully contain `inner`? (layout rectangles, inclusive edges)
fn contains(outer: Rectangle, inner: Rectangle) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

/// An open node: the report so far plus the widget `Id` that opened it
/// (follow-up hooks with the same `Id` refine the node).
struct Frame {
    node: WidgetReport,
    id: Option<Id>,
}

/// The walk operation. Generic over the app so [`HarnessApp::kind_of`]
/// contributes semantics for `Id`-carrying widgets; apps hand it to
/// `Task::widget` from their hook arm (the [`HarnessApp::run_hook`]
/// default does exactly that).
pub struct TreeDumpOp<P: HarnessApp> {
    tx: TreeDumpTx,
    stack: Vec<Frame>,
    roots: Vec<WidgetReport>,
    ids: HashMap<Id, u64>,
    next_handle: u64,
    // `fn() -> P` keeps the app type parameter without inheriting its
    // (thread-local) `!Send`: `Operation` requires `Send`, and the op runs
    // inside the Emulator's runner context (§5.1), never crossing threads.
    app: PhantomData<fn() -> P>,
}

impl<P: HarnessApp> std::fmt::Debug for TreeDumpOp<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeDumpOp")
            .field("tx", &self.tx)
            .field("open_frames", &self.stack.len())
            .finish_non_exhaustive()
    }
}

impl<P: HarnessApp> TreeDumpOp<P> {
    pub fn new(tx: TreeDumpTx) -> Self {
        Self {
            tx,
            stack: Vec::new(),
            roots: Vec::new(),
            ids: HashMap::new(),
            next_handle: WINDOW_HANDLE + 1,
            app: PhantomData,
        }
    }

    /// Attach `finished` to its parent (or the roots when the stack is
    /// empty).
    fn attach(&mut self, finished: WidgetReport) {
        match self.stack.last_mut() {
            Some(Frame { node, .. }) => node.children.push(finished),
            None => self.roots.push(finished),
        }
    }

    /// Close everything above `depth` (inclusive of the frame at `depth`):
    /// containers close through their own `traverse`, leaves through their
    /// parent's.
    fn unwind_to(&mut self, depth: usize) {
        while self.stack.len() > depth {
            let frame = self.stack.pop().expect("frame above depth");
            self.attach(frame.node);
        }
    }

    /// Open (or refine) the node for the widget this hook reports.
    fn open(&mut self, kind: &'static str, id: Option<&Id>, bounds: Rectangle) {
        // Leaf widgets never call `traverse`; they close when the next
        // sibling opens. A finished sibling no longer contains the new
        // widget's bounds.
        while let Some(frame) = self.stack.last() {
            let node_bounds = match frame.node.bounds {
                Some([x0, y0, x1, y1]) => Rectangle::new(
                    Point::new(x0 as f32, y0 as f32),
                    Size::new((x1 - x0) as f32, (y1 - y0) as f32),
                ),
                None => break, // a bounds-less node never closes by geometry
            };
            if contains(node_bounds, bounds) {
                break;
            }
            let depth = self.stack.len();
            self.unwind_to(depth - 1);
        }

        // Follow-up hook on the widget that just opened: refine in place.
        if let Some(id) = id {
            if let Some(frame) = self.stack.last_mut() {
                if frame.id.as_ref() == Some(id) {
                    if specificity(kind) > specificity(frame.node.kind.as_str()) {
                        frame.node.kind = kind.to_string();
                    }
                    return;
                }
            }
        }

        // Handles: `Id`-carrying widgets intern stably (walk order within
        // one dump, id equality across dumps); id-less widgets ride the
        // walk counter — stable while the structure is unchanged, and the
        // engine's generation counters guard recycling (§8.2). Every
        // widget reserves a fresh slot so the two sources never collide.
        let reserved = self.next_handle;
        self.next_handle += 1;
        let handle = match id {
            Some(id) => *self.ids.entry(id.clone()).or_insert(reserved),
            None => reserved,
        };
        let kind = match id {
            // The app's semantic contribution wins over the hook's
            // capability class (§4.6 "adds role semantics via a
            // contributed widget-kind registry").
            Some(id) => P::kind_of(id).unwrap_or(kind),
            None => kind,
        };
        self.stack.push(Frame {
            node: WidgetReport::new(kind, handle).with_bounds(
                rect_bounds(bounds)[0],
                rect_bounds(bounds)[1],
                rect_bounds(bounds)[2],
                rect_bounds(bounds)[3],
            ),
            id: id.cloned(),
        });
    }
}

impl<P: HarnessApp, T> Operation<T> for TreeDumpOp<P> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        // The widget's own frame sits at `depth - 1`: it opened before
        // asking for traversal. Everything opened since belongs to its
        // subtree.
        let depth = self.stack.len();
        operate(self);
        self.unwind_to(depth.saturating_sub(1));
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.open("container", id, bounds);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        _translation: iced_test::core::Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.open("scrollable", id, bounds);
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        self.open("focusable", id, bounds);
        if let Some(frame) = self.stack.last_mut() {
            frame.node.focused = state.is_focused();
        }
    }

    fn text_input(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn TextInput) {
        self.open("text_input", id, bounds);
        if let Some(frame) = self.stack.last_mut() {
            // The visible text: the value, or the placeholder when empty
            // (the §9 quirk row — a placeholder-filled report still clears
            // correctly because backspaces on an empty editor are no-ops).
            frame.node.value = Some(state.text().to_owned());
        }
    }

    fn text(&mut self, id: Option<&Id>, bounds: Rectangle, content: &str) {
        self.open("text", id, bounds);
        if let Some(frame) = self.stack.last_mut() {
            frame.node.text = Some(content.to_owned());
        }
    }

    fn finish(&self) -> Outcome<T> {
        // The result ships over the channel, not the task output — the
        // Emulator drops widget-operation outputs (§4.6 "the channel is
        // the safe boundary"). Every stock widget traverses, so the walk
        // has fully unwound into `roots` by the time `finish` runs.
        if let Some(root) = self.roots.first().cloned() {
            self.tx.send(DumpResult { root });
        }
        Outcome::None
    }
}

/// AccessKit naming (name-from-content): a node without its own text
/// adopts the content of a single text child — a button's label lives
/// inside the button (iced walks buttons as containers around a text), and
/// the §8 role+name locators key on the accessible name.
fn content_label(report: &WidgetReport) -> Option<String> {
    if let Some(text) = &report.text {
        return Some(text.clone());
    }
    if report.children.len() == 1 {
        return content_label(&report.children[0]);
    }
    None
}

pub fn adopt_names(report: &mut WidgetReport) {
    for child in &mut report.children {
        adopt_names(child);
    }
    // Plain containers keep the raw capability shape (a Group's name is
    // not its content); semantic kinds — the app's `kind_of` contribution
    // and the hook-derived widget kinds — take the name from content.
    if report.text.is_none() && report.kind != "container" {
        if let Some(text) = content_label(report) {
            report.text = Some(text);
        }
    }
}

/// The app-side read-back contract (§4.6): the ONE hook an iced Program
/// adds so the harness can read the laid-out widget tree back.
pub trait HarnessApp: Program + 'static {
    /// Wrap the read-back request into the app's message vocabulary.
    fn hook(tx: TreeDumpTx) -> Self::Message;

    /// The app's update arm for `Self::hook(tx)`: run the tree-dump
    /// operation against the laid-out UI. The default returns exactly the
    /// `Task::widget(TreeDumpOp)` the §4.6 read-back specifies — apps only
    /// route the hook message here.
    fn run_hook(_state: &mut Self::State, tx: TreeDumpTx) -> Task<Self::Message> {
        iced_test::runtime::task::widget(TreeDumpOp::<Self>::new(tx))
    }

    /// The app's semantic contribution for an `Id`-carrying widget
    /// (§4.6 "role registry contribution"): the widget-kind string the
    /// logical tree presents (registry-mapped to an AccessKit role, see
    /// `crate::registry::defaults`). Buttons and other interactive widgets
    /// contribute here — iced 0.14's operation hooks carry no widget-type
    /// names for them. Default: the hook's capability class.
    fn kind_of(_id: &Id) -> Option<&'static str> {
        None
    }
}
