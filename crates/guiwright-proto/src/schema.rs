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
}
