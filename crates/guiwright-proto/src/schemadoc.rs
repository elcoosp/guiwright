//! Published GWP schema (§16 "Other-language clients — GWP over the wire;
//! schema-published", M3): a machine-readable JSON description of the V1
//! wire contract — framing, codec rules, transport list, and the
//! **append-only variant tables** other-language clients hard-code.
//!
//! [`gwp_schema_json`] is the single source of truth. The golden test
//! (`tests/schema_golden.rs`) pins the emitted document byte-for-byte
//! against `tests/schema-golden.json` (bless: `GUIWRIGHT_BLESS=1`), so a
//! variant reorder — not just an append — fails the build, exactly like
//! `wire_discriminants_are_append_only`. The committed copy published to
//! `clients/typescript/gwp-schema.json` is regenerated from this function.

use crate::schema::{MAX_BLOB_BYTES, MAX_FRAME_BYTES, PROTO_VERSION, VIDEO_DEFAULT_FPS};

/// A wire variant: its postcard discriminant (the varint variant index)
/// and a human-readable payload shape.
struct Variant {
    index: u64,
    name: &'static str,
    payload: &'static str,
}

const fn v(index: u64, name: &'static str, payload: &'static str) -> Variant {
    Variant {
        index,
        name,
        payload,
    }
}

/// The request table, in wire order (§7). Append-only: `Video` follows
/// `Bye` (M3), `Window` follows `Video` (M6, §16 v0.4 multi-window) and
/// `Gesture` follows `Window` (M7, §9 input pipeline); the pinning tests
/// enforce the order.
const REQUESTS: &[Variant] = &[
    v(0, "Hello", "{ proto_version: u32 varint, agent: string }"),
    v(1, "Ping", "—"),
    v(2, "TreeSnapshot", "—"),
    v(
        3,
        "Resolve",
        "{ selector: SelectorV1, timeout_ms: u64 varint }",
    ),
    v(
        4,
        "Action",
        "{ node: NodeKeyV1, generation: u32 varint, action: SemActionV1, force: bool, timeout_ms: u64 varint }",
    ),
    v(5, "Input", "{ events: seq<InputEventV1> } — injected into the sending connection's active window target (M7) → InputQueued"),
    v(6, "Clock", "ClockV1 (see domains.clock)"),
    v(7, "Mock", "MockV1 (see domains.mock)"),
    v(8, "Inspect", "InspectV1 (see domains.inspect)"),
    v(9, "Bye", "—"),
    v(10, "Video", "VideoV1 (see domains.video)"),
    v(11, "Window", "WindowV1 (see domains.window)"),
    v(12, "Gesture", "GestureV1 (see domains.gesture)"),
];

/// The response table, in wire order. Append-only: `VideoFile` follows
/// `Bye` (M3), the M6 window replies append at 12–13 and the M7
/// `GestureDone` reply at 14.
const RESPONSES: &[Variant] = &[
    v(
        0,
        "Welcome",
        "{ proto_version: u32 varint, session_id: string, tree_seq: u64 varint, frame_seq: u64 varint, adapter: AdapterInfoV1 }",
    ),
    v(1, "Pong", "—"),
    v(
        2,
        "TreeSnapshot",
        "{ tree_seq: u64 varint, node_count: u64 varint }",
    ),
    v(
        3,
        "Resolved",
        "{ node: NodeKeyV1, generation: u32 varint, rect: option<[f64; 4]> }",
    ),
    v(4, "ActionDone", "{ frame_seq: u64 varint, detail: string }"),
    v(5, "InputQueued", "{ frame_seq: u64 varint }"),
    v(
        6,
        "ClockState",
        "{ virtual_time_ms: option<u64 varint>, paused: bool, steps: u64 varint }",
    ),
    v(7, "MockDone", "{ id: string, detail: string }"),
    v(8, "Codegen", "{ code: string, warnings: seq<string> }"),
    v(9, "Error", "{ code: ErrorCode (u8), message: string }"),
    v(10, "Bye", "—"),
    v(
        11,
        "VideoFile",
        "{ path: string, frames: u64 varint, dropped: u64 varint, duration_ms: u64 varint }",
    ),
    v(12, "Windows", "{ windows: seq<WindowInfoV1> }"),
    v(13, "WindowUsed", "{ handle: string }"),
    v(
        14,
        "GestureDone",
        "{ frame_seq: u64 varint, detail: string }",
    ),
];

/// Server-push events (multiplexed by `seq` per topic, §7).
const EVENTS: &[Variant] = &[
    v(0, "TreeChanged", "{ seq: u64 varint }"),
    v(1, "FramePresented", "{ seq: u64 varint, hash: u64 varint }"),
    v(2, "Log", "{ record: string }"),
];

fn variants(table: &[Variant]) -> serde_json::Value {
    serde_json::Value::Array(
        table
            .iter()
            .map(|vr| {
                serde_json::json!({
                    "index": vr.index,
                    "name": vr.name,
                    "payload": vr.payload,
                })
            })
            .collect(),
    )
}

/// Domain enums, in wire order (nested one level under their request).
fn domain_table(name: &str, rows: &[(u64, &str, &str)]) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "variants": rows.iter().map(|(i, n, p)| serde_json::json!({
            "index": i, "name": n, "payload": p,
        })).collect::<Vec<_>>(),
    })
}

/// Emit the published GWP schema document (`gwp-schema/v1`).
///
/// The document is hand-authored (not derived via a reflection macro) on
/// purpose: it is the reviewed contract other-language clients implement,
/// and the golden test pins it against silent drift. Every variant table
/// mirrors the Rust enums in [`crate::schema`] — the discriminant-pinning
/// test there and the golden test here fail the build together if the
/// two ever disagree.
pub fn gwp_schema_json() -> serde_json::Value {
    // The table-vs-enum consistency proof lives in the golden test
    // (`tests/schema_golden.rs`): it asserts the Rust enums' variant
    // counts against these tables AND pins the emitted document
    // byte-for-byte, so any drift (reorder, rename, new variant) fails
    // the build together with `wire_discriminants_are_append_only`.

    serde_json::json!({
        "schema": "gwp-schema/v1",
        "protocol_version": PROTO_VERSION,
        "title": "Guiwright Protocol (GWP) wire schema",
        "description": "Binary-first CDP-style protocol: sessions -> targets (windows) -> domains; request/response + server-push events. This document is the contract for non-Rust clients (§16).",
        "framing": {
            "prefix": "4-byte little-endian u32 body length",
            "body": "postcard-encoded payload",
            "max_frame_bytes": MAX_FRAME_BYTES,
            "envelope": {
                "requests": "GwpRequest { V1 = 0 } (one variant)",
                "responses": "GwpResponse { V1 = 0 } (one variant)",
                "events": "GwpEvent { V1 = 0 } (one variant)"
            }
        },
        "blob_side_channel": {
            "encoding": "lz4_flex::compress_prepend_size over postcard bytes",
            "decode": "lz4_flex::decompress_size_prepended",
            "max_decompressed_bytes": MAX_BLOB_BYTES
        },
        "transports": [
            { "name": "uds", "role": "default (same-host apps)", "server": "GwpServerHandle::spawn (uds://path)" },
            { "name": "tcp", "role": "CI containers + other-language clients (M3)", "server": "GwpServerHandle::spawn_tcp (host:port)" }
        ],
        "codec": {
            "name": "postcard",
            "rules": [
                "enum variants are the varint (LEB128, u32) of the variant index — tables below are the pinning contract",
                "unsigned ints (u32/u64/usize) are LEB128 varints",
                "signed ints (i16/i32/i64) are zigzag-varints (none appear in V1 payloads)",
                "f32/f64 are 4/8 raw little-endian bytes",
                "bool is one byte (0x00/0x01)",
                "option<T> is a 0x00/0x01 tag byte followed by T when present",
                "string and seq<T> are a varint length prefix followed by the items",
                "structs encode fields in declaration order"
            ]
        },
        "conversation": [
            "client connects, sends Hello { proto_version, agent } as the FIRST frame",
            "driver replies Welcome { proto_version, session_id, tree_seq, frame_seq, adapter } or Error { UnsupportedVersion }",
            "any other first frame is rejected with Error { NotHandshaked }",
            "each subsequent request is answered by exactly one response frame; queued events may be flushed between responses",
            "Bye is answered by Bye and closes the connection"
        ],
        "requests": variants(REQUESTS),
        "responses": variants(RESPONSES),
        "events": variants(EVENTS),
        "error_codes": ["UnsupportedVersion", "NotHandshaked", "UnknownNode", "StaleGeneration", "StrictModeViolation", "Timeout", "Unsupported", "Internal", "UnknownWindow"],
        "domains": {
            "clock": domain_table("ClockV1", &[
                (0, "InstallVirtual", "—"),
                (1, "FastForward", "{ ms: u64 varint }"),
                (2, "RunFor", "{ ms: u64 varint }"),
                (3, "Pause", "—"),
                (4, "Resume", "—"),
                (5, "SetTime", "{ ms: u64 varint }"),
                (6, "State", "—"),
            ]),
            "mock": domain_table("MockV1", &[
                (0, "InstallSeam", "{ id: string, response: option<string> }"),
                (1, "Clear", "{ id: string }"),
                (2, "ResetState", "{ scope: string }"),
                (3, "Call", "{ id: string, scope: string }"),
            ]),
            "inspect": domain_table("InspectV1", &[
                (0, "StartRecording", "—"),
                (1, "StopRecording", "—"),
            ]),
            "video": domain_table("VideoV1", &[
                (0, "Start", "{ opts: VideoOptsV1 { fps: u32 varint (0 = driver default), dir: option<string> (client-owned artifact dir) } }"),
                (1, "Stop", "—"),
            ]),
            "window": domain_table("WindowV1", &[
                (0, "List", "— → Windows"),
                (1, "Use", "{ handle: string (WindowInfoV1.handle) } → WindowUsed | Error { UnknownWindow }; aims the CALLING connection at the target — subsequent Resolves only see that window's subtree (per-connection state, §16 v0.4)"),
            ]),
            "gesture": domain_table("GestureV1", &[
                (0, "Click", "{ x: f64, y: f64, button: PointerButtonV1, count: u32 varint (Playwright clickCount: 1 = click, 2 = double-click) } → GestureDone; synthesized as move→down→up ×count (§9)"),
                (1, "Drag", "{ from_x: f64, from_y: f64, to_x: f64, to_y: f64, button: PointerButtonV1 } → GestureDone; down→ratchet-interpolated moves (adapter pacing, ~5 px/tick)→up (§9)"),
                (2, "Wheel", "{ dx: f64, dy: f64 } → GestureDone; logical-px deltas"),
                (3, "Key", "{ key: string, modifiers: seq<string> } → GestureDone; synthesized as down→up (§8.4 key-name vocabulary)"),
                (4, "Type", "{ text: string } → GestureDone; one TextInput commit — adapters translate to IME or per-key events (§4.2)"),
            ]),
        },
        "window_info": {
            "name": "WindowInfoV1",
            "fields": "{ handle: string (stable target id: the AccessKit subtree TreeId for multi-window adapters, 'main' for single-window), title: string, focused: bool }",
        },
        "video_default_fps": VIDEO_DEFAULT_FPS,
    })
}
