/**
 * The wire Trace domain's TS half (§7 "Domain: trace & inspector",
 * `Trace { Start(opts) | Stop | Attach(stream) }`): the **Start/Stop**
 * archive-control arms ride the new `RequestV1::Trace` domain —
 * `traceStart(opts)` / `traceStop()` on `GwpClient` mirroring the Rust
 * seam's `ClientConn::trace` row (the m55 videoStart/videoStop shape:
 * the start arm acks, the stop arm carries the finalized archive — the
 * `guiwright-<test>-<ts>.zip` path plus the §12 stats the recorder
 * reports). The **Attach** arm is the M18 live feed (`subscribe` /
 * `nextEvent`), unchanged here. The bytes pin against the Rust golden
 * vectors (the M3 video pin family's shape) — the two clients fail the
 * build together on drift.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { encodeRequest } from "../src/codec.ts";
import { encodeFrame } from "../src/framing.ts";
import type { GwpRequest } from "../src/schema.ts";
import { TraceV1 } from "../src/schema.ts";

const here = dirname(fileURLToPath(import.meta.url));
const golden = JSON.parse(readFileSync(join(here, "golden.json"), "utf8"));
const vectorByName = new Map(
  (golden.requests as { name: string; frame_hex: string }[]).map((v) => [v.name, v]),
);
const hexToBytes = (hex: string): Uint8Array => {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
};

const requestBytes = (req: GwpRequest): Uint8Array => encodeFrame(encodeRequest(req));

test("trace start composes the client-owned opts row", () => {
  const req: GwpRequest = {
    kind: "trace",
    cmd: { kind: "start", opts: { test: "e2e_trace", dir: "/tmp/gw-trace" } },
  };
  const vector = vectorByName.get("trace_start");
  assert.ok(vector, `golden vector for trace_start exists`);
  assert.deepEqual(
    Array.from(requestBytes(req)),
    Array.from(hexToBytes(vector.frame_hex)),
    "trace_start must byte-match the Rust golden",
  );
});

test("trace stop composes the unit row", () => {
  const req: GwpRequest = { kind: "trace", cmd: { kind: "stop" } };
  const vector = vectorByName.get("trace_stop");
  assert.ok(vector, `golden vector for trace_stop exists`);
  assert.deepEqual(
    Array.from(requestBytes(req)),
    Array.from(hexToBytes(vector.frame_hex)),
    "trace_stop must byte-match the Rust golden",
  );
});

test("the TraceV1 enum table mirrors the wire discriminants", () => {
  assert.equal(TraceV1.Start, 0);
  assert.equal(TraceV1.Stop, 1);
});
