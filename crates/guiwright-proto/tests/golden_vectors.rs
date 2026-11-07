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
    AdapterInfoV1, CapsV1, ClockV1, ErrorCode, InputEventV1, InspectV1, MockV1, NodeKeyV1,
    PROTO_VERSION, PointerButtonV1, RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1,
    VideoOptsV1, VideoV1,
};

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../clients/typescript/test/golden.json"
);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// (name, hex of the **framed** request — 4-byte LE length + postcard body).
fn request_vectors() -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut push = |name: &'static str, req: guiwright_proto::schema::RequestV1| {
        let frame = encode_frame(&guiwright_proto::GwpRequest::V1(req)).expect("encode");
        out.push((name, hex(&frame)));
    };

    push(
        "hello",
        RequestV1::Hello {
            proto_version: PROTO_VERSION,
            agent: "@guiwright/gwp/0.3".into(),
        },
    );
    push("ping", RequestV1::Ping);
    push("tree_snapshot", RequestV1::TreeSnapshot);
    push(
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
    );
    push(
        "resolve_test_id",
        RequestV1::Resolve {
            selector: SelectorV1::TestId("btn-save".into()),
            timeout_ms: 0,
        },
    );
    push(
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
    );
    push(
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
    );
    push(
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
    );
    push(
        "clock_fast_forward",
        RequestV1::Clock(ClockV1::FastForward { ms: 30_000 }),
    );
    push("clock_state", RequestV1::Clock(ClockV1::State));
    push(
        "mock_install_seam",
        RequestV1::Mock(MockV1::InstallSeam {
            id: "reset_db".into(),
            response: Some("canned".into()),
        }),
    );
    push(
        "mock_call",
        RequestV1::Mock(MockV1::Call {
            id: "seed_users".into(),
            scope: "smoke".into(),
        }),
    );
    push(
        "inspect_start",
        RequestV1::Inspect(InspectV1::StartRecording),
    );
    push(
        "video_start",
        RequestV1::Video(VideoV1::Start(VideoOptsV1 {
            fps: 30,
            dir: Some("test-results/videos".into()),
        })),
    );
    push("video_stop", RequestV1::Video(VideoV1::Stop));
    push("bye", RequestV1::Bye);
    out
}

/// (name, hex of the **framed** response, expected decoded JSON the
/// TypeScript decoder must produce).
fn response_vectors() -> Vec<(&'static str, String, serde_json::Value)> {
    let mut out: Vec<(&'static str, String, serde_json::Value)> = Vec::new();
    let mut push = |name: &'static str, resp: ResponseV1, decoded: serde_json::Value| {
        let frame = encode_frame(&guiwright_proto::GwpResponse::V1(resp)).expect("encode");
        out.push((name, hex(&frame), decoded));
    };

    push(
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
    );
    push(
        "pong",
        ResponseV1::Pong,
        serde_json::json!({"kind": "pong"}),
    );
    push(
        "tree_snapshot",
        ResponseV1::TreeSnapshot {
            tree_seq: 3,
            node_count: 8,
        },
        serde_json::json!({"kind": "tree_snapshot", "tree_seq": 3, "node_count": 8}),
    );
    push(
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
    );
    push(
        "action_done",
        ResponseV1::ActionDone {
            frame_seq: 18,
            detail: "clicked".into(),
        },
        serde_json::json!({"kind": "action_done", "frame_seq": 18, "detail": "clicked"}),
    );
    push(
        "input_queued",
        ResponseV1::InputQueued { frame_seq: 19 },
        serde_json::json!({"kind": "input_queued", "frame_seq": 19}),
    );
    push(
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
    );
    push(
        "mock_done",
        ResponseV1::MockDone {
            id: "reset_db".into(),
            detail: "state reset to seed-42".into(),
        },
        serde_json::json!({"kind": "mock_done", "id": "reset_db", "detail": "state reset to seed-42"}),
    );
    push(
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
    );
    push(
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
    );
    push("bye", ResponseV1::Bye, serde_json::json!({"kind": "bye"}));
    push(
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
    );
    out
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
