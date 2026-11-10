//! Golden wire vectors for the other-language clients (§16, M3):
//! `clients/typescript/test/golden.json` pins the exact postcard bytes the
//! Rust codec produces so the TypeScript codec can be verified against
//! the real wire, byte for byte.
//!
//! Bless after an intentional schema change (same flow as the schema
//! golden):
//!
//! ```text
//! GUIWRIGHT_BLESS=1 cargo test -p guiwright-proto --test golden_vectors
//! ```
//!
//! The TypeScript side re-pins the vendor copy in
//! `test/codec.test.ts`; the two fail the build together on drift.

use guiwright_proto::framing::encode_frame;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, ClockV1, ErrorCode, GestureV1, InputEventV1, InspectV1, MockV1,
    NodeKeyV1, PROTO_VERSION, PointerButtonV1, RequestV1, ResponseV1, SelectorV1, SemActionV1,
    TextMatchV1, VideoOptsV1, VideoV1, WindowInfoV1, WindowV1,
};

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../clients/typescript/test/golden.json"
);

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").expect("string write never fails");
    }
    out
}

/// Core session requests (§16 v0.4): hello / ping / tree / resolve.
fn core_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        (
            "hello",
            RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: "@guiwright/gwp/0.3".into(),
            },
        ),
        ("ping", RequestV1::Ping),
        ("tree_snapshot", RequestV1::TreeSnapshot),
        (
            "resolve_role_named",
            RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                timeout_ms: 5000,
            },
        ),
        (
            "resolve_test_id",
            RequestV1::Resolve {
                selector: SelectorV1::TestId("btn-save".into()),
                timeout_ms: 0,
            },
        ),
    ]
}
/// Action + raw input requests (the §9 input surface).
fn action_input_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        (
            "action_click",
            RequestV1::Action {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 2,
                },
                generation: 0,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 5000,
            },
        ),
        (
            "action_press",
            RequestV1::Action {
                node: NodeKeyV1::Logical {
                    adapter: "floem".into(),
                    id: 7,
                },
                generation: 3,
                action: SemActionV1::Press {
                    key: "Enter".into(),
                    modifiers: vec!["Shift".into()],
                },
                force: true,
                timeout_ms: 1000,
            },
        ),
        (
            "input_pointer",
            RequestV1::Input {
                events: vec![
                    InputEventV1::PointerMove { x: 12.5, y: 30.0 },
                    InputEventV1::PointerDown {
                        x: 12.5,
                        y: 30.0,
                        button: PointerButtonV1::Primary,
                    },
                    InputEventV1::PointerUp {
                        x: 12.5,
                        y: 30.0,
                        button: PointerButtonV1::Primary,
                    },
                ],
            },
        ),
    ]
}

/// Clock / mock / inspect / video service requests.
fn service_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        (
            "clock_fast_forward",
            RequestV1::Clock(ClockV1::FastForward { ms: 30_000 }),
        ),
        ("clock_state", RequestV1::Clock(ClockV1::State)),
        (
            "mock_install_seam",
            RequestV1::Mock(MockV1::InstallSeam {
                id: "reset_db".into(),
                response: Some("canned".into()),
            }),
        ),
        (
            "mock_call",
            RequestV1::Mock(MockV1::Call {
                id: "seed_users".into(),
                scope: "smoke".into(),
            }),
        ),
        (
            "inspect_start",
            RequestV1::Inspect(InspectV1::StartRecording),
        ),
        // M8 (§13): the overlay/highlight half of the Inspect domain.
        (
            "inspect_enable_overlay",
            RequestV1::Inspect(InspectV1::EnableOverlay),
        ),
        (
            "inspect_disable_overlay",
            RequestV1::Inspect(InspectV1::DisableOverlay),
        ),
        (
            "inspect_highlight",
            RequestV1::Inspect(InspectV1::Highlight {
                node: Some(NodeKeyV1::AccessKit {
                    tree: [
                        0xa6, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        0x00, 0x00, 0x00, 0x01,
                    ],
                    node: 2,
                }),
            }),
        ),
        (
            "inspect_highlight_clear",
            RequestV1::Inspect(InspectV1::Highlight { node: None }),
        ),
        (
            "video_start",
            RequestV1::Video(VideoV1::Start(VideoOptsV1 {
                fps: 30,
                dir: Some("test-results/videos".into()),
            })),
        ),
        ("video_stop", RequestV1::Video(VideoV1::Stop)),
    ]
}

/// M6 (§16 v0.4) window + M7 (§9) gesture requests, in wire order.
fn multi_window_gesture_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        ("window_list", RequestV1::Window(WindowV1::List)),
        (
            "window_use",
            RequestV1::Window(WindowV1::Use {
                handle: "a6000000-0000-0000-0000-000000000001".into(),
            }),
        ),
        (
            "gesture_click",
            RequestV1::Gesture(GestureV1::Click {
                x: 12.5,
                y: 30.0,
                button: PointerButtonV1::Primary,
                count: 2,
            }),
        ),
        (
            "gesture_drag",
            RequestV1::Gesture(GestureV1::Drag {
                from_x: 10.0,
                from_y: 10.0,
                to_x: 110.0,
                to_y: 60.0,
                button: PointerButtonV1::Primary,
            }),
        ),
        (
            "gesture_key",
            RequestV1::Gesture(GestureV1::Key {
                key: "Enter".into(),
                modifiers: vec!["Shift".into()],
            }),
        ),
        ("bye", RequestV1::Bye),
    ]
}

/// (name, hex of the **framed** request — 4-byte LE length + postcard body).
fn request_vectors() -> Vec<(&'static str, String)> {
    let requests = core_requests()
        .into_iter()
        .chain(action_input_requests())
        .chain(service_requests())
        .chain(multi_window_gesture_requests());
    requests
        .map(|(name, req)| {
            let frame = encode_frame(&guiwright_proto::GwpRequest::V1(req)).expect("encode");
            (name, hex(&frame))
        })
        .collect()
}

/// Core session responses (§16 v0.4): welcome / pong / tree / resolve / action.
fn core_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![
        (
            "welcome",
            ResponseV1::Welcome {
                proto_version: PROTO_VERSION,
                session_id: "mock-4242".into(),
                tree_seq: 3,
                frame_seq: 17,
                adapter: AdapterInfoV1 {
                    name: "mock".into(),
                    framework: "fixture".into(),
                    framework_version: "0.1".into(),
                    caps: CapsV1 {
                        tree_update: true,
                        logical_snapshot: false,
                        input: true,
                        last_frame: false,
                        render_frame: false,
                        clock: true,
                    },
                },
            },
            serde_json::json!({
                "kind": "welcome",
                "proto_version": PROTO_VERSION,
                "session_id": "mock-4242",
                "tree_seq": 3,
                "frame_seq": 17,
                "adapter": {
                    "name": "mock",
                    "framework": "fixture",
                    "framework_version": "0.1",
                    "caps": {
                        "tree_update": true,
                        "logical_snapshot": false,
                        "input": true,
                        "last_frame": false,
                        "render_frame": false,
                        "clock": true,
                    },
                },
            }),
        ),
        (
            "pong",
            ResponseV1::Pong,
            serde_json::json!({"kind": "pong"}),
        ),
        (
            "tree_snapshot",
            ResponseV1::TreeSnapshot {
                tree_seq: 3,
                node_count: 8,
            },
            serde_json::json!({"kind": "tree_snapshot", "tree_seq": 3, "node_count": 8}),
        ),
        (
            "resolved",
            ResponseV1::Resolved {
                node: NodeKeyV1::AccessKit {
                    tree: [0x01; 16],
                    node: 2,
                },
                generation: 0,
                rect: Some([0.0, 0.0, 120.0, 32.0]),
            },
            serde_json::json!({
                "kind": "resolved",
                "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 2},
                "generation": 0,
                "rect": [0.0, 0.0, 120.0, 32.0],
            }),
        ),
        (
            "action_done",
            ResponseV1::ActionDone {
                frame_seq: 18,
                detail: "clicked".into(),
            },
            serde_json::json!({"kind": "action_done", "frame_seq": 18, "detail": "clicked"}),
        ),
    ]
}

/// Clock / mock / codegen / error service responses.
fn service_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![
        (
            "input_queued",
            ResponseV1::InputQueued { frame_seq: 19 },
            serde_json::json!({"kind": "input_queued", "frame_seq": 19}),
        ),
        (
            "clock_state",
            ResponseV1::ClockState {
                virtual_time_ms: Some(30_000),
                paused: false,
                steps: 60,
            },
            serde_json::json!({
                "kind": "clock_state",
                "virtual_time_ms": 30_000,
                "paused": false,
                "steps": 60,
            }),
        ),
        (
            "mock_done",
            ResponseV1::MockDone {
                id: "reset_db".into(),
                detail: "state reset to seed-42".into(),
            },
            serde_json::json!({"kind": "mock_done", "id": "reset_db", "detail": "state reset to seed-42"}),
        ),
        (
            "codegen",
            ResponseV1::Codegen {
                code: "app.get_by_test_id(\"save\").click()".into(),
                warnings: vec!["ambiguous text".into()],
            },
            serde_json::json!({
                "kind": "codegen",
                "code": "app.get_by_test_id(\"save\").click()",
                "warnings": ["ambiguous text"],
            }),
        ),
        (
            "error",
            ResponseV1::Error {
                code: ErrorCode::StrictModeViolation,
                message: "strict mode violation".into(),
            },
            serde_json::json!({
                "kind": "error",
                "code": ErrorCode::StrictModeViolation as u32,
                "message": "strict mode violation",
            }),
        ),
        ("bye", ResponseV1::Bye, serde_json::json!({"kind": "bye"})),
    ]
}

/// Video / window (M6) / gesture (M7) responses, in wire order.
fn media_window_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![
        (
            "video_file",
            ResponseV1::VideoFile {
                path: "test-results/videos/guiwright-smoke.webm".into(),
                frames: 42,
                dropped: 7,
                duration_ms: 1680,
            },
            serde_json::json!({
                "kind": "video_file",
                "path": "test-results/videos/guiwright-smoke.webm",
                "frames": 42,
                "dropped": 7,
                "duration_ms": 1680,
            }),
        ),
        // M6 (§16 v0.4): the window replies append after VideoFile.
        (
            "windows",
            ResponseV1::Windows {
                windows: vec![
                    WindowInfoV1 {
                        handle: "a6000000-0000-0000-0000-000000000001".into(),
                        title: "Editor".into(),
                        focused: true,
                    },
                    WindowInfoV1 {
                        handle: "a6000000-0000-0000-0000-000000000002".into(),
                        title: "Preview".into(),
                        focused: false,
                    },
                ],
            },
            serde_json::json!({
                "kind": "windows",
                "windows": [
                    {"handle": "a6000000-0000-0000-0000-000000000001", "title": "Editor", "focused": true},
                    {"handle": "a6000000-0000-0000-0000-000000000002", "title": "Preview", "focused": false},
                ],
            }),
        ),
        (
            "window_used",
            ResponseV1::WindowUsed {
                handle: "a6000000-0000-0000-0000-000000000001".into(),
            },
            serde_json::json!({
                "kind": "window_used",
                "handle": "a6000000-0000-0000-0000-000000000001",
            }),
        ),
        // M7 (§9): the gesture reply appends after WindowUsed.
        (
            "gesture_done",
            ResponseV1::GestureDone {
                frame_seq: 20,
                detail: "clicked at (12.5, 30) — 3 events".into(),
            },
            serde_json::json!({
                "kind": "gesture_done",
                "frame_seq": 20,
                "detail": "clicked at (12.5, 30) — 3 events",
            }),
        ),
    ]
}

/// (name, hex of the **framed** response, expected decoded JSON the
/// TypeScript decoder must produce).
fn response_vectors() -> Vec<(&'static str, String, serde_json::Value)> {
    let responses = core_responses()
        .into_iter()
        .chain(service_responses())
        .chain(media_window_responses());
    responses
        .map(|(name, resp, decoded)| {
            let frame = encode_frame(&guiwright_proto::GwpResponse::V1(resp)).expect("encode");
            (name, hex(&frame), decoded)
        })
        .collect()
}

#[test]
fn write_golden_vectors() {
    let requests: serde_json::Value = serde_json::Value::Array(
        request_vectors()
            .into_iter()
            .map(|(name, frame_hex)| serde_json::json!({ "name": name, "frame_hex": frame_hex }))
            .collect(),
    );
    let responses: serde_json::Value = serde_json::Value::Array(
        response_vectors()
            .into_iter()
            .map(|(name, frame_hex, decoded)| {
                serde_json::json!({ "name": name, "frame_hex": frame_hex, "decoded": decoded })
            })
            .collect(),
    );
    let doc = serde_json::json!({
        "schema": "gwp-golden/v1",
        "generator": "guiwright-proto tests/golden_vectors.rs (bless: GUIWRIGHT_BLESS=1)",
        "requests": requests,
        "responses": responses,
    });
    let rendered = serde_json::to_string_pretty(&doc).expect("pretty") + "\n";

    if std::env::var("GUIWRIGHT_BLESS").ok().as_deref() == Some("1") {
        std::fs::write(GOLDEN_PATH, rendered).expect("write golden");
        eprintln!("blessed: {GOLDEN_PATH}");
        return;
    }

    let committed = std::fs::read_to_string(GOLDEN_PATH).unwrap_or_else(|e| {
        panic!("golden missing at {GOLDEN_PATH} (run with GUIWRIGHT_BLESS=1): {e}")
    });
    assert_eq!(
        rendered, committed,
        "golden vectors drifted; if intentional, re-run with GUIWRIGHT_BLESS=1"
    );
}
