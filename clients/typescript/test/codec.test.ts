/**
 * Codec tests (§16 M3): the TypeScript codec against golden vectors
 * captured from the Rust `guiwright-proto` (byte-for-byte), plus the
 * framing reassembler and the vendored schema copy.
 *
 * Requests are pinned encode-only (TS builds the same shapes the Rust
 * test did → byte-identical frames); responses are pinned decode-only
 * (Rust bytes → the expected decoded JSON).
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { encodeRequest, decodeResponse, isRejected } from "../src/codec.ts";
import { encodeFrame, FrameReader, MAX_FRAME_BYTES } from "../src/framing.ts";
import { filter } from "../src/filter.ts";
import {
  ClockV1,
  ErrorCode,
  EventKindV1,
  GestureV1,
  GwpRequest,
  GwpResponse,
  InputEventV1,
  InspectV1,
  MockV1,
  NodeKeyV1,
  PointerButtonV1,
  PROTO_VERSION,
  RequestV1,
  ResponseV1,
  SelectorV1,
  SemActionV1,
  ScreenshotV1,
  SnapshotV1,
  SubscribeV1,
  TraceV1,
  VideoV1,
  WindowV1,
} from "../src/schema.ts";

const here = dirname(fileURLToPath(import.meta.url));
const golden = JSON.parse(readFileSync(join(here, "golden.json"), "utf8"));
const hexToBytes = (hex: string): Uint8Array => {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
};

test("golden manifest is the expected shape", () => {
  assert.equal(golden.schema, "gwp-golden/v1");
  assert.ok(golden.requests.length >= 24, "every request variant is pinned");
  assert.ok(golden.responses.length >= 18, "every response variant is pinned");
  assert.ok(
    golden.requests.some((v: any) => v.name === "resolve_all_role"),
    "the M30 resolve-all request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "resolved_all"),
    "the M30 resolve-all reply is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "hit_test_save_center"),
    "the M32 hit-test request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "hit_test_hit") &&
      golden.responses.some((v: any) => v.name === "hit_test_miss"),
    "the M32 hit-test reply (hit + miss) is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "describe_save_handle"),
    "the M36 node-describe request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "described_save_button"),
    "the M36 node-describe reply is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "describe_all_buttons"),
    "the M39 describe-all request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "described_all_buttons"),
    "the M39 describe-all reply is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "describe_all_identity_buttons"),
    "the M64 describe-all identity request is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "describe_identity_save_handle"),
    "the M66 single-node identity describe request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "described_identity_install_button"),
    "the M66 single-node identity describe reply is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "describe_identity_state_save_handle"),
    "the M68 identity-state describe request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "described_identity_state_install_button"),
    "the M68 identity-state describe reply is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "describe_all_identity_state_checkboxes"),
    "the M70 set-width identity-state request is pinned",
  );
  assert.ok(
    golden.responses.some((v: any) => v.name === "described_all_identity_state_checkboxes"),
    "the M70 set-width identity-state reply is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "action_select_option"),
    "the m89 select-vocabulary request is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "action_scroll_into_view"),
    "the m90 scroll-into-view request is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "resolve_has"),
    "the m91 has-filter request is pinned",
  );
  assert.ok(
    golden.requests.some((v: any) => v.name === "resolve_filter"),
    "the m34 filter-row request is pinned (the m91 helper speaks it)",
  );
});

test("requests encode byte-identical to the Rust wire", () => {
  const vectorByName = new Map(golden.requests.map((v: any) => [v.name, v]));
  const build: Record<string, () => any> = {
    hello: () => ({
      kind: "hello",
      proto_version: PROTO_VERSION,
      agent: "@guiwright/gwp/0.3",
    }),
    ping: () => ({ kind: "ping" }),
    tree_snapshot: () => ({ kind: "tree_snapshot" }),
    resolve_role_named: () => ({
      kind: "resolve",
      selector: {
        kind: "role",
        role: "Button",
        name: { text: "Save", exact: false, ignore_case: true },
      },
      timeout_ms: 5000,
    }),
    resolve_test_id: () => ({
      kind: "resolve",
      selector: { kind: "test_id", id: "btn-save" },
      timeout_ms: 0,
    }),
    // The m91 has filter: the appended SelectorV1 variant's bytes pin
    // the published table (source, then the inner selector).
    resolve_has: () => ({
      kind: "resolve",
      selector: {
        kind: "has",
        source: { kind: "role", role: "Group", name: null },
        has: { kind: "role", role: "Button", name: null },
      },
      timeout_ms: 5000,
    }),
    // The m34 filter row through the m91 typed helper: the helper's
    // output (hasText default, hasNot, nth) encodes byte-identical to
    // the Rust wire — the raw rows no suite assembles by hand.
    resolve_filter: () => ({
      kind: "resolve",
      selector: filter(
        { kind: "role", role: "Button", name: null },
        {
          hasText: "Save",
          hasNot: { kind: "role", role: "TextInput", name: null },
          nth: "first",
        },
      ),
      timeout_ms: 5000,
    }),
    action_click: () => ({
      kind: "action",
      node: { kind: "accesskit", tree: "0".repeat(32), node: 2 },
      generation: 0,
      action: { kind: "click" },
      force: false,
      timeout_ms: 5000,
    }),
    action_press: () => ({
      kind: "action",
      node: { kind: "logical", adapter: "floem", id: 7 },
      generation: 3,
      action: { kind: "press", key: "Enter", modifiers: ["Shift"] },
      force: true,
      timeout_ms: 1000,
    }),
    // The m89 select vocabulary: the appended SemActionV1 variant's
    // bytes pin the published table against the Rust codec.
    action_select_option: () => ({
      kind: "action",
      node: { kind: "accesskit", tree: "0".repeat(32), node: 4 },
      generation: 0,
      action: { kind: "select_option", option: "France" },
      force: false,
      timeout_ms: 5000,
    }),
    // The m90 scroll-into-view vocabulary: the appended variant's
    // bytes pin the table.
    action_scroll_into_view: () => ({
      kind: "action",
      node: { kind: "accesskit", tree: "0".repeat(32), node: 2 },
      generation: 0,
      action: { kind: "scroll_into_view_if_needed" },
      force: false,
      timeout_ms: 5000,
    }),
    input_pointer: () => ({
      kind: "input",
      events: [
        { kind: "pointer_move", x: 12.5, y: 30.0 },
        { kind: "pointer_down", x: 12.5, y: 30.0, button: PointerButtonV1.Primary },
        { kind: "pointer_up", x: 12.5, y: 30.0, button: PointerButtonV1.Primary },
      ],
    }),
    clock_fast_forward: () => ({ kind: "clock", cmd: { kind: "fast_forward", ms: 30_000 } }),
    clock_state: () => ({ kind: "clock", cmd: { kind: "state" } }),
    mock_install_seam: () => ({
      kind: "mock",
      cmd: { kind: "install_seam", id: "reset_db", response: "canned" },
    }),
    mock_call: () => ({ kind: "mock", cmd: { kind: "call", id: "seed_users", scope: "smoke" } }),
    inspect_start: () => ({ kind: "inspect", cmd: { kind: "start_recording" } }),
    inspect_enable_overlay: () => ({ kind: "inspect", cmd: { kind: "enable_overlay" } }),
    inspect_disable_overlay: () => ({ kind: "inspect", cmd: { kind: "disable_overlay" } }),
    inspect_highlight: () => ({
      kind: "inspect",
      cmd: {
        kind: "highlight",
        node: { kind: "accesskit", tree: "a6000000000000000000000000000001", node: 2 },
      },
    }),
    inspect_highlight_clear: () => ({ kind: "inspect", cmd: { kind: "highlight", node: null } }),
    video_start: () => ({
      kind: "video",
      cmd: { kind: "start", opts: { fps: 30, dir: "test-results/videos" } },
    }),
    video_stop: () => ({ kind: "video", cmd: { kind: "stop" } }),
    window_list: () => ({ kind: "window", cmd: { kind: "list" } }),
    window_use: () => ({
      kind: "window",
      cmd: { kind: "use", handle: "a6000000-0000-0000-0000-000000000001" },
    }),
    gesture_click: () => ({
      kind: "gesture",
      cmd: {
        kind: "click",
        x: 12.5,
        y: 30.0,
        button: PointerButtonV1.Primary,
        count: 2,
      },
    }),
    gesture_drag: () => ({
      kind: "gesture",
      cmd: {
        kind: "drag",
        from_x: 10.0,
        from_y: 10.0,
        to_x: 110.0,
        to_y: 60.0,
        button: PointerButtonV1.Primary,
      },
    }),
    gesture_key: () => ({
      kind: "gesture",
      cmd: { kind: "key", key: "Enter", modifiers: ["Shift"] },
    }),
    screenshot_window: () => ({ kind: "screenshot", cmd: { kind: "window" } }),
    screenshot_element: () => ({
      kind: "screenshot",
      cmd: {
        kind: "element",
        selector: {
          kind: "role",
          role: "Button",
          name: { text: "Save", exact: false, ignore_case: true },
        },
        timeout_ms: 5000,
      },
    }),
    snapshot_full: () => ({ kind: "snapshot", cmd: { kind: "full" } }),
    snapshot_filtered: () => ({
      kind: "snapshot",
      cmd: {
        kind: "filtered",
        selector: { kind: "role", role: "Group", name: null },
        timeout_ms: 2500,
      },
    }),
    subscribe_on: () => ({
      kind: "subscribe",
      cmd: { kind: "on", kinds: ["tree_changed", "frame_presented"] },
    }),
    subscribe_off: () => ({ kind: "subscribe", cmd: { kind: "off" } }),
    // The resume vocabulary (the m18 record's named §7 follow-on): the
    // appended `Resume` variant's bytes pin the published table beside
    // the M12 pair.
    subscribe_resume: () => ({
      kind: "subscribe",
      cmd: {
        kind: "resume",
        kinds: ["tree_changed"],
        cursor: { tree_seq: 12, frame_seq: 34 },
      },
    }),
    resolve_all_role: () => ({
      kind: "resolve_all",
      selector: { kind: "role", role: "Button", name: null },
    }),
    hit_test_save_center: () => ({ kind: "hit_test", x: 76.0, y: 32.0 }),
    describe_save_handle: () => ({
      kind: "describe",
      node: { kind: "accesskit", tree: "01".repeat(16), node: 2 },
      generation: 0,
    }),
    describe_all_buttons: () => ({
      kind: "describe_all",
      selector: { kind: "role", role: "Button", name: null },
    }),
    describe_all_identity_buttons: () => ({
      kind: "describe_all_identity",
      selector: { kind: "role", role: "Button", name: null },
    }),
    describe_identity_save_handle: () => ({
      kind: "describe_identity",
      node: { kind: "accesskit", tree: "01".repeat(16), node: 2 },
      generation: 0,
    }),
    describe_identity_state_save_handle: () => ({
      kind: "describe_identity_state",
      node: { kind: "accesskit", tree: "01".repeat(16), node: 2 },
      generation: 0,
    }),
    describe_all_identity_state_checkboxes: () => ({
      kind: "describe_all_identity_state",
      selector: { kind: "role", role: "CheckBox", name: null },
    }),
    // The regex text probes (the m34 standing note cashed): the five
    // appended SelectorV1 rows, the pattern source as the payload — the
    // bytes pin the published table (the M64 rule at selector width).
    resolve_text_regex: () => ({
      kind: "resolve",
      selector: { kind: "text_regex", pattern: "(?i)save|cancel" },
      timeout_ms: 5000,
    }),
    resolve_label_regex: () => ({
      kind: "resolve",
      selector: { kind: "label_regex", pattern: "^Wel.* Ada$" },
      timeout_ms: 0,
    }),
    resolve_value_regex: () => ({
      kind: "resolve",
      selector: { kind: "value_regex", pattern: "^\\S+@\\S+\\.\\S+$" },
      timeout_ms: 0,
    }),
    resolve_role_name_regex: () => ({
      kind: "resolve",
      selector: {
        kind: "role_name_regex",
        role: "Button",
        pattern: "^(Save|Cancel)$",
      },
      timeout_ms: 5000,
    }),
    resolve_has_text_regex: () => ({
      kind: "resolve",
      selector: {
        kind: "has_text_regex",
        source: { kind: "role", role: "Button", name: null },
        pattern: "^Save$",
      },
      timeout_ms: 0,
    }),
    bye: () => ({ kind: "bye" }),
  };

  for (const [name, buildFn] of Object.entries(build)) {
    const vector = vectorByName.get(name);
    assert.ok(vector, `golden vector for ${name} exists`);
    const built = encodeFrame(encodeRequest(buildFn()));
    const expected = hexToBytes(vector.frame_hex);
    assert.deepEqual(
      Array.from(built),
      Array.from(expected),
      `${name} must encode to the exact Rust bytes`,
    );
  }
});

test("responses decode to the expected shapes", () => {
  for (const vector of golden.responses) {
    const frame = hexToBytes(vector.frame_hex);
    const body = frame.subarray(4);
    const decoded = decodeResponse(body);
    // Byte payloads (the §7 Shot png) decode to Uint8Array; the JSON
    // golden form is the plain array of the same bytes (bytes have no
    // JSON form — the same rule that renders NodeKey trees as hex).
    const actual = JSON.parse(
      JSON.stringify(decoded, (_k, v) => (v instanceof Uint8Array ? Array.from(v) : v)),
    );
    assert.deepEqual(actual, vector.decoded, `${vector.name} must decode exactly`);
  }
});

test("error responses are recognizable via isRejected", () => {
  const vector = golden.responses.find((v: any) => v.name === "error");
  const resp = decodeResponse(hexToBytes(vector.frame_hex).subarray(4));
  assert.ok(isRejected(resp, "StrictModeViolation"));
  assert.ok(!isRejected(resp, "Timeout"));
});

test("FrameReader reassembles split and coalesced chunks", () => {
  const frames = [
    encodeFrame(encodeRequest({ kind: "ping" })),
    encodeFrame(encodeRequest({ kind: "bye" })),
  ];
  const stream = new Uint8Array(frames[0].length + frames[1].length);
  stream.set(frames[0], 0);
  stream.set(frames[1], frames[0].length);

  // Split at every possible boundary: the reader must tolerate 1-byte chunks.
  const reader = new FrameReader();
  const seen: Uint8Array[] = [];
  for (const byte of stream) {
    seen.push(...reader.push(new Uint8Array([byte])));
  }
  assert.equal(seen.length, 2);
  assert.deepEqual(Array.from(seen[0]), Array.from(frames[0].subarray(4)));

  // Coalesced delivery lands both at once.
  const reader2 = new FrameReader();
  assert.equal(reader2.push(stream).length, 2);
});

test("oversized frames are rejected (the §7 guard)", () => {
  const evil = new Uint8Array(4);
  new DataView(evil.buffer).setUint32(0, MAX_FRAME_BYTES + 1, true);
  assert.throws(() => new FrameReader().push(evil), /exceeds the guard/);
});

test("the vendored schema copy matches the compiled variant tables", () => {
  const schema = JSON.parse(
    readFileSync(join(here, "..", "gwp-schema.json"), "utf8"),
  );
  const names = (table: any[]) => table.map((v) => Number(v.index)).join(",");
  const byName = (table: any[]) => table.map((v) => v.name);

  const requests = schema.requests;
  assert.equal(names(requests), [...Array(requests.length).keys()].join(","));
  assert.deepEqual(byName(requests).slice(-15), [
    "Video",
    "Window",
    "Gesture",
    "Screenshot",
    "Snapshot",
    "Subscribe",
    "ResolveAll",
    "HitTest",
    "Describe",
    "DescribeAll",
    "DescribeAllIdentity",
    "DescribeIdentity",
    "DescribeIdentityState",
    "DescribeAllIdentityState",
    "Trace",
  ]);
  assert.deepEqual(
    Object.values(RequestV1),
    requests.map((v: any) => Number(v.index)),
  );

  const responses = schema.responses;
  assert.deepEqual(byName(responses).slice(-16), [
    "Windows",
    "WindowUsed",
    "GestureDone",
    "Shot",
    "TreeDump",
    "Event",
    "Subscribed",
    "ResolvedAll",
    "HitTest",
    "Described",
    "DescribedAll",
    "DescribedAllIdentity",
    "DescribedIdentity",
    "DescribedIdentityState",
    "DescribedAllIdentityState",
    "TraceFile",
  ]);
  assert.deepEqual(
    Object.values(ResponseV1),
    responses.map((v: any) => Number(v.index)),
  );

  const domains = schema.domains;
  assert.deepEqual(Object.values(ClockV1), domains.clock.variants.map((v: any) => Number(v.index)));
  assert.deepEqual(Object.values(MockV1), domains.mock.variants.map((v: any) => Number(v.index)));
  // The M8 overlay half rides the published table too (the pre-M12 hole
  // — the vendored copy now covers the whole InspectV1 domain).
  assert.deepEqual(
    Object.values(InspectV1),
    domains.inspect.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(VideoV1),
    domains.video.variants.map((v: any) => Number(v.index)),
  );
  // The M97 archive-control domain rides the published table too (the
  // Start/Stop arms the M18 Attach arm completes).
  assert.deepEqual(
    Object.values(TraceV1),
    domains.trace.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(WindowV1),
    domains.window.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(GestureV1),
    domains.gesture.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(ScreenshotV1),
    domains.screenshot.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(SnapshotV1),
    domains.snapshot.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(SubscribeV1),
    domains.subscribe.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(EventKindV1),
    domains.event_kind.variants.map((v: any) => Number(v.index)),
  );
  assert.equal(schema.protocol_version, PROTO_VERSION);
});

test("the protocol version rides the schema and the client", async () => {
  assert.equal(PROTO_VERSION, 1);
  const client = await import("../src/client.ts");
  assert.equal(client.PROTO_VERSION, 1);
});

// Enum tables referenced above are exercised via their constant values —
// a reorder in Rust shifts the golden bytes and fails the encode tests.
void GwpRequest;
void GwpResponse;
void SelectorV1;
void SemActionV1;
void InputEventV1;
void NodeKeyV1;
void ErrorCode;
