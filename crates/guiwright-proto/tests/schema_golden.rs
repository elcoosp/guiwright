//! Golden pin for the published GWP schema (`guiwright_proto::schemadoc`,
//! §16 "schema-published"). The document is the reviewed contract that
//! other-language clients implement; this test fails the build on any
//! drift — reorders, renames, payload-shape edits — not just on appends.
//!
//! Bless after an intentional schema change:
//!
//! ```text
//! GUIWRIGHT_BLESS=1 cargo test -p guiwright-proto --test schema_golden
//! ```
//!
//! (same env-var flow as the screenshot baselines, §10), then re-publish
//! the copy the TypeScript client vendors: copy the printed path's file to
//! `clients/typescript/gwp-schema.json`.

use guiwright_proto::schema::{ClockV1, MockV1};
use guiwright_proto::{RequestV1, ResponseV1, schemadoc::gwp_schema_json};

const GOLDEN_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/schema-golden.json");

/// The hand-authored variant tables in `schemadoc` must track the real
/// enums: counts first (a new variant forces both sides to move), then the
/// golden file pins names, order and payloads byte-for-byte.
#[test]
fn published_schema_matches_the_wire_enums() {
    let doc = gwp_schema_json();

    let count = |key: &str| -> usize {
        doc[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} table missing"))
            .len()
    };
    assert_eq!(count("requests"), request_variants(), "requests table");
    assert_eq!(count("responses"), response_variants(), "responses table");
    assert_eq!(count("events"), 3, "events table");

    let domains = &doc["domains"];
    assert_eq!(
        domains["clock"]["variants"].as_array().unwrap().len(),
        clock_variants()
    );
    assert_eq!(
        domains["mock"]["variants"].as_array().unwrap().len(),
        mock_variants()
    );
    assert_eq!(domains["inspect"]["variants"].as_array().unwrap().len(), 2);
    assert_eq!(domains["video"]["variants"].as_array().unwrap().len(), 2);
    assert_eq!(domains["window"]["variants"].as_array().unwrap().len(), 2);
    assert_eq!(domains["gesture"]["variants"].as_array().unwrap().len(), 5);

    // Wire indices are exactly the enum positions: spot-check the ends of
    // both request/response tables (the append-only frontier).
    assert_eq!(doc["requests"][9]["name"], "Bye");
    assert_eq!(doc["requests"][10]["name"], "Video");
    assert_eq!(doc["requests"][11]["name"], "Window");
    assert_eq!(doc["requests"][12]["name"], "Gesture");
    assert_eq!(doc["responses"][10]["name"], "Bye");
    assert_eq!(doc["responses"][11]["name"], "VideoFile");
    assert_eq!(doc["responses"][12]["name"], "Windows");
    assert_eq!(doc["responses"][13]["name"], "WindowUsed");
    assert_eq!(doc["responses"][14]["name"], "GestureDone");

    // The error-code table is flat on the wire: appending UnknownWindow
    // keeps the pre-M6 codes byte-identical.
    assert_eq!(
        doc["error_codes"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .as_str()
            .unwrap(),
        "UnknownWindow"
    );
}

/// Exhaustive matches: adding a variant anywhere below fails compilation
/// here and forces the schemadoc tables + golden to move with the wire.
fn response_variants() -> usize {
    let probe = |r: ResponseV1| match r {
        ResponseV1::Welcome { .. }
        | ResponseV1::Pong
        | ResponseV1::TreeSnapshot { .. }
        | ResponseV1::Resolved { .. }
        | ResponseV1::ActionDone { .. }
        | ResponseV1::InputQueued { .. }
        | ResponseV1::ClockState { .. }
        | ResponseV1::MockDone { .. }
        | ResponseV1::Codegen { .. }
        | ResponseV1::Error { .. }
        | ResponseV1::Bye
        | ResponseV1::VideoFile { .. }
        | ResponseV1::Windows { .. }
        | ResponseV1::WindowUsed { .. }
        | ResponseV1::GestureDone { .. } => 15usize,
    };
    probe(ResponseV1::Pong)
}

fn request_variants() -> usize {
    13 + match RequestV1::Ping {
        RequestV1::Hello { .. }
        | RequestV1::Ping
        | RequestV1::TreeSnapshot
        | RequestV1::Resolve { .. }
        | RequestV1::Action { .. }
        | RequestV1::Input { .. }
        | RequestV1::Clock(_)
        | RequestV1::Mock(_)
        | RequestV1::Inspect(_)
        | RequestV1::Bye
        | RequestV1::Video(_)
        | RequestV1::Window(_)
        | RequestV1::Gesture(_) => 0,
    }
}

fn clock_variants() -> usize {
    let probe = |c: ClockV1| match c {
        ClockV1::InstallVirtual
        | ClockV1::FastForward { .. }
        | ClockV1::RunFor { .. }
        | ClockV1::Pause
        | ClockV1::Resume
        | ClockV1::SetTime { .. }
        | ClockV1::State => 7usize,
    };
    probe(ClockV1::State)
}

fn mock_variants() -> usize {
    let probe = |m: MockV1| match m {
        MockV1::InstallSeam { .. }
        | MockV1::Clear { .. }
        | MockV1::ResetState { .. }
        | MockV1::Call { .. } => 4usize,
    };
    probe(MockV1::Clear { id: String::new() })
}

/// The full document is pinned byte-for-byte against the committed golden.
#[test]
fn gwp_schema_json_matches_golden() {
    let doc = serde_json::to_string_pretty(&gwp_schema_json()).expect("pretty json") + "\n";

    if std::env::var("GUIWRIGHT_BLESS").ok().as_deref() == Some("1") {
        std::fs::write(GOLDEN_PATH, &doc).expect("write golden");
        eprintln!("blessed: {GOLDEN_PATH}");
        return;
    }

    let golden = std::fs::read_to_string(GOLDEN_PATH).unwrap_or_else(|e| {
        panic!("golden missing at {GOLDEN_PATH} (run with GUIWRIGHT_BLESS=1): {e}")
    });
    assert_eq!(
        doc, golden,
        "published GWP schema drifted from the golden; if intentional, re-run with GUIWRIGHT_BLESS=1 and re-vendor clients/typescript/gwp-schema.json"
    );
}
