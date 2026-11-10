/**
 * Client tests: `GwpClient` against an in-process Node TCP server that
 * speaks the wire by hand (independent of the codec under test where
 * possible). The cross-language e2e (`scripts/e2e.mjs`) covers the same
 * conversation against a real Rust server.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import * as net from "node:net";

import { GwpClient, GwpRejected } from "../src/client.ts";
import { Encoder } from "../src/postcard.ts";

/** A minimal hand-rolled GWP server: Welcome → Pong → clock reply → Bye. */
function startMockServer(): Promise<{ port: number; close: () => void }> {
  const server = net.createServer((socket) => {
    let first = true;
    socket.on("data", (chunk: Buffer) => {
      // Parse the 4-byte length prefix + body; we only read the variant
      // byte (offset 4 = envelope, 5 = variant) — deliberately hand-rolled.
      const len = chunk.readUInt32LE(0);
      const body = chunk.subarray(4, 4 + len);
      const envelope = body[0];
      assert.equal(envelope, 0, "V1 envelope");
      const variant = body[1];
      const reply = (payload: Uint8Array) => {
        const frame = new Uint8Array(4 + payload.length);
        const view = new DataView(frame.buffer);
        view.setUint32(0, payload.length, true);
        frame.set(payload, 4);
        socket.write(frame);
      };
      const resp = (payload: number[]) => reply(Uint8Array.from(payload));

      if (variant === 0) {
        // Hello → Welcome{...} — hand-encoded postcard.
        const enc = new Encoder();
        enc.u32(0); // GwpResponse::V1
        enc.u32(0); // Welcome
        enc.u32(1); // proto_version
        enc.str("mock-session");
        enc.u64(1);
        enc.u64(2);
        enc.str("mock");
        enc.str("fixture");
        enc.str("0.1");
        for (const bit of [true, false, true, false, false, true]) enc.bool(bit);
        resp(Array.from(enc.finish()));
        first = false;
        return;
      }
      void first;
      if (variant === 1) {
        // Ping → Pong
        resp([0, 1]);
        return;
      }
      if (variant === 6) {
        // Clock(State) → ClockState{Some(32000), false, 0}
        const enc = new Encoder();
        enc.u32(0).u32(6);
        enc.optionTag(true).u64(32_000);
        enc.bool(false);
        enc.u64(0);
        resp(Array.from(enc.finish()));
        return;
      }
      if (variant === 8) {
        // Inspect(…) → Pong (§13 M8 overlay/highlight surface; the mock
        // server acknowledges the domain like the real driver does).
        resp([0, 1]);
        return;
      }
      if (variant === 9) {
        // Bye → Bye
        resp([0, 10]);
        socket.end();
        return;
      }
      // Anything else → Error{Internal}
      const enc = new Encoder();
      enc.u32(0).u32(9);
      enc.u32(7);
      enc.str("not implemented in the mock server");
      resp(Array.from(enc.finish()));
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address() as net.AddressInfo;
      resolve({ port: addr.port, close: () => server.close() });
    });
  });
}

test("handshake, ping, clock and a typed rejection over real TCP", async () => {
  const server = await startMockServer();
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    const welcome = await client.handshake("client.test/0.1");
    assert.equal(welcome.session_id, "mock-session");
    assert.equal(welcome.proto_version, 1);
    assert.equal(welcome.adapter.name, "mock");
    assert.equal(welcome.adapter.caps.tree_update, true);
    assert.equal(welcome.adapter.caps.clock, true);

    await client.ping();

    const clock = await client.clock({ kind: "state" });
    assert.equal(clock.virtual_time_ms, 32_000);
    assert.equal(clock.paused, false);

    // TreeSnapshot is not implemented by the mock → a typed rejection.
    await assert.rejects(
      () => client.treeSnapshot(),
      (err: unknown) => {
        assert.ok(err instanceof GwpRejected, `expected GwpRejected, got ${err}`);
        assert.equal((err as GwpRejected).codeName, "Internal");
        return true;
      },
    );

    // Inspector overlay (§13, M8): enable → highlight → clear → disable,
    // the exact conversation the REPL runs.
    await client.inspectOverlay(true);
    await client.inspectHighlight({
      kind: "accesskit",
      tree: "a6000000000000000000000000000001",
      node: 2,
    });
    await client.inspectHighlight(null);
    await client.inspectOverlay(false);

    await client.close();
  } finally {
    server.close();
    server.closeAllConnections?.();
  }
});

test("requests before handshake fail fast (§7 conversation)", async () => {
  const server = await startMockServer();
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    try {
      // ping() is async: the gate rejects (no sync throw).
      await assert.rejects(() => client.ping(), /handshake/);
    } finally {
      await client.close();
    }
  } finally {
    server.close();
    server.closeAllConnections?.();
  }
});
