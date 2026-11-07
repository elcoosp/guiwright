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
import {
  ClockV1,
  ErrorCode,
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
  assert.ok(golden.requests.length >= 21, "every request variant is pinned");
  assert.ok(golden.responses.length >= 15, "every response variant is pinned");
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
    assert.deepEqual(decoded, vector.decoded, `${vector.name} must decode exactly`);
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
  assert.deepEqual(byName(requests).slice(-4), ["Bye", "Video", "Window", "Gesture"]);
  assert.deepEqual(
    Object.values(RequestV1),
    requests.map((v: any) => Number(v.index)),
  );

  const responses = schema.responses;
  assert.deepEqual(byName(responses).slice(-4), [
    "VideoFile",
    "Windows",
    "WindowUsed",
    "GestureDone",
  ]);
  assert.deepEqual(
    Object.values(ResponseV1),
    responses.map((v: any) => Number(v.index)),
  );

  const domains = schema.domains;
  assert.deepEqual(Object.values(ClockV1), domains.clock.variants.map((v: any) => Number(v.index)));
  assert.deepEqual(Object.values(MockV1), domains.mock.variants.map((v: any) => Number(v.index)));
  assert.deepEqual(
    Object.values(VideoV1),
    domains.video.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(WindowV1),
    domains.window.variants.map((v: any) => Number(v.index)),
  );
  assert.deepEqual(
    Object.values(GestureV1),
    domains.gesture.variants.map((v: any) => Number(v.index)),
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
void InspectV1;
void NodeKeyV1;
void ErrorCode;
