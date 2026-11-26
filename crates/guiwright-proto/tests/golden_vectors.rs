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
//! Under cargo-nextest the bless write is refused (the fixed repo path
//! would be raced by the per-test process fan-out); bless via plain
//! `cargo test`.
//!
//! The TypeScript side re-pins the vendor copy in
//! `test/codec.test.ts`; the two fail the build together on drift.

mod common;

use guiwright_proto::framing::encode_frame;
use guiwright_proto::schema::{
    AdapterInfoV1, CapsV1, CheckedStateV1, ClockV1, DescribeAllV1, DescribeV1,
    DescribedIdentityStateV1, DescribedIdentityV1, DescribedV1, ErrorCode, EventKindV1, EventV1,
    GestureV1, InputEventV1, InspectV1, MockV1, NodeKeyV1, NthV1, PROTO_VERSION, PointerButtonV1,
    RequestV1, ResolvedMatchV1, ResponseV1, ResumeCursorV1, ScreenshotV1, SelectorV1, SemActionV1,
    SnapshotNodeV1, SnapshotV1, SubscribeV1, TextMatchV1, TraceOptsV1, TraceV1, VideoOptsV1,
    VideoV1, WindowInfoV1, WindowV1,
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
        // The m89 select vocabulary (the m19 record's named §16
        // follow-on): the option rides by accessible name — the
        // appended variant's bytes pin the published table.
        (
            "action_select_option",
            RequestV1::Action {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 4,
                },
                generation: 0,
                action: SemActionV1::SelectOption {
                    option: "France".into(),
                },
                force: false,
                timeout_ms: 5000,
            },
        ),
        // The m90 scroll-into-view vocabulary (the m19 record's named
        // §16 follow-on): the appended variant's bytes pin the table.
        (
            "action_scroll_into_view",
            RequestV1::Action {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 2,
                },
                generation: 0,
                action: SemActionV1::ScrollIntoView,
                force: false,
                timeout_ms: 5000,
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

/// M6 (§16 v0.4) window + M7 (§9) gesture + M10 (§7 render) requests, in
/// wire order.
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
        // M10 (§7 render domain): the window + element screenshot requests.
        (
            "screenshot_window",
            RequestV1::Screenshot(ScreenshotV1::Window),
        ),
        (
            "screenshot_element",
            RequestV1::Screenshot(ScreenshotV1::Element {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                timeout_ms: 5000,
            }),
        ),
        // M11 (§7 tree domain): the full-tree dump request.
        ("snapshot_full", RequestV1::Snapshot(SnapshotV1::Full)),
        // M13 (§7 tree domain): the filtered-dump arm appends after Full.
        (
            "snapshot_filtered",
            RequestV1::Snapshot(SnapshotV1::Filtered {
                selector: SelectorV1::Role {
                    role: "Group".into(),
                    name: None,
                },
                timeout_ms: 2_500,
            }),
        ),
        // M12 (§7 event channel): the subscription domain appends after Snapshot.
        (
            "subscribe_on",
            RequestV1::Subscribe(SubscribeV1::On {
                kinds: vec![EventKindV1::TreeChanged, EventKindV1::FramePresented],
            }),
        ),
        ("subscribe_off", RequestV1::Subscribe(SubscribeV1::Off)),
        // The resume vocabulary (the m18 record's named §7 follow-on):
        // the reconnect cursor rides the appended `Resume` variant — the
        // bytes pin the published table beside the M12 pair.
        (
            "subscribe_resume",
            RequestV1::Subscribe(SubscribeV1::Resume {
                kinds: vec![EventKindV1::TreeChanged],
                cursor: ResumeCursorV1 {
                    tree_seq: 12,
                    frame_seq: 34,
                },
            }),
        ),
        ("bye", RequestV1::Bye),
    ]
}

/// M30 (§8.2's set read): the resolve-all request — the enumeration over
/// the wire, no timeout (it never waits).
fn resolve_all_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "resolve_all_role",
        RequestV1::ResolveAll {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: None,
            },
        },
    )]
}

/// The `has` filter (the m34 record's named follow-on): the
/// selector-valued positive filter rides its own appended variant — the
/// `Filter` row's encodings stay byte-pinned, the widening is its own
/// variant (the M64 rule).
fn has_filter_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "resolve_has",
        RequestV1::Resolve {
            selector: SelectorV1::Has {
                source: Box::new(SelectorV1::Role {
                    role: "Group".into(),
                    name: None,
                }),
                has: Box::new(SelectorV1::Role {
                    role: "Button".into(),
                    name: None,
                }),
            },
            timeout_ms: 5000,
        },
    )]
}

/// The m34 filter row's byte pin (the m91 TS helper composes exactly
/// this shape): one `Resolve` carrying the full `Filter` payload —
/// `has_text`, `has_not` and the `nth` pick in one row. The `Filter`
/// encodings predate the goldens' selector pins; this vector pins them
/// now that a typed client surface speaks them.
fn filter_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "resolve_filter",
        RequestV1::Resolve {
            selector: SelectorV1::Filter {
                source: Box::new(SelectorV1::Role {
                    role: "Button".into(),
                    name: None,
                }),
                has_text: Some(TextMatchV1 {
                    text: "Save".into(),
                    exact: false,
                    ignore_case: true,
                }),
                has_not: Some(Box::new(SelectorV1::Role {
                    role: "TextInput".into(),
                    name: None,
                })),
                nth: Some(NthV1::First),
            },
            timeout_ms: 5000,
        },
    )]
}

/// M32 (the hit-test read): the point request — physical px in the
/// tree's bounds space, no timeout (the read is total, it never waits).
fn hit_test_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "hit_test_save_center",
        RequestV1::HitTest { x: 76.0, y: 32.0 },
    )]
}

/// M36 (the node-describe read): the handle request — the dump-row form
/// a resolve/hit row mints (the same tree/node vocabulary the Action
/// request takes) plus the generation it was minted at (the Action
/// boundary's guard).
fn describe_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "describe_save_handle",
        RequestV1::Describe(DescribeV1 {
            node: NodeKeyV1::AccessKit {
                tree: [0x01; 16],
                node: 2,
            },
            generation: 0,
        }),
    )]
}

/// M39 (the describe-all read): the set read carrying facts — the
/// `ResolveAll` request's exact shape (the m39 record's payload reuse).
fn describe_all_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "describe_all_buttons",
        RequestV1::DescribeAll(DescribeAllV1 {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: None,
            },
        }),
    )]
}

/// M64 (the describe-all identity read): the same payload — the
/// description fact's widening rides its own appended domain (the M39
/// encodings stay byte-pinned).
fn describe_all_identity_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "describe_all_identity_buttons",
        RequestV1::DescribeAllIdentity(DescribeAllV1 {
            selector: SelectorV1::Role {
                role: "Button".into(),
                name: None,
            },
        }),
    )]
}

/// M66 (the single-node identity describe): the M36 handle pair in —
/// the same payload [`RequestV1::Describe`] carries — the description
/// fact's widening at handle width (the m64 record's named standing
/// gap), one round trip instead of the whole-set walk.
fn describe_identity_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "describe_identity_save_handle",
        RequestV1::DescribeIdentity(DescribeV1 {
            node: NodeKeyV1::AccessKit {
                tree: [0x01; 16],
                node: 2,
            },
            generation: 0,
        }),
    )]
}

/// M68 (the identity read's state facts at handle width): the m67
/// record's named standing gap — the state three riding beside the
/// identity row. The request reuses the M36 handle pair verbatim (the
/// m66 payload-reuse rule, one more domain), so its frame differs from
/// `describe_identity_save_handle` only in the variant byte.
fn describe_identity_state_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "describe_identity_state_save_handle",
        RequestV1::DescribeIdentityState(DescribeV1 {
            node: NodeKeyV1::AccessKit {
                tree: [0x01; 16],
                node: 2,
            },
            generation: 0,
        }),
    )]
}

/// M70 (the identity read's state facts at set width): the m68/m69
/// records' named standing gap — the state three riding the whole match
/// set. The request reuses the M39/m64 `DescribeAllV1` walk payload (the
/// m64 payload-reuse rule at set width), so its frame differs from
/// `describe_all_identity_buttons` only in the variant byte.
fn describe_all_identity_state_requests() -> Vec<(&'static str, RequestV1)> {
    vec![(
        "describe_all_identity_state_checkboxes",
        RequestV1::DescribeAllIdentityState(DescribeAllV1 {
            selector: SelectorV1::Role {
                role: "CheckBox".into(),
                name: None,
            },
        }),
    )]
}

/// (name, hex of the **framed** request — 4-byte LE length + postcard body).
/// The trace domain's byte pins (M97, §7 "Domain: trace & inspector" — the
/// archive-control Start/Stop arms the M18 Attach arm completes): the
/// client-owned opts row (test label + artifact dir) and the unit stop
/// row. The TS client builds the same shapes through
/// `traceStart()`/`traceStop()` — the two clients fail the build together
/// on drift.
fn trace_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        (
            "trace_start",
            RequestV1::Trace(TraceV1::Start(TraceOptsV1 {
                test: "e2e_trace".into(),
                dir: Some("/tmp/gw-trace".into()),
            })),
        ),
        ("trace_stop", RequestV1::Trace(TraceV1::Stop)),
    ]
}

/// (name, hex of the **framed** request — 4-byte LE length + postcard body).
/// The regex text probes' byte pins (the m34 standing note cashed — the
/// m93/m94 honest lists' first item): the five appended `SelectorV1`
/// rows, the pattern source riding as the payload. The string arms'
/// encodings are untouched (the M64 rule at selector width); these
/// vectors pin the rows the TS codec table mirrors.
fn regex_text_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        (
            "resolve_text_regex",
            RequestV1::Resolve {
                selector: SelectorV1::TextRegex {
                    pattern: "(?i)save|cancel".into(),
                },
                timeout_ms: 5000,
            },
        ),
        (
            "resolve_label_regex",
            RequestV1::Resolve {
                selector: SelectorV1::LabelRegex {
                    pattern: "^Wel.* Ada$".into(),
                },
                timeout_ms: 0,
            },
        ),
        (
            "resolve_value_regex",
            RequestV1::Resolve {
                selector: SelectorV1::ValueRegex {
                    pattern: r"^\S+@\S+\.\S+$".into(),
                },
                timeout_ms: 0,
            },
        ),
        (
            "resolve_role_name_regex",
            RequestV1::Resolve {
                selector: SelectorV1::RoleNameRegex {
                    role: "Button".into(),
                    pattern: "^(Save|Cancel)$".into(),
                },
                timeout_ms: 5000,
            },
        ),
        (
            "resolve_has_text_regex",
            RequestV1::Resolve {
                selector: SelectorV1::HasTextRegex {
                    source: Box::new(SelectorV1::Role {
                        role: "Button".into(),
                        name: None,
                    }),
                    pattern: "^Save$".into(),
                },
                timeout_ms: 0,
            },
        ),
    ]
}

/// The `getBy*` locator factories' string-row pins (the m95 record's
/// named TS pass — the kit's `get_by_*` sugars mirrored client-side):
/// the byte-pinned string rows the factories compose, the shapes no
/// earlier vector carried (the fuzzy role row rides
/// `resolve_role_named`, the test-id row `resolve_test_id`, the regex
/// rows their m95 pins). The TS factory test builds the same shapes
/// through `getByRole`/`getByText`/`getByLabel`/`getByValue` — the two
/// clients fail the build together on drift.
fn getby_requests() -> Vec<(&'static str, RequestV1)> {
    vec![
        (
            "resolve_role_unnamed",
            RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: None,
                },
                timeout_ms: 0,
            },
        ),
        (
            "resolve_role_named_exact",
            RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: true,
                        ignore_case: false,
                    }),
                },
                timeout_ms: 5000,
            },
        ),
        (
            "resolve_text",
            RequestV1::Resolve {
                selector: SelectorV1::Text(TextMatchV1 {
                    text: "Save".into(),
                    exact: false,
                    ignore_case: true,
                }),
                timeout_ms: 5000,
            },
        ),
        (
            "resolve_label",
            RequestV1::Resolve {
                selector: SelectorV1::Label(TextMatchV1 {
                    text: "Welcome, Ada".into(),
                    exact: false,
                    ignore_case: true,
                }),
                timeout_ms: 5000,
            },
        ),
        (
            "resolve_value",
            RequestV1::Resolve {
                selector: SelectorV1::Value(TextMatchV1 {
                    text: "ada@example.dev".into(),
                    exact: false,
                    ignore_case: true,
                }),
                timeout_ms: 5000,
            },
        ),
    ]
}

fn request_vectors() -> Vec<(&'static str, String)> {
    let requests = core_requests()
        .into_iter()
        .chain(action_input_requests())
        .chain(service_requests())
        .chain(multi_window_gesture_requests())
        .chain(resolve_all_requests())
        .chain(has_filter_requests())
        .chain(regex_text_requests())
        .chain(getby_requests())
        .chain(filter_requests())
        .chain(hit_test_requests())
        .chain(describe_requests())
        .chain(describe_all_requests())
        .chain(describe_all_identity_requests())
        .chain(describe_identity_requests())
        .chain(describe_identity_state_requests())
        .chain(describe_all_identity_state_requests())
        .chain(trace_requests());
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

/// M30 (§8.2's set read): the resolve-all reply — two rows, the same
/// per-match facts the single `Resolved` reply carries.
fn resolve_all_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "resolved_all",
        ResponseV1::ResolvedAll {
            matches: vec![
                ResolvedMatchV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 2,
                    },
                    generation: 0,
                    rect: Some([0.0, 0.0, 120.0, 32.0]),
                },
                ResolvedMatchV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 3,
                    },
                    generation: 0,
                    rect: Some([0.0, 40.0, 120.0, 72.0]),
                },
            ],
        },
        serde_json::json!({
            "kind": "resolved_all",
            "matches": [
                {"node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 2}, "generation": 0, "rect": [0.0, 0.0, 120.0, 32.0]},
                {"node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 3}, "generation": 0, "rect": [0.0, 40.0, 120.0, 72.0]},
            ],
        }),
    )]
}

/// M32 (the hit-test read): the hit reply — one row on a hit (the same
/// per-match facts the resolve-all rows carry), `None` on a miss.
fn hit_test_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![
        (
            "hit_test_hit",
            ResponseV1::HitTest {
                hit: Some(ResolvedMatchV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 2,
                    },
                    generation: 0,
                    rect: Some([16.0, 16.0, 136.0, 48.0]),
                }),
            },
            serde_json::json!({
                "kind": "hit_test",
                "hit": {"node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 2}, "generation": 0, "rect": [16.0, 16.0, 136.0, 48.0]},
            }),
        ),
        (
            "hit_test_miss",
            ResponseV1::HitTest { hit: None },
            serde_json::json!({
                "kind": "hit_test",
                "hit": null,
            }),
        ),
    ]
}

/// M36 (the node-describe read): the identity-facts reply — the dump
/// row's facts, one node instead of a walk; the handle echoes the
/// request's (the dump-row rule).
fn describe_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "described_save_button",
        ResponseV1::Described(DescribedV1 {
            node: NodeKeyV1::AccessKit {
                tree: [0x01; 16],
                node: 2,
            },
            generation: 0,
            role: "Button".into(),
            name: Some("Save".into()),
            value: None,
            bounds: Some([16.0, 16.0, 136.0, 48.0]),
        }),
        serde_json::json!({
            "kind": "described",
            "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 2},
            "generation": 0,
            "role": "Button",
            "name": "Save",
            "value": null,
            "bounds": [16.0, 16.0, 136.0, 48.0],
        }),
    )]
}

/// M39 (the describe-all read): the set read carrying facts — the
/// resolve-all walk's rows plus the Described identity facts (the
/// m37 record's standing gap, closed).
fn describe_all_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "described_all_buttons",
        ResponseV1::DescribedAll {
            rows: vec![
                DescribedV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 2,
                    },
                    generation: 0,
                    role: "Button".into(),
                    name: Some("Save".into()),
                    value: None,
                    bounds: Some([16.0, 16.0, 136.0, 48.0]),
                },
                DescribedV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 3,
                    },
                    generation: 0,
                    role: "Button".into(),
                    name: Some("Cancel".into()),
                    value: None,
                    bounds: Some([16.0, 56.0, 136.0, 88.0]),
                },
            ],
        },
        serde_json::json!({
            "kind": "described_all",
            "rows": [
                {
                    "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 2},
                    "generation": 0,
                    "role": "Button",
                    "name": "Save",
                    "value": null,
                    "bounds": [16.0, 16.0, 136.0, 48.0],
                },
                {
                    "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 3},
                    "generation": 0,
                    "role": "Button",
                    "name": "Cancel",
                    "value": null,
                    "bounds": [16.0, 56.0, 136.0, 88.0],
                },
            ],
        }),
    )]
}

/// M66 (the single-node identity describe): the M64 identity row at
/// handle width — the identity four plus the description fact, the
/// request's handle echoed (the dump-row rule). The M36
/// `described_save_button` reply keeps its byte-exact encoding; the
/// widening rides its own appended reply variant.
fn describe_identity_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "described_identity_install_button",
        ResponseV1::DescribedIdentity(DescribedIdentityV1 {
            node: NodeKeyV1::AccessKit {
                tree: [0x01; 16],
                node: 4,
            },
            generation: 0,
            role: "Button".into(),
            name: Some("Install".into()),
            value: None,
            bounds: Some([16.0, 204.0, 136.0, 236.0]),
            description: Some("Installs the latest build".into()),
        }),
        serde_json::json!({
            "kind": "described_identity",
            "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 4},
            "generation": 0,
            "role": "Button",
            "name": "Install",
            "value": null,
            "bounds": [16.0, 204.0, 136.0, 236.0],
            "description": "Installs the latest build",
        }),
    )]
}

/// M68 (the identity read's state facts at handle width): the M64
/// identity row's fields plus the state three — one enabled,
/// not-checkable button row (the m66 row keeps its byte-exact encoding;
/// the widening rides its own appended reply variant) and one focused,
/// mixed-checked checkbox row (the mixed state is its own vocabulary
/// word, never an Option<bool>).
fn describe_identity_state_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![
        (
            "described_identity_state_install_button",
            ResponseV1::DescribedIdentityState(DescribedIdentityStateV1 {
                node: NodeKeyV1::AccessKit {
                    tree: [0x01; 16],
                    node: 4,
                },
                generation: 0,
                role: "Button".into(),
                name: Some("Install".into()),
                value: None,
                bounds: Some([16.0, 204.0, 136.0, 236.0]),
                description: Some("Installs the latest build".into()),
                focused: false,
                enabled: true,
                checked: CheckedStateV1::NotCheckable,
            }),
            serde_json::json!({
                "kind": "described_identity_state",
                "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 4},
                "generation": 0,
                "role": "Button",
                "name": "Install",
                "value": null,
                "bounds": [16.0, 204.0, 136.0, 236.0],
                "description": "Installs the latest build",
                "focused": false,
                "enabled": true,
                "checked": "not_checkable",
            }),
        ),
        (
            "described_identity_state_mixed_checkbox",
            ResponseV1::DescribedIdentityState(DescribedIdentityStateV1 {
                node: NodeKeyV1::AccessKit {
                    tree: [0x01; 16],
                    node: 9,
                },
                generation: 0,
                role: "CheckBox".into(),
                name: Some("Partially installed".into()),
                value: None,
                bounds: Some([16.0, 284.0, 136.0, 316.0]),
                description: None,
                focused: true,
                enabled: true,
                checked: CheckedStateV1::Mixed,
            }),
            serde_json::json!({
                "kind": "described_identity_state",
                "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 9},
                "generation": 0,
                "role": "CheckBox",
                "name": "Partially installed",
                "value": null,
                "bounds": [16.0, 284.0, 136.0, 316.0],
                "description": null,
                "focused": true,
                "enabled": true,
                "checked": "mixed",
            }),
        ),
    ]
}

/// M70 (the identity read's state facts at set width): the M68 row per
/// match — the m68/m69 records' named standing gap cashed at set width
/// (the M64/M68 set-row encodings keep their byte-exact shapes; the
/// widening rides its own appended reply variant). One checked checkbox
/// row and one mixed one: the tri-state and the document order ride
/// together.
fn describe_all_identity_state_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "described_all_identity_state_checkboxes",
        ResponseV1::DescribedAllIdentityState {
            rows: vec![
                DescribedIdentityStateV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 9,
                    },
                    generation: 0,
                    role: "CheckBox".into(),
                    name: Some("Notifications".into()),
                    value: None,
                    bounds: Some([16.0, 236.0, 136.0, 268.0]),
                    description: None,
                    focused: false,
                    enabled: true,
                    checked: CheckedStateV1::Checked,
                },
                DescribedIdentityStateV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0x01; 16],
                        node: 11,
                    },
                    generation: 0,
                    role: "CheckBox".into(),
                    name: Some("Partially installed".into()),
                    value: None,
                    bounds: Some([16.0, 284.0, 136.0, 316.0]),
                    description: None,
                    focused: false,
                    enabled: true,
                    checked: CheckedStateV1::Mixed,
                },
            ],
        },
        serde_json::json!({
            "kind": "described_all_identity_state",
            "rows": [
                {
                    "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 9},
                    "generation": 0,
                    "role": "CheckBox",
                    "name": "Notifications",
                    "value": null,
                    "bounds": [16.0, 236.0, 136.0, 268.0],
                    "description": null,
                    "focused": false,
                    "enabled": true,
                    "checked": "checked",
                },
                {
                    "node": {"kind": "accesskit", "tree": "01".repeat(16), "node": 11},
                    "generation": 0,
                    "role": "CheckBox",
                    "name": "Partially installed",
                    "value": null,
                    "bounds": [16.0, 284.0, 136.0, 316.0],
                    "description": null,
                    "focused": false,
                    "enabled": true,
                    "checked": "mixed",
                },
            ],
        }),
    )]
}

/// The trace domain's reply pin (M97): the finalized-archive carrier —
/// the path plus the §12 recorder stats the reply names (the event rows,
/// the hash-deduped frame set, the ring pressure, the zip size). The TS
/// codec's `trace_file` row mirrors it.
fn trace_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "trace_file",
        ResponseV1::TraceFile {
            path: "/tmp/gw-trace/guiwright-trace-4242-1764000000.zip".into(),
            events: 12,
            frames: 3,
            records_dropped: 0,
            archive_bytes: 4096,
        },
        serde_json::json!({
            "kind": "trace_file",
            "path": "/tmp/gw-trace/guiwright-trace-4242-1764000000.zip",
            "events": 12,
            "frames": 3,
            "records_dropped": 0,
            "archive_bytes": 4096,
        }),
    )]
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
        // M10 (§7 render domain): the Shot reply appends after GestureDone.
        // The PNG payload is real bytes (a 1×1 red pixel) so the decoder
        // proves seq<u8> round-trips through postcard.
        (
            "shot",
            ResponseV1::Shot {
                frame_seq: 21,
                width: 1,
                height: 1,
                png: vec![
                    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF,
                ],
            },
            serde_json::json!({
                "kind": "shot",
                "frame_seq": 21,
                "width": 1,
                "height": 1,
                "png": [137, 80, 78, 71, 13, 10, 26, 10, 222, 173, 190, 239],
            }),
        ),
    ]
}

/// M11 (§7 tree domain): the `TreeDump` reply — a two-row dump (window root
/// + one child) proves the nested struct rows decode.
fn tree_dump_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![(
        "tree_dump",
        ResponseV1::TreeDump {
            tree_seq: 3,
            node_count: 2,
            focus: Some(NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: 0,
            }),
            nodes: vec![
                SnapshotNodeV1 {
                    key: NodeKeyV1::AccessKit {
                        tree: [0u8; 16],
                        node: 0,
                    },
                    role: "Window".into(),
                    name: Some("Test App".into()),
                    value: None,
                    bounds: Some([0.0, 0.0, 480.0, 240.0]),
                    children: 1,
                },
                SnapshotNodeV1 {
                    key: NodeKeyV1::AccessKit {
                        tree: [0u8; 16],
                        node: 2,
                    },
                    role: "Button".into(),
                    name: Some("Save".into()),
                    value: None,
                    bounds: Some([16.0, 16.0, 136.0, 48.0]),
                    children: 0,
                },
            ],
        },
        serde_json::json!({
            "kind": "tree_dump",
            "tree_seq": 3,
            "node_count": 2,
            "focus": {"kind": "accesskit", "tree": "0".repeat(32), "node": 0},
            "nodes": [
                {"key": {"kind": "accesskit", "tree": "0".repeat(32), "node": 0}, "role": "Window", "name": "Test App", "value": null, "bounds": [0.0, 0.0, 480.0, 240.0], "children": 1},
                {"key": {"kind": "accesskit", "tree": "0".repeat(32), "node": 2}, "role": "Button", "name": "Save", "value": null, "bounds": [16.0, 16.0, 136.0, 48.0], "children": 0},
            ],
        }),
    )]
}

/// (name, hex of the **framed** response, expected decoded JSON the
/// TypeScript decoder must produce).
fn response_vectors() -> Vec<(&'static str, String, serde_json::Value)> {
    let responses = core_responses()
        .into_iter()
        .chain(resolve_all_responses())
        .chain(hit_test_responses())
        .chain(describe_responses())
        .chain(describe_all_responses())
        .chain(describe_identity_responses())
        .chain(describe_identity_state_responses())
        .chain(describe_all_identity_state_responses())
        .chain(trace_responses())
        .chain(service_responses())
        .chain(media_window_responses())
        .chain(tree_dump_responses())
        .chain(event_responses());
    responses
        .map(|(name, resp, decoded)| {
            let frame = encode_frame(&guiwright_proto::GwpResponse::V1(resp)).expect("encode");
            (name, hex(&frame), decoded)
        })
        .collect()
}

/// M12 (§7 event channel): the pushed-event replies and the subscription
/// ack — the frames a subscribed connection receives between responses.
fn event_responses() -> Vec<(&'static str, ResponseV1, serde_json::Value)> {
    vec![
        (
            "event_tree_changed",
            ResponseV1::Event(EventV1::TreeChanged { seq: 4 }),
            serde_json::json!({ "kind": "event", "event": { "kind": "tree_changed", "seq": 4 } }),
        ),
        (
            "event_frame_presented",
            ResponseV1::Event(EventV1::FramePresented {
                seq: 7,
                hash: 0xDEAD_BEEF,
            }),
            serde_json::json!({
                "kind": "event",
                "event": { "kind": "frame_presented", "seq": 7, "hash": 3_735_928_559_u64 }
            }),
        ),
        (
            "event_log",
            ResponseV1::Event(EventV1::Log {
                record: "video writer dropped 2 static frames".into(),
            }),
            serde_json::json!({
                "kind": "event",
                "event": { "kind": "log", "record": "video writer dropped 2 static frames" }
            }),
        ),
        // The m20 record's named event: the state change rides the same
        // event envelope — the node and its post-action tri-state; the
        // bytes pin the appended EventV1/EventKindV1 rows.
        (
            "event_state_change",
            ResponseV1::Event(EventV1::StateChange {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 10,
                },
                checked: CheckedStateV1::Checked,
            }),
            serde_json::json!({
                "kind": "event",
                "event": {
                    "kind": "state_change",
                    "node": { "kind": "accesskit", "tree": "00".repeat(16), "node": 10 },
                    "checked": "checked"
                }
            }),
        ),
        (
            "subscribed",
            ResponseV1::Subscribed {
                kinds: vec![EventKindV1::TreeChanged, EventKindV1::FramePresented],
            },
            serde_json::json!({
                "kind": "subscribed",
                "kinds": ["tree_changed", "frame_presented"],
            }),
        ),
    ]
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

    if common::bless_write_permitted(
        std::env::var("GUIWRIGHT_BLESS").ok().as_deref(),
        std::env::var("NEXTEST").ok().as_deref(),
    ) {
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
