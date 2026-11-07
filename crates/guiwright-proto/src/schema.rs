//! Guiwright Protocol (GWP) — wire schema V1 (architecture §7).
//!
//! CDP-inspired: versioned, binary-first (postcard). Every payload enum has
//! an explicit version variant (`GwpRequest::V1`) so old clients get clean
//! errors instead of garbage. Wire types are decoupled from
//! `guiwright-core` types on purpose: the wire schema can evolve
//! independently (AddError-style explicit `V1` variants, §7 "old clients
//! get clean errors").

use serde::{Deserialize, Serialize};

/// Current protocol version. Bumped on breaking schema changes; the `V1`
/// variant stays parseable forever.
pub const PROTO_VERSION: u32 = 1;

/// Maximum encoded frame body size (64 MiB) — guards against corrupt or
/// hostile length prefixes.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Maximum decompressed blob size (256 MiB, the default trace ring cap §12).
pub const MAX_BLOB_BYTES: usize = 256 * 1024 * 1024;

/// Client → driver request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GwpRequest {
    V1(RequestV1),
}

/// Driver → client response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GwpResponse {
    V1(ResponseV1),
}

/// Server-push events (multiplexed by `seq` per topic, §7).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GwpEvent {
    V1(EventV1),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum RequestV1 {
    /// Handshake (§7). Must be the first request on a connection.
    Hello { proto_version: u32, agent: String },
    /// Liveness/latency probe.
    Ping,
    /// Snapshot of the logical tree (summary in M0; full snapshot rides the
    /// blob side-channel once traces land).
    TreeSnapshot,
    /// Resolve a selector with the strict-mode policy (§8.2).
    Resolve {
        selector: SelectorV1,
        timeout_ms: u64,
    },
    /// Apply a semantic action on a resolved node (§8.4).
    Action {
        node: NodeKeyV1,
        generation: u32,
        action: SemActionV1,
        force: bool,
        timeout_ms: u64,
    },
    /// Inject synthetic input through the real pipeline (§9).
    Input { events: Vec<InputEventV1> },
    /// Virtual clock domain (§7 `Clock` + §14): install/fast-forward/pause.
    Clock(ClockV1),
    /// Mock domain (§7 `Mock` + §14 state seams): install/clear/reset.
    Mock(MockV1),
    /// Inspector domain (§7 `Inspect` + §13): overlay + codegen recording.
    Inspect(InspectV1),
    /// Close the session cleanly.
    Bye,
    /// Video domain (§7 `Video` + §10 "the trace recorder doubles as
    /// video"): start/stop a frame recording of the session.
    ///
    /// Wire discipline: request/response variants are **append-only**
    /// within a `V1` envelope — `Video` follows `Bye` (discriminant 10)
    /// so every pre-M3 client keeps its byte-exact encodings; the
    /// pinning tests below fail the build on any reordering.
    Video(VideoV1),
    /// Window domain (§7 + §16 v0.4 multi-window, M6): enumerate the
    /// session's window targets and aim the calling connection at one of
    /// them — Playwright's "targets per window". Wire discipline: appended
    /// after `Video` (discriminant 11); see [`RequestV1::Video`].
    Window(WindowV1),
    /// Gesture domain (§9 input pipeline, M7): high-level synthetic
    /// gestures — the session synthesizes the raw event sequence
    /// (per-adapter pacing, §18) and injects it through the target
    /// window's real pipeline. Wire discipline: appended after `Window`
    /// (discriminant 12); see [`RequestV1::Video`].
    Gesture(GestureV1),
}

/// `Clock { InstallVirtual | FastForward(ms) | RunFor(ms) | Pause | Resume |
/// SetTime(t) }` (§7) plus the state probe the kit needs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockV1 {
    InstallVirtual,
    FastForward {
        ms: u64,
    },
    RunFor {
        ms: u64,
    },
    Pause,
    Resume,
    /// Absolute virtual time (§7 `SetTime(t)`), milliseconds.
    SetTime {
        ms: u64,
    },
    /// Read the current clock state without mutating it.
    State,
}

/// `Mock { InstallSeam { id, response } | Clear(id) | ResetState { scope } }`
/// (§7) plus `Call` — run a registered seam once from the test side.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MockV1 {
    /// Install (or re-aim) a seam by its registered id. `response` presets a
    /// canned result for the mock domain (None = run the real hook).
    InstallSeam {
        id: String,
        response: Option<String>,
    },
    /// Remove a seam override; the real hook (if any) runs again.
    Clear { id: String },
    /// Reset app state to a fixture scope (§14 "fixture parity with
    /// Playwright contexts") — runs the session's reset seam.
    ResetState { scope: String },
    /// Execute a registered seam once (arg = scope/fixture tag).
    Call { id: String, scope: String },
}

/// `Inspect { EnableOverlay | Highlight(handle) | StartRecording |
/// StopRecording }` (§7). M2 ships the codegen recorder half (§13); the
/// live highlight handles land with the interactive overlay session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum InspectV1 {
    /// Start the interaction → code recorder (§13).
    StartRecording,
    /// Stop and fetch the generated kit code.
    StopRecording,
}

/// `Video { Start(opts) | Stop }` → `VideoFile { path }` (§7, §10). The
/// driver records presented frames between the two calls: hash-equality
/// dedup drops static repetitions (§10 backpressure), fps paces the ring,
/// and the encoder subprocess (ffmpeg) or PNG-sequence fallback finalizes
/// the file at `Stop`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoV1 {
    /// Start recording with these options (§10).
    Start(VideoOptsV1),
    /// Stop and finalize the recording file.
    Stop,
}

/// Video recording options (§10).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoOptsV1 {
    /// Target frame-rate cap. `0` selects the driver default (25 fps).
    /// Frames presented faster than `1/fps` are collapsed by the
    /// hash-equality dedup — a static UI records to keyframe + no-ops.
    pub fps: u32,
    /// Output directory for the recording (Playwright-style: the client
    /// owns the artifact dir). `None` = the driver's default
    /// (`GUIWRIGHT_VIDEO_DIR` or the OS temp dir). Relative paths resolve
    /// against the driver process's cwd.
    pub dir: Option<String>,
}

impl Default for VideoOptsV1 {
    fn default() -> Self {
        Self {
            fps: VIDEO_DEFAULT_FPS,
            dir: None,
        }
    }
}

/// Driver default frame-rate cap (§10: Playwright records ≈25 fps).
pub const VIDEO_DEFAULT_FPS: u32 = 25;

/// `Window { List | Use { handle } }` (§7 + §16 v0.4, M6). A multi-window
/// app exposes one target per window; `Use` aims the **calling connection**
/// at one target — subsequent `Resolve` requests (and the auto-wait they
/// drive) only see that window's subtree. Other connections keep their own
/// target: per-connection state lives on the server bridge, not in the
/// shared session (§11 "one driver session serves all connections").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowV1 {
    /// List the session's window targets → `Windows`.
    List,
    /// Aim this connection at a window target → `WindowUsed`, or
    /// `Error { UnknownWindow }` when the handle is unknown (never existed
    /// or closed). The node identities of `Action` are already
    /// window-unique (NodeKey carries the TreeId), so scoping concerns
    /// resolution only.
    Use { handle: String },
}

/// One window target as reported by `Windows` (§16 v0.4, M6). `handle` is
/// the stable target id — the AccessKit subtree `TreeId` for grafted
/// (multi-window) adapters, `"main"` for single-window adapters.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowInfoV1 {
    pub handle: String,
    pub title: String,
    pub focused: bool,
}

/// `Gesture { Click | Drag | Wheel | Key | Type }` (§9 synthesis, M7): the
/// wire-level gesture vocabulary. The **driver** synthesizes the raw
/// `InputEventV1` sequence (click = move→down→up; drag = down→
/// ratchet-interpolated moves→up, §9 "ratchet ~5 px/tick"; key = down→up;
/// type = one text commit) from its adapter's pacing table — synthesis is
/// driver-side so each framework's quirks stay encoded at one place
/// (§18 "per-adapter gesture pacing tables"). Coordinates are logical px
/// **relative to the target window** (the sending connection's active
/// target, defaulting to `main`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GestureV1 {
    /// Press and release at a window-relative position. `count` follows
    /// Playwright's clickCount: 1 = click, 2 = double-click (down/up
    /// repeated at the same point).
    Click {
        x: f64,
        y: f64,
        button: PointerButtonV1,
        count: u32,
    },
    /// Drag from one window-relative position to another; the driver
    /// interpolates the moves (§9 ratchet pacing) so hover/drag state
    /// machines see a real pointer path.
    Drag {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        button: PointerButtonV1,
    },
    /// Wheel scroll by logical-px deltas.
    Wheel { dx: f64, dy: f64 },
    /// Press and release a key with modifiers (key names are the §8.4
    /// `Press` vocabulary, e.g. "Enter", "a").
    Key { key: String, modifiers: Vec<String> },
    /// Commit text in one step (adapters translate to IME or per-key
    /// events per §4.2 keyboard fidelity).
    Type { text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ResponseV1 {
    /// Handshake reply (§7 `Welcome`).
    Welcome {
        proto_version: u32,
        session_id: String,
        tree_seq: u64,
        frame_seq: u64,
        adapter: AdapterInfoV1,
    },
    Pong,
    TreeSnapshot {
        tree_seq: u64,
        node_count: usize,
    },
    Resolved {
        node: NodeKeyV1,
        generation: u32,
        /// Window-relative bounds, physical px (AccessKit convention).
        rect: Option<[f64; 4]>,
    },
    ActionDone {
        /// The frame the action was applied at — clients can await exactly
        /// this frame, no polling (§7 multiplexing).
        frame_seq: u64,
        detail: String,
    },
    InputQueued {
        frame_seq: u64,
    },
    /// Clock state reply (§14 `ClockState`), milliseconds on the wire.
    ClockState {
        virtual_time_ms: Option<u64>,
        paused: bool,
        /// Jumps applied by the servicing command (frame-sized steps).
        steps: u64,
    },
    /// Mock domain reply — seam executed or bookkeeping applied.
    MockDone {
        id: String,
        /// Seam result detail (or the canned override response).
        detail: String,
    },
    /// Inspector reply — the generated kit code (§13).
    Codegen {
        code: String,
        /// Strict-mode warnings collected while recording (§13).
        warnings: Vec<String>,
    },
    /// Rejected request: unknown version, bad sequence, adapter error…
    Error {
        code: ErrorCode,
        message: String,
    },
    Bye,
    /// Video domain reply (§10): the finalized recording file. Append-only
    /// wire discipline — see [`RequestV1::Video`]. `dropped` counts frames
    /// shed by the §10 hash-equality backpressure (static UI compresses to
    /// a keyframe + no-ops), `duration_ms` is the wall-clock span the
    /// recording covers.
    VideoFile {
        path: String,
        frames: u64,
        dropped: u64,
        duration_ms: u64,
    },
    /// Window domain reply (§16 v0.4, M6): the session's window targets.
    /// Append-only wire discipline — see [`RequestV1::Video`].
    Windows {
        windows: Vec<WindowInfoV1>,
    },
    /// Window domain reply: this connection is now aimed at the window
    /// target (§16 v0.4). The server bridge records the active handle and
    /// stamps subsequent requests with it.
    WindowUsed {
        handle: String,
    },
    /// Gesture domain reply (§9, M7): the synthesized event batch was
    /// injected through the target window's pipeline and applied inside
    /// the next frame. Append-only wire discipline — see
    /// [`RequestV1::Video`].
    GestureDone {
        /// The frame the gesture was applied at (rendezvous, §15.4).
        frame_seq: u64,
        /// Human-readable synthesis summary (event count, positions).
        detail: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum EventV1 {
    TreeChanged { seq: u64 },
    FramePresented { seq: u64, hash: u64 },
    Log { record: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    UnsupportedVersion,
    NotHandshaked,
    UnknownNode,
    StaleGeneration,
    StrictModeViolation,
    Timeout,
    Unsupported,
    Internal,
    /// M6 (§16 v0.4): `Window::Use` named a handle the session does not
    /// know (never existed or closed). Append-only — see the pinning test.
    UnknownWindow,
}

/// Adapter self-description reported in `Welcome` (§6.1 `AdapterMeta`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AdapterInfoV1 {
    pub name: String,
    pub framework: String,
    pub framework_version: String,
    /// Capability flags (§6.2 capability matrix).
    pub caps: CapsV1,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsV1 {
    pub tree_update: bool,
    pub logical_snapshot: bool,
    pub input: bool,
    pub last_frame: bool,
    pub render_frame: bool,
    pub clock: bool,
}

/// Wire form of [`guiwright_core::NodeKey`] (§6.1, normalized identities).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeKeyV1 {
    AccessKit {
        /// AccessKit `TreeId` is a Uuid on the wire.
        tree: [u8; 16],
        node: u64,
    },
    Logical {
        adapter: String,
        id: u64,
    },
}

/// Wire form of [`guiwright_core::Selector`] (§8.1 `SelectorV1`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SelectorV1 {
    Role {
        /// `accesskit::Role` as u8 discriminant string for forward
        /// compatibility we serialize the Debug-ish name.
        role: String,
        name: Option<TextMatchV1>,
    },
    Label(TextMatchV1),
    Text(TextMatchV1),
    TestId(String),
    Value(TextMatchV1),
    All(Vec<SelectorV1>),
    Filter {
        source: Box<SelectorV1>,
        has_text: Option<TextMatchV1>,
        has_not: Option<Box<SelectorV1>>,
        nth: Option<NthV1>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TextMatchV1 {
    pub text: String,
    pub exact: bool,
    pub ignore_case: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NthV1 {
    First,
    Last,
    Index(usize),
}

/// Wire form of [`guiwright_driver::SemAction`] (§8.4).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SemActionV1 {
    Click,
    DoubleClick,
    SetValue { text: String },
    Focus,
    Blur,
    Hover,
    Press { key: String, modifiers: Vec<String> },
    Scroll { dx: f64, dy: f64 },
    Check,
    Uncheck,
}

/// Wire form of [`guiwright_driver::InputEvent`] (§9).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum InputEventV1 {
    PointerDown {
        x: f64,
        y: f64,
        button: PointerButtonV1,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: PointerButtonV1,
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
    /// Text commit (IME-backed on some adapters, §4.2 keyboard fidelity).
    TextInput {
        text: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PointerButtonV1 {
    Primary,
    Secondary,
    Middle,
}

impl From<guiwright_core::NodeKey> for NodeKeyV1 {
    fn from(key: guiwright_core::NodeKey) -> Self {
        use guiwright_core::NodeKey;
        match key {
            NodeKey::AccessKit { tree, node } => NodeKeyV1::AccessKit {
                tree: tree.0.as_bytes().to_owned(),
                node: node.0,
            },
            NodeKey::Logical { adapter, id } => NodeKeyV1::Logical {
                adapter: adapter.to_string(),
                id,
            },
        }
    }
}

impl From<NodeKeyV1> for guiwright_core::NodeKey {
    fn from(v: NodeKeyV1) -> Self {
        use guiwright_core::NodeKey;
        match v {
            NodeKeyV1::AccessKit { tree, node } => NodeKey::AccessKit {
                tree: accesskit::TreeId(uuid::Uuid::from_bytes(tree)),
                node: accesskit::NodeId(node),
            },
            NodeKeyV1::Logical { adapter, id } => {
                // Box-leak to a &'static str: adapter names are static
                // registry keys, and NodeKey is Copy. The count of distinct
                // adapters is bounded and tiny.
                NodeKey::Logical {
                    adapter: leak_adapter_name(adapter),
                    id,
                }
            }
        }
    }
}

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::OnceLock;

fn adapter_registry() -> &'static Mutex<HashSet<&'static str>> {
    static REGISTRY: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

fn leak_adapter_name(name: String) -> &'static str {
    let mut registry = adapter_registry().lock().expect("adapter registry");
    if let Some(existing) = registry.get(name.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.into_boxed_str());
    registry.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_core::{NodeKey, Selector, TextMatch};
    use std::collections::HashSet;

    #[test]
    fn node_key_roundtrip_accesskit() {
        let key = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: accesskit::NodeId(42),
        };
        let wire: NodeKeyV1 = key.into();
        let back: NodeKey = wire.into();
        assert_eq!(key, back);
    }

    #[test]
    fn node_key_roundtrip_logical() {
        let key = NodeKey::Logical {
            adapter: "iced",
            id: 7,
        };
        let wire: NodeKeyV1 = key.into();
        let back: NodeKey = wire.into();
        assert_eq!(key, back);
        // Different payload, same adapter name → identical &'static.
        let again: NodeKey = NodeKeyV1::Logical {
            adapter: "iced".into(),
            id: 9,
        }
        .into();
        let NodeKey::Logical { adapter, .. } = again else {
            panic!("logical");
        };
        let NodeKey::Logical { adapter: first, .. } = back else {
            panic!("logical");
        };
        assert!(std::ptr::eq(adapter, first), "interned adapter name");
    }

    #[test]
    fn selector_roundtrip_preserves_matching_semantics() {
        let selector = Selector::role_named(accesskit::Role::Button, "Save").first();
        let wire: SelectorV1 = selector.clone().into();
        let back: Selector = wire.into();
        assert_eq!(selector, back);
    }

    #[test]
    fn text_match_v1_roundtrip() {
        let m = TextMatch::exact("Ada");
        let wire: TextMatchV1 = m.clone().into();
        let back: TextMatch = wire.into();
        assert_eq!(m, back);
    }

    #[test]
    fn postcard_roundtrip_all_requests() {
        let requests = vec![
            GwpRequest::V1(RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: "guiwright-kit/0.1".into(),
            }),
            GwpRequest::V1(RequestV1::Ping),
            GwpRequest::V1(RequestV1::TreeSnapshot),
            GwpRequest::V1(RequestV1::Resolve {
                selector: SelectorV1::TestId("save".into()),
                timeout_ms: 5000,
            }),
            GwpRequest::V1(RequestV1::Action {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 2,
                },
                generation: 0,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 5000,
            }),
            GwpRequest::V1(RequestV1::Input {
                events: vec![InputEventV1::PointerDown {
                    x: 10.0,
                    y: 20.0,
                    button: PointerButtonV1::Primary,
                }],
            }),
            GwpRequest::V1(RequestV1::Clock(ClockV1::FastForward { ms: 30_000 })),
            GwpRequest::V1(RequestV1::Mock(MockV1::ResetState {
                scope: "seed-42".into(),
            })),
            GwpRequest::V1(RequestV1::Mock(MockV1::InstallSeam {
                id: "reset_db".into(),
                response: None,
            })),
            GwpRequest::V1(RequestV1::Inspect(InspectV1::StartRecording)),
            GwpRequest::V1(RequestV1::Bye),
        ];
        let seen: HashSet<String> = HashSet::new();
        let _ = seen;
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
    }

    #[test]
    fn postcard_roundtrip_all_responses() {
        let responses = vec![
            GwpResponse::V1(ResponseV1::Welcome {
                proto_version: PROTO_VERSION,
                session_id: "s-1".into(),
                tree_seq: 3,
                frame_seq: 17,
                adapter: AdapterInfoV1 {
                    name: "egui".into(),
                    framework: "egui".into(),
                    framework_version: "0.37.0".into(),
                    caps: CapsV1 {
                        tree_update: true,
                        input: true,
                        clock: true,
                        ..Default::default()
                    },
                },
            }),
            GwpResponse::V1(ResponseV1::Pong),
            GwpResponse::V1(ResponseV1::Resolved {
                node: NodeKeyV1::Logical {
                    adapter: "iced".into(),
                    id: 5,
                },
                generation: 1,
                rect: Some([0.0, 0.0, 100.0, 40.0]),
            }),
            GwpResponse::V1(ResponseV1::ActionDone {
                frame_seq: 18,
                detail: "clicked".into(),
            }),
            GwpResponse::V1(ResponseV1::ClockState {
                virtual_time_ms: Some(30_000),
                paused: false,
                steps: 2,
            }),
            GwpResponse::V1(ResponseV1::MockDone {
                id: "reset_db".into(),
                detail: "reset to seed-42".into(),
            }),
            GwpResponse::V1(ResponseV1::Codegen {
                code: "app.get_by_test_id(\"save\").click()".into(),
                warnings: vec![],
            }),
            GwpResponse::V1(ResponseV1::Error {
                code: ErrorCode::UnsupportedVersion,
                message: "client sent proto 0".into(),
            }),
        ];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    #[test]
    fn events_roundtrip() {
        let events = vec![
            GwpEvent::V1(EventV1::TreeChanged { seq: 9 }),
            GwpEvent::V1(EventV1::FramePresented {
                seq: 10,
                hash: 0xAB,
            }),
            GwpEvent::V1(EventV1::Log {
                record: "hello".into(),
            }),
        ];
        for ev in events {
            let bytes = postcard::to_allocvec(&ev).expect("encode");
            let back: GwpEvent = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(ev, back);
        }
    }

    /// §7 clock & mock domains round-trip (M2).
    #[test]
    fn clock_and_mock_domains_roundtrip() {
        let clocks = [
            ClockV1::InstallVirtual,
            ClockV1::FastForward { ms: 16 },
            ClockV1::RunFor { ms: 500 },
            ClockV1::Pause,
            ClockV1::Resume,
            ClockV1::SetTime { ms: 120_000 },
            ClockV1::State,
        ];
        for cmd in clocks {
            let bytes = postcard::to_allocvec(&cmd).expect("encode");
            let back: ClockV1 = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(cmd, back);
        }
        let mocks = [
            MockV1::InstallSeam {
                id: "reset_db".into(),
                response: Some("fixture-ok".into()),
            },
            MockV1::Clear {
                id: "reset_db".into(),
            },
            MockV1::ResetState {
                scope: "seed-42".into(),
            },
            MockV1::Call {
                id: "seed_users".into(),
                scope: "smoke".into(),
            },
        ];
        for cmd in mocks {
            let bytes = postcard::to_allocvec(&cmd).expect("encode");
            let back: MockV1 = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(cmd, back);
        }
    }

    /// §7 `Video` domain round-trip (M3, §10).
    #[test]
    fn video_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Video(VideoV1::Start(VideoOptsV1 {
                fps: 30,
                dir: Some("test-results/videos".into()),
            }))),
            GwpRequest::V1(RequestV1::Video(VideoV1::Start(VideoOptsV1::default()))),
            GwpRequest::V1(RequestV1::Video(VideoV1::Stop)),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let resp = GwpResponse::V1(ResponseV1::VideoFile {
            path: "guiwright-smoke-1730889600.webm".into(),
            frames: 42,
            dropped: 7,
            duration_ms: 1680,
        });
        let bytes = postcard::to_allocvec(&resp).expect("encode");
        let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(resp, back);
    }

    /// §7 `Window` domain round-trip (M6, §16 v0.4 multi-window).
    #[test]
    fn window_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Window(WindowV1::List)),
            GwpRequest::V1(RequestV1::Window(WindowV1::Use {
                handle: "a6000000-0000-0000-0000-000000000001".into(),
            })),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [
            GwpResponse::V1(ResponseV1::Windows {
                windows: vec![WindowInfoV1 {
                    handle: "a6000000-0000-0000-0000-000000000001".into(),
                    title: "Editor".into(),
                    focused: true,
                }],
            }),
            GwpResponse::V1(ResponseV1::WindowUsed {
                handle: "a6000000-0000-0000-0000-000000000001".into(),
            }),
        ];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// §7 `Gesture` domain round-trip (M7, §9 input pipeline): the
    /// high-level synthetic gestures and the `GestureDone` reply.
    #[test]
    fn gesture_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Click {
                x: 12.5,
                y: 30.0,
                button: PointerButtonV1::Primary,
                count: 1,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Click {
                x: 4.0,
                y: 8.0,
                button: PointerButtonV1::Secondary,
                count: 2,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Drag {
                from_x: 10.0,
                from_y: 10.0,
                to_x: 110.0,
                to_y: 60.0,
                button: PointerButtonV1::Primary,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Wheel { dx: 0.0, dy: -120.0 })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Key {
                key: "Enter".into(),
                modifiers: vec!["Shift".into()],
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Type {
                text: "héllo wörld".into(),
            })),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [GwpResponse::V1(ResponseV1::GestureDone {
            frame_seq: 20,
            detail: "clicked at (12.5, 30) — 3 events".into(),
        })];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// postcard encodes enum variants as a varint of the variant index, so
    /// **variant order is the wire format**. §7 "old clients get clean
    /// errors" → within a `V1` envelope variants are append-only: pre-M3
    /// requests keep discriminants 0–9 (`Bye` = 9), `Video` appends at
    /// 10, the M6 `Window` domain at 11 and the M7 `Gesture` domain at 12;
    /// responses keep 0–10 (`Bye` = 10), `VideoFile` appends at 11, the
    /// M6 window replies at 12–13 and the M7 `GestureDone` reply at 14.
    /// Reordering or inserting mid-enum fails this test — add new variants
    /// at the end and extend the pin list instead.
    #[test]
    fn wire_discriminants_are_append_only() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            // The first byte(s) of a postcard enum are the varint variant
            // index; every variant index pinned here is < 128 (single byte).
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }

        // Requests (M0–M2 alphabet kept intact, M3 `Video` appended).
        assert_eq!(
            discriminant(&RequestV1::Hello {
                proto_version: 1,
                agent: String::new()
            }),
            0
        );
        assert_eq!(discriminant(&RequestV1::Ping), 1);
        assert_eq!(discriminant(&RequestV1::TreeSnapshot), 2);
        assert_eq!(
            discriminant(&RequestV1::Resolve {
                selector: SelectorV1::TestId(String::new()),
                timeout_ms: 0
            }),
            3
        );
        assert_eq!(
            discriminant(&RequestV1::Action {
                node: NodeKeyV1::Logical {
                    adapter: String::new(),
                    id: 0
                },
                generation: 0,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 0
            }),
            4
        );
        assert_eq!(discriminant(&RequestV1::Input { events: Vec::new() }), 5);
        assert_eq!(discriminant(&RequestV1::Clock(ClockV1::State)), 6);
        assert_eq!(
            discriminant(&RequestV1::Mock(MockV1::Clear { id: String::new() })),
            7
        );
        assert_eq!(
            discriminant(&RequestV1::Inspect(InspectV1::StartRecording)),
            8
        );
        assert_eq!(discriminant(&RequestV1::Bye), 9);
        assert_eq!(discriminant(&RequestV1::Video(VideoV1::Stop)), 10);
        assert_eq!(discriminant(&RequestV1::Window(WindowV1::List)), 11);
        // M7 (§9): the gesture domain appends after Window.
        assert_eq!(
            discriminant(&RequestV1::Gesture(GestureV1::Wheel { dx: 0.0, dy: 0.0 })),
            12
        );

        // Responses.
        assert_eq!(
            discriminant(&ResponseV1::Welcome {
                proto_version: 1,
                session_id: String::new(),
                tree_seq: 0,
                frame_seq: 0,
                adapter: AdapterInfoV1 {
                    name: String::new(),
                    framework: String::new(),
                    framework_version: String::new(),
                    caps: CapsV1::default()
                }
            }),
            0
        );
        assert_eq!(discriminant(&ResponseV1::Pong), 1);
        assert_eq!(
            discriminant(&ResponseV1::TreeSnapshot {
                tree_seq: 0,
                node_count: 0
            }),
            2
        );
        assert_eq!(
            discriminant(&ResponseV1::Resolved {
                node: NodeKeyV1::Logical {
                    adapter: String::new(),
                    id: 0
                },
                generation: 0,
                rect: None
            }),
            3
        );
        assert_eq!(
            discriminant(&ResponseV1::ActionDone {
                frame_seq: 0,
                detail: String::new()
            }),
            4
        );
        assert_eq!(discriminant(&ResponseV1::InputQueued { frame_seq: 0 }), 5);
        assert_eq!(
            discriminant(&ResponseV1::ClockState {
                virtual_time_ms: None,
                paused: false,
                steps: 0
            }),
            6
        );
        assert_eq!(
            discriminant(&ResponseV1::MockDone {
                id: String::new(),
                detail: String::new()
            }),
            7
        );
        assert_eq!(
            discriminant(&ResponseV1::Codegen {
                code: String::new(),
                warnings: Vec::new()
            }),
            8
        );
        assert_eq!(
            discriminant(&ResponseV1::Error {
                code: ErrorCode::Internal,
                message: String::new()
            }),
            9
        );
        assert_eq!(discriminant(&ResponseV1::Bye), 10);
        assert_eq!(
            discriminant(&ResponseV1::VideoFile {
                path: String::new(),
                frames: 0,
                dropped: 0,
                duration_ms: 0
            }),
            11
        );
        assert_eq!(
            discriminant(&ResponseV1::Windows {
                windows: Vec::new()
            }),
            12
        );
        assert_eq!(
            discriminant(&ResponseV1::WindowUsed {
                handle: String::new()
            }),
            13
        );
        assert_eq!(
            discriminant(&ResponseV1::GestureDone {
                frame_seq: 0,
                detail: String::new()
            }),
            14
        );

        // Nested domain enums are pinned too — other-language clients
        // hard-code these tables (clients/typescript).
        assert_eq!(discriminant(&VideoV1::Start(VideoOptsV1::default())), 0);
        assert_eq!(discriminant(&VideoV1::Stop), 1);
        assert_eq!(discriminant(&ClockV1::InstallVirtual), 0);
        assert_eq!(discriminant(&ClockV1::State), 6);
        assert_eq!(
            discriminant(&MockV1::InstallSeam {
                id: String::new(),
                response: None
            }),
            0
        );
        assert_eq!(
            discriminant(&MockV1::Call {
                id: String::new(),
                scope: String::new()
            }),
            3
        );
        assert_eq!(discriminant(&InspectV1::StartRecording), 0);
        assert_eq!(discriminant(&InspectV1::StopRecording), 1);
        assert_eq!(discriminant(&WindowV1::List), 0);
        assert_eq!(
            discriminant(&WindowV1::Use {
                handle: String::new()
            }),
            1
        );
        assert_eq!(
            discriminant(&GestureV1::Click {
                x: 0.0,
                y: 0.0,
                button: PointerButtonV1::Primary,
                count: 1
            }),
            0
        );
        assert_eq!(
            discriminant(&GestureV1::Drag {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 0.0,
                to_y: 0.0,
                button: PointerButtonV1::Primary
            }),
            1
        );
        assert_eq!(discriminant(&GestureV1::Wheel { dx: 0.0, dy: 0.0 }), 2);
        assert_eq!(
            discriminant(&GestureV1::Key {
                key: String::new(),
                modifiers: Vec::new()
            }),
            3
        );
        assert_eq!(discriminant(&GestureV1::Type { text: String::new() }), 4);
        // ErrorCode is flat on the wire (u8 in Error payloads) — appending
        // UnknownWindow keeps the pre-M6 codes byte-identical.
        assert_eq!(discriminant(&ErrorCode::Internal), 7);
        assert_eq!(discriminant(&ErrorCode::UnknownWindow), 8);
    }
}
