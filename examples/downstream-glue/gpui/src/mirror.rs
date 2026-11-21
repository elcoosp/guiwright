//! The gpui API **mirror** — the binding surface, scripted.
//!
//! This is the dependency-free stand-in for gpui 0.2.2 + `test-support`
//! (the Zed platform stack the real `glue/gpui` rides): every name and
//! signature here is the real surface the binding exercises, reduced to
//! the behavior the binding touches:
//!
//! * **Input** — [`Window::dispatch_event`] with [`PlatformInput`]
//!   payloads (the canonical seam, §4.5 "synthetic events are
//!   bit-identical to OS input"): pointer events hit-test against the
//!   a11y bounds the app restated, clicks fire the node's behavior.
//! * **A11y** — [`Window::a11y_tree_update`]: the app restates the full
//!   `TreeUpdate` per frame while the window is active (Level-1
//!   unconditional AccessKit, §4.5); the engine's
//!   [`guiwright_adapter_gpui::A11yHybrid`] carries the activation model.
//! * **Entities** — [`Entity<T>`]: the `Rc<RefCell>` handle the fact
//!   readers close over (the real glue's `SemBinding` readers take
//!   `&gpui::App` — same shape, mirrored confinement: everything here is
//!   main-thread, like gpui's `Rc` entities).
//! * **Test support** — [`VisualTestContext::from_window`] dispatches
//!   like the real test-support context; the seeded virtual dispatcher
//!   ([`App::dispatch_round`]) is what the engine's clock plan drives.
//!
//! Deliberate divergences (documented real-harness seams, not oversights):
//! no `HeadlessRenderer` factory (the pixels blocker — the mirror renders
//! a scripted raster instead, and `render_frame` stays test-support-gated
//! in caps), no scheduler session bookkeeping (the fill blocker that
//! panics on the REAL `KeyDown` path — why the adapter's `SetValue` stays
//! `Unsupported` on both sides), and one window per app (the spike's
//! single-window contract).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use accesskit::{Node, NodeId, TreeId, TreeInfo, TreeUpdate};

/// A logical-px point (gpui `point`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// gpui's mouse button vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// gpui `MouseMoveEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct MouseMoveEvent {
    pub position: Point,
}

/// gpui `MouseDownEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct MouseDownEvent {
    pub position: Point,
    pub button: MouseButton,
    pub click_count: u32,
}

/// gpui `MouseUpEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct MouseUpEvent {
    pub position: Point,
    pub button: MouseButton,
    pub click_count: u32,
}

/// gpui `Keystroke` (the `VisualTestContext::simulate_keystrokes`
/// vocabulary unit).
#[derive(Clone, Debug, PartialEq)]
pub struct Keystroke {
    pub modifiers: Vec<String>,
    pub key: String,
}

/// gpui `KeyDownEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyDownEvent {
    pub keystroke: Keystroke,
}

/// gpui `PlatformInput` (the dispatch payload subset the binding uses).
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformInput {
    MouseMove(MouseMoveEvent),
    MouseDown(MouseDownEvent),
    MouseUp(MouseUpEvent),
    KeyDown(KeyDownEvent),
}

/// gpui `DispatchEventResult` (the propagation flag the dispatchers read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchEventResult {
    pub propagate: bool,
}

/// The app context (gpui `App`): the seeded virtual dispatcher's clock
/// state lives here — what the engine's clock plan drives (§4.5 "precisely
/// what `TestPlatform` does", §6.2 "⚠️ test-support dispatcher").
#[derive(Debug, Default)]
pub struct App {
    virtual_ms: u64,
    paused: bool,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// One dispatcher round: run every timer/task scheduled before the
    /// (virtual) deadline, then flush a frame (§14; the mirror schedules
    /// nothing of its own, so the round is the advance itself).
    pub fn dispatch_round(&mut self, advance_ms: u64) {
        self.virtual_ms += advance_ms;
    }

    /// The virtual clock the §14 probe reports.
    pub fn virtual_ms(&self) -> u64 {
        self.virtual_ms
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn paused(&self) -> bool {
        self.paused
    }
}

/// gpui `Entity<T>` — the `Rc<RefCell>` handle (main-thread confinement,
/// §4.5): the app state the views render from AND the runner context the
/// §14 seams write through.
#[derive(Debug)]
pub struct Entity<T: 'static>(Rc<RefCell<T>>);

impl<T: 'static> Clone for Entity<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: 'static> Entity<T> {
    pub fn new(value: T) -> Self {
        Self(Rc::new(RefCell::new(value)))
    }

    /// The read the fact readers close over (the real glue's readers take
    /// `&gpui::App`; the mirror reads the entity directly — same
    /// shared-state seam, mirrored shape).
    pub fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let guard = self.0.borrow();
        read(&guard)
    }

    pub fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.0.borrow_mut();
        update(&mut guard)
    }

    /// The mutable runner context (§14): the entity handed to `run_in`
    /// tasks as the `&mut dyn Any` the hook writes.
    pub fn any_mut(&self) -> std::cell::RefMut<'_, T> {
        self.0.borrow_mut()
    }
}

/// A node behavior the mirror dispatches to (the click behavior keyed by
/// the a11y `NodeId` the app restated — the a11y `ActionRequest`-shaped
/// routing, §8.4).
pub type NodeAction = Rc<dyn Fn()>;

/// The key handler behind the focused element (`on_key_down`, §4.5).
pub type KeyHandler = Rc<dyn Fn(&KeyDownEvent)>;

/// The platform window (gpui `Window` / `PlatformWindow`): `dispatch_event`
/// is the canonical input seam, `a11y_tree_update` the Level-1 tree path.
pub struct Window {
    /// The per-frame a11y restatement builder (the app's Level-1 payload).
    a11y_builder: Rc<dyn Fn() -> TreeUpdate>,
    /// The behaviors keyed by node id (clicks route through a11y bounds).
    actions: Rc<RefCell<HashMap<u64, NodeAction>>>,
    /// The bounds the hit-tests and `act` read (restated per frame).
    bounds: Rc<RefCell<HashMap<u64, [f64; 4]>>>,
    /// The key handler (the focused element's `on_key_down`).
    key_handler: Option<KeyHandler>,
}

impl Window {
    pub fn new(
        a11y_builder: Rc<dyn Fn() -> TreeUpdate>,
        actions: Rc<RefCell<HashMap<u64, NodeAction>>>,
        bounds: Rc<RefCell<HashMap<u64, [f64; 4]>>>,
    ) -> Self {
        Self {
            a11y_builder,
            actions,
            bounds,
            key_handler: None,
        }
    }

    /// The canonical input seam (§4.5): pointer events hit-test the
    /// restated a11y bounds; a press/release pair on the same node fires
    /// its behavior (the click semantics); keys route to the focused
    /// element's handler. Mirrors `Window::dispatch_event`.
    pub fn dispatch_event(&mut self, input: PlatformInput) -> DispatchEventResult {
        match input {
            PlatformInput::MouseUp(event) => {
                let node = self.hit_test(event.position, event.button);
                if let Some(node) = node
                    && let Some(action) = self.actions.borrow().get(&node).cloned()
                {
                    action();
                }
                DispatchEventResult { propagate: false }
            }
            PlatformInput::KeyDown(event) => {
                if let Some(handler) = &self.key_handler {
                    handler(&event);
                }
                DispatchEventResult { propagate: false }
            }
            PlatformInput::MouseMove(_) | PlatformInput::MouseDown(_) => {
                DispatchEventResult { propagate: true }
            }
        }
    }

    /// `PlatformWindow::a11y_tree_update` while active (Level-1, §4.5):
    /// the app restates the full update; the mirror also refreshes the
    /// bounds the hit-tests read. The driver attach force-activates a11y
    /// (the engine's `A11yHybrid` immediate-activation row), so this runs
    /// from the first pump.
    pub fn a11y_tree_update(&mut self) -> TreeUpdate {
        let update = (self.a11y_builder)();
        let mut fresh = HashMap::new();
        for (id, node) in &update.nodes {
            if let Some(rect) = node.bounds() {
                fresh.insert(id.0, [rect.x0, rect.y0, rect.x1, rect.y1]);
            }
        }
        *self.bounds.borrow_mut() = fresh;
        update
    }

    /// The topmost node containing the point (the hit-test `act` and the
    /// synthesized clicks route through).
    fn hit_test(&self, position: Point, _button: MouseButton) -> Option<u64> {
        // Topmost match wins: the SMALLEST node containing the point
        // (the window root contains every point, so "last in hash
        // iteration order" would be nondeterministic — gpui's real
        // hit-testing resolves the deepest element the same way).
        let bounds = self.bounds.borrow();
        let mut best: Option<(u64, f64)> = None;
        for (id, rect) in bounds.iter() {
            if position.x >= rect[0]
                && position.x < rect[2]
                && position.y >= rect[1]
                && position.y < rect[3]
            {
                let area = (rect[2] - rect[0]) * (rect[3] - rect[1]);
                if best.is_none_or(|(_, best_area)| area < best_area) {
                    best = Some((*id, area));
                }
            }
        }
        best.map(|(id, _)| id)
    }

    /// The window's root tree id (the single-window persona, §16 v0.4).
    pub fn root_tree_id(&self) -> TreeId {
        TreeId::ROOT
    }

    /// The tree identity the updates carry (AccessKit convention).
    pub fn tree_info(&self, root: NodeId) -> TreeInfo {
        TreeInfo {
            root,
            toolkit_name: Some("guiwright-gpui-mirror".to_string()),
            toolkit_version: None,
        }
    }
}

/// gpui `VisualTestContext` (test-support): the dispatch handle tests use
/// — `from_window` + the same `PlatformInput` choke point (§4.5 "the
/// `VisualTestContext` keystroke vocabulary").
pub struct VisualTestContext {
    window: Rc<RefCell<Window>>,
}

impl VisualTestContext {
    pub fn from_window(window: &Rc<RefCell<Window>>, _cx: &App) -> Self {
        Self {
            window: Rc::clone(window),
        }
    }

    /// Dispatch through the window's real pipeline (no bypass).
    pub fn dispatch(&mut self, input: PlatformInput) -> DispatchEventResult {
        self.window.borrow_mut().dispatch_event(input)
    }
}

/// The counter app state (the `Entity` the views render from and the
/// seams reset).
#[derive(Debug, Clone)]
pub struct CounterState {
    pub count: u64,
    pub label: String,
}

/// The counter window factory: restates the a11y tree per frame from the
/// entity (Level-1 AccessKit, stable node ids, real bounds), wires the
/// increment behavior to the button node, and keeps the a11y payload the
/// adapter's activation model applies.
pub fn counter_window(
    entity: &Entity<CounterState>,
) -> (Rc<RefCell<Window>>, Entity<CounterState>) {
    // Stable a11y ids (the app mints them, Level-1: the driver consumes
    // them as-is — `NodeKey::AccessKit`, §6.1).
    let button_id = NodeId(1);
    let label_id = NodeId(2);
    let root_id = NodeId(0);

    let actions: Rc<RefCell<HashMap<u64, NodeAction>>> = Rc::new(RefCell::new(HashMap::new()));
    let bounds: Rc<RefCell<HashMap<u64, [f64; 4]>>> = Rc::new(RefCell::new(HashMap::new()));

    let increment_entity = entity.clone();
    actions.borrow_mut().insert(
        button_id.0,
        Rc::new(move || {
            increment_entity.update(|state| {
                state.count += 1;
                state.label = format!("Count: {}", state.count);
            });
        }),
    );

    let a11y_entity = entity.clone();
    let a11y_builder: Rc<dyn Fn() -> TreeUpdate> = Rc::new(move || {
        let count = a11y_entity.read(|state| state.count);
        let mut button = Node::new(accesskit::Role::Button);
        button.set_label("Increment");
        button.set_bounds(accesskit::Rect {
            x0: 16.0,
            y0: 16.0,
            x1: 136.0,
            y1: 48.0,
        });
        let mut text = Node::new(accesskit::Role::Label);
        text.set_label(format!("Count: {count}"));
        text.set_bounds(accesskit::Rect {
            x0: 16.0,
            y0: 64.0,
            x1: 136.0,
            y1: 96.0,
        });
        let mut root = Node::new(accesskit::Role::Window);
        root.set_label("counter");
        root.set_bounds(accesskit::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 480.0,
            y1: 240.0,
        });
        root.set_children([button_id, label_id]);
        TreeUpdate {
            nodes: vec![(root_id, root), (button_id, button), (label_id, text)],
            tree: Some(TreeInfo {
                root: root_id,
                toolkit_name: Some("guiwright-gpui-mirror".to_string()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: button_id,
        }
    });

    let window = Rc::new(RefCell::new(Window::new(
        a11y_builder,
        Rc::clone(&actions),
        Rc::clone(&bounds),
    )));
    (window, entity.clone())
}
