//! The [`FrameworkAdapter`] trait (architecture §6.1).
//!
//! Implemented once per GUI framework. **All methods execute on the UI
//! thread**, marshalled by the host at frame boundaries (§5.1): the adapter
//! never touches widgets off-thread, and never observes torn trees.

use serde::{Deserialize, Serialize};

/// Adapter self-description (reported in GWP `Welcome` and traces, §6.1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdapterMeta {
    pub name: &'static str,
    pub framework: &'static str,
    pub framework_version: String,
    pub caps: Caps,
}

/// Capability flags — the per-adapter row of the §6.2 matrix.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Caps {
    /// Level-1: incremental AccessKit tree diffs.
    pub tree_update: bool,
    /// Level-2 fallback: geometry + role/label polling.
    pub logical_snapshot: bool,
    /// Synthetic input through the real event pipeline (§9).
    pub input: bool,
    /// Last presented frame without re-render.
    pub last_frame: bool,
    /// Headless on-demand rendering.
    pub render_frame: bool,
    /// Virtual clock control (§14).
    pub clock: bool,
}

/// One window target as reported by an adapter (§16 v0.4 multi-window, M6).
/// The handle is the stable target id clients address windows by: the
/// AccessKit subtree `TreeId` string for grafted (multi-window) adapters,
/// [`guiwright_core::MAIN_WINDOW`] for single-window ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub handle: String,
    pub title: String,
    pub focused: bool,
}

/// Bounded pump: how long the adapter may spin its loop before returning.
#[derive(Clone, Copy, Debug)]
pub struct FrameDeadline {
    pub at: std::time::Instant,
}

impl FrameDeadline {
    pub fn within(timeout: std::time::Duration) -> Self {
        Self {
            at: std::time::Instant::now() + timeout,
        }
    }

    pub fn remaining(&self) -> std::time::Duration {
        self.at.saturating_duration_since(std::time::Instant::now())
    }

    pub fn expired(&self) -> bool {
        std::time::Instant::now() >= self.at
    }
}

/// A presented frame stamp (§10 `FramePresented { seq, hash }`); the hash is
/// the cheap visual-change signal (xxh3 of a scene/pixel digest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameStamp {
    /// Adapter-local sequence, monotonic per adapter.
    pub seq: u64,
    pub hash: u64,
}

/// What happened during a pump (§6.1 "Returns what happened").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpOutcome {
    /// Frames presented during this pump, in order.
    pub frames: Vec<FrameStamp>,
    /// The app still wants more frames (not quiescent yet).
    pub repaint_requested: bool,
    /// The app requested exit.
    pub quit_requested: bool,
}

impl PumpOutcome {
    /// Quiescent = the app asked for no more repaints. Presented frames do
    /// not imply more work: harness-driven adapters present one pass per
    /// pump even when the UI is static.
    pub fn is_quiescent(&self) -> bool {
        !self.repaint_requested
    }
}

/// Semantic actions (§8.4 `SemAction`) — adapters translate these to
/// AccessKit `ActionRequest`s or framework-native paths, preferring the
/// framework's own semantics.
#[derive(Clone, Debug, PartialEq)]
pub enum SemAction {
    Click,
    DoubleClick,
    SetValue {
        text: String,
    },
    Focus,
    Blur,
    Hover,
    /// Physical key press with modifiers (§9 keyboard vocabulary).
    Press {
        key: String,
        modifiers: Vec<String>,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    Check,
    Uncheck,
}

impl SemAction {
    /// Trace-friendly label.
    pub fn describe(&self) -> String {
        match self {
            SemAction::Click => "click".into(),
            SemAction::DoubleClick => "dblclick".into(),
            SemAction::SetValue { text } => format!("fill({text:?})"),
            SemAction::Focus => "focus".into(),
            SemAction::Blur => "blur".into(),
            SemAction::Hover => "hover".into(),
            SemAction::Press { key, modifiers } => format!("press({}+{key})", modifiers.join("+")),
            SemAction::Scroll { dx, dy } => format!("scroll({dx},{dy})"),
            SemAction::Check => "check".into(),
            SemAction::Uncheck => "uncheck".into(),
        }
    }
}

/// Result of applying a semantic action (§7 `ActionDone`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    /// The frame the action was applied at (rendezvous, §15.4).
    pub applied_at_frame: u64,
    pub detail: String,
}

/// Synthetic input events (§9 `InputEventV1`); coordinates are logical px
/// relative to the window — adapters own the scale conversion.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    PointerDown {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    PointerMove {
        x: f64,
        y: f64,
    },
    Wheel {
        dx: f64,
        dy: f64,
    },
    KeyDown {
        key: String,
        modifiers: Vec<String>,
    },
    KeyUp {
        key: String,
        modifiers: Vec<String>,
    },
    /// Text commit (IME-backed on some adapters, §4.2).
    TextInput {
        text: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

/// Per-adapter gesture pacing (§9 synthesis; §18 "per-adapter gesture
/// pacing tables, validated by cross-framework gesture tests"). Adapters
/// override [`FrameworkAdapter::gesture_pacing`] to encode their
/// framework's input quirks; the session's synthesis reads it per gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GesturePacing {
    /// Maximum pointer travel per synthesized drag move (logical px) —
    /// §9 "ratchet ~5 px/tick, mirroring slint's flickable-aware drag
    /// pacing". Non-positive values collapse to a single move.
    pub drag_ratchet_px: f64,
}

impl Default for GesturePacing {
    fn default() -> Self {
        Self {
            drag_ratchet_px: 5.0,
        }
    }
}

/// A captured frame (§6.1 `FrameRef`). M0 adapters ship hash-only frames;
/// pixel buffers arrive with the capture pipeline (§15.5, M1).
#[derive(Clone, Debug)]
pub struct FrameRef {
    pub stamp: FrameStamp,
    pub size: (u32, u32),
    /// Premultiplied RGBA8 when the adapter can produce pixels.
    pub pixels: Option<std::sync::Arc<[u8]>>,
}

/// Level-2 logical-tree diff (§6.1 `logical_snapshot`). The adapter
/// synthesizes an **AccessKit-shaped** [`accesskit::TreeUpdate`] from its
/// framework tree (roles from the widget-kind registry, labels from text,
/// bounds from visible rects, test ids): the consumer tree, selectors,
/// actionability and `expect` then run unchanged (§4.6 "adds role
/// semantics via a contributed widget-kind registry"). The adapter mints
/// the `NodeId`s, so [`FrameworkAdapter::act`] receives exactly those ids
/// back (§6.1 NodeKey normalization). Level-1 adapters return `None` from
/// `logical_snapshot`.
#[derive(Clone, Debug)]
pub struct LogicalTreeDiff {
    /// Owning adapter's static name — the [`guiwright_core::NodeKey::Logical`]
    /// vocabulary (e.g. `"iced"`).
    pub adapter: &'static str,
    /// The synthesized tree update (full update, carries `TreeInfo`).
    pub update: accesskit::TreeUpdate,
    /// Adapter-minted ids that left the tree since the last diff.
    pub removed: Vec<u64>,
    /// `(minted node id, test id)` — registered into the O(1) hash on the
    /// driver side after the update applies (§8.1 `getByTestId`).
    pub test_ids: Vec<(u64, String)>,
    /// `true` when anything was added/updated.
    pub changed: bool,
}

/// Virtual clock commands (§14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockCmd {
    InstallVirtual,
    /// Advance virtual time in frame-sized steps until drained.
    FastForward(std::time::Duration),
    RunFor(std::time::Duration),
    Pause,
    Resume,
    /// Absolute virtual time (§7 `SetTime(t)`).
    SetTime(std::time::Duration),
    /// Read the current state without mutating anything.
    Probe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockState {
    /// Current virtual time, when a virtual clock is installed.
    pub virtual_time: Option<std::time::Duration>,
    pub paused: bool,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AdapterError {
    /// The adapter's capability matrix row says no (§6.2).
    #[error("adapter does not support {0}")]
    Unsupported(&'static str),
    #[error("adapter internal error: {0}")]
    Internal(String),
}

/// The per-framework adapter trait (§6.1). Defaults encode "not supported"
/// so adapters implement exactly what their capability matrix advertises.
pub trait FrameworkAdapter: 'static {
    fn meta(&self) -> &AdapterMeta;

    /// The session's window targets (§16 v0.4, M6). Default: one `main`
    /// window — every single-window adapter inherits it unchanged.
    /// Multi-window adapters return one entry per window; **handles MUST
    /// equal the AccessKit subtree `TreeId` strings the adapter pushes** —
    /// the tree is the scoping source of truth
    /// ([`Session::window_scope`](crate::session::Session::window_scope)
    /// validates against it), `Window::Use` fails with `UnknownWindow` for
    /// handles the tree does not know.
    fn windows(&mut self) -> Vec<WindowInfo> {
        vec![WindowInfo {
            handle: guiwright_core::MAIN_WINDOW.to_string(),
            title: self.meta().framework.to_string(),
            focused: true,
        }]
    }

    /// Pump frames until the app is quiescent or `deadline` hits (§6.1).
    fn pump(&mut self, deadline: FrameDeadline) -> PumpOutcome;

    /// Latest semantic-tree diff (Level-1). `None` = unchanged (§6.1).
    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate>;

    /// Level-2 fallback: geometry + role/label polling.
    fn logical_snapshot(&mut self) -> Option<LogicalTreeDiff> {
        None
    }

    /// Apply a semantic action on a resolved node (§8.4).
    fn act(
        &mut self,
        node: accesskit::NodeId,
        action: SemAction,
    ) -> Result<ActionOutcome, AdapterError>;

    /// Inject synthetic input through the framework's REAL pipeline (§9).
    /// The single-window hook: coordinates are logical px relative to the
    /// adapter's only window.
    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        let _ = ev;
        Err(AdapterError::Unsupported("input"))
    }

    /// Window-targeted input injection (§9 coordinates, M7): `window` is
    /// the target handle as reported by [`FrameworkAdapter::windows`] and
    /// coordinates are logical px **relative to that window**. Default
    /// ignores the handle — a single-window adapter has exactly one
    /// coordinate space, so every existing adapter keeps compiling and
    /// stays honest (the M6 `windows()` pattern). Multi-window adapters
    /// override to route; the session validates the handle against the
    /// tree first, so the handle is always live.
    fn input_window(&mut self, window: &str, ev: InputEvent) -> Result<(), AdapterError> {
        let _ = window;
        self.input(ev)
    }

    /// This adapter's gesture pacing table (§9 synthesis; §18). The
    /// default is the §9 ratchet (5 px/tick).
    fn gesture_pacing(&self) -> GesturePacing {
        GesturePacing::default()
    }

    /// Last presented frame — no re-render (§10).
    fn last_frame(&mut self) -> Option<FrameRef> {
        None
    }

    /// Render on demand (headless-capable adapters only, §11).
    fn render_frame(&mut self, size: Option<(u32, u32)>) -> Result<FrameRef, AdapterError> {
        let _ = size;
        Err(AdapterError::Unsupported("render_frame"))
    }

    /// Virtual clock (§14).
    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        let _ = cmd;
        Err(AdapterError::Unsupported("clock"))
    }

    /// State seams (§14 mock domain): execute `task` **inside the
    /// adapter's runner context** — the app state owned by the UI thread.
    /// The task receives `&mut dyn Any`; adapters document which concrete
    /// type they hand over (the mock hands over what
    /// [`crate::mock::MockAdapter::with_state`] received). Default = the
    /// adapter has no seam surface (capability-matrix row says no, §6.2).
    fn run_in(&mut self, task: crate::seams::SeamTask) -> Result<String, AdapterError> {
        let _ = task;
        Err(AdapterError::Unsupported("run_in (state seams, §14)"))
    }
}
