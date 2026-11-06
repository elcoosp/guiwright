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

/// A captured frame (§6.1 `FrameRef`). M0 adapters ship hash-only frames;
/// pixel buffers arrive with the capture pipeline (§15.5, M1).
#[derive(Clone, Debug)]
pub struct FrameRef {
    pub stamp: FrameStamp,
    pub size: (u32, u32),
    /// Premultiplied RGBA8 when the adapter can produce pixels.
    pub pixels: Option<std::sync::Arc<[u8]>>,
}

/// Level-2 logical-tree diff (§6.1 `logical_snapshot`). Populated by
/// adapters without an AccessKit tree (iced, floem). Level-1 adapters
/// return `None` from `logical_snapshot`.
#[derive(Clone, Debug, Default)]
pub struct LogicalTreeDiff {
    /// Adapter-minted logical ids that left the tree.
    pub removed: Vec<u64>,
    /// `true` when anything was added/updated (adapter-specific payload
    /// lands with the iced/floem adapters, M1).
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
    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        let _ = ev;
        Err(AdapterError::Unsupported("input"))
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
}
