/**
 * Client tests: `GwpClient` against an in-process Node TCP server that
 * speaks the wire by hand (independent of the codec under test where
 * possible). The cross-language e2e (`scripts/e2e.ts`) covers the same
 * conversation against a real Rust server.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import * as net from "node:net";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { GwpClient, GwpRejected, GwpWaitTimeout } from "../src/client.ts";
import { Encoder } from "../src/postcard.ts";

/** Hex → bytes (the golden vectors' wire form, the codec tests' shape). */
function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

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

/** A mock server that speaks the §7 event channel (M12): Subscribe →
 * Subscribed + a pushed Event frame, interleaved with normal replies. */
function startEventServer(): Promise<{ port: number; close: () => void }> {
  const server = net.createServer((socket) => {
    socket.on("data", (chunk: Buffer) => {
      const len = chunk.readUInt32LE(0);
      const body = chunk.subarray(4, 4 + len);
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
        // Hello → Welcome (the same hand-encoded shape as startMockServer).
        const enc = new Encoder();
        enc.u32(0).u32(0).u32(1);
        enc.str("mock-events");
        enc.u64(1);
        enc.u64(2);
        enc.str("mock");
        enc.str("fixture");
        enc.str("0.1");
        for (const bit of [true, false, true, false, false, true]) enc.bool(bit);
        resp(Array.from(enc.finish()));
        return;
      }
      if (variant === 1) {
        // Ping → Pong, with an Event frame pushed right before it — the
        // interleave the client's demux must survive.
        resp([0, 17, 0, 4]); // Event(TreeChanged { seq: 4 })
        resp([0, 1]); // Pong
        return;
      }
      if (variant === 15) {
        // Subscribe → Subscribed { kinds } (+ the event half above).
        const cmd = body[2];
        if (cmd === 0) {
          resp([0, 18, 2, 0, 1]); // Subscribed { kinds: [TreeChanged, FramePresented] }
        } else {
          resp([0, 18, 0]); // Subscribed { kinds: [] }
        }
        return;
      }
      if (variant === 9) {
        resp([0, 10]);
        socket.end();
        return;
      }
      const enc = new Encoder();
      enc.u32(0).u32(9).u32(7);
      enc.str("not implemented in the event mock");
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

test("subscribe + nextEvent demux pushed events from responses (§7, M12)", async () => {
  const server = await startEventServer();
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    await client.handshake("client.test/0.1");

    // The ack echoes the honored kinds.
    const ack = await client.subscribe(["tree_changed", "frame_presented"]);
    assert.deepEqual(ack, ["tree_changed", "frame_presented"]);

    // A ping whose Pong arrives interleaved with a pushed Event frame:
    // the demux must land the Pong on the request and the Event in the
    // buffer — never swapped.
    await client.ping();
    const event = await client.nextEvent();
    assert.deepEqual(event, { kind: "tree_changed", seq: 4 });

    // The waiter path: arm nextEvent() BEFORE the event arrives.
    const pending = client.nextEvent();
    await client.ping(); // the mock pushes an event before every Pong
    assert.deepEqual(await pending, { kind: "tree_changed", seq: 4 });

    // Off: the ack is empty.
    await client.unsubscribe();
    await client.close();
  } finally {
    server.close();
    server.closeAllConnections?.();
  }
});

/** The M41 bulk-read server: handshake, then the DescribeAll request
 * (variant 19) is answered with the blessed `described_all_buttons`
 * frame — the real Rust server's M39 reply, replayed byte-for-byte. */
function startBulkReadServer(replyBody: Uint8Array): Promise<{
  port: number;
  close: () => void;
}> {
  return new Promise((resolve) => {
    const server = net.createServer((socket) => {
      socket.on("data", (chunk: Buffer) => {
        const variant = chunk[5];
        const reply = (payload: Uint8Array) => {
          const frame = new Uint8Array(4 + payload.length);
          const view = new DataView(frame.buffer);
          view.setUint32(0, payload.length, true);
          frame.set(payload, 4);
          socket.write(frame);
        };
        if (variant === 0) {
          // Hello → Welcome{...} — the hand-encoded postcard the other
          // tests speak (§7: the handshake must land first).
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
          reply(enc.finish());
          return;
        }
        if (variant === 19) {
          // DescribeAll → the blessed DescribedAll rows.
          reply(replyBody);
          return;
        }
        if (variant === 9) {
          // Bye → Bye (the clean close, §7's last word).
          reply(Uint8Array.from([0, 10]));
          socket.end();
          return;
        }
        // Anything else → Error{Internal} (the other tests' fallback).
        const enc = new Encoder();
        enc.u32(0).u32(9);
        enc.u32(7);
        enc.str("not implemented in the bulk-read mock server");
        reply(Array.from(enc.finish()));
      });
    });
    server.listen(0, "127.0.0.1", () => {
      const port = (server.address() as net.AddressInfo).port;
      resolve({
        port,
        close: () => {
          server.close();
          server.closeAllConnections?.();
        },
      });
    });
  });
}

test("the bulk reads derive the kit source orders from the describe-all rows (M41)", async () => {
  // The golden file is the blessed cross-language contract: the M39
  // vector is what a real Rust server answers DescribeAll with.
  const golden = JSON.parse(
    readFileSync(join(dirname(fileURLToPath(import.meta.url)), "golden.json"), "utf8"),
  );
  const described = golden.responses.find(
    (v: { name: string }) => v.name === "described_all_buttons",
  );
  const frame = hexToBytes(described.frame_hex);
  const server = await startBulkReadServer(frame.subarray(4));
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    await client.handshake("client.test/0.1");

    // texts: the m29 source order — label first, value fallback. The
    // vector is the 2-row byte-pin fixture (the real 4-button walk is
    // the e2e leg's), its rows carry no values, so both reads answer
    // the names in document order — `values` through the label
    // fallback (the m31 rule).
    const texts = await client.allTextContents({ kind: "role", role: "Button", name: null });
    assert.deepEqual(texts, ["Save", "Cancel"]);
    const values = await client.allValues({ kind: "role", role: "Button", name: null });
    assert.deepEqual(
      values,
      ["Save", "Cancel"],
      "no values on the vector — the label fallback answers (the m31 rule)",
    );

    await client.close();
  } finally {
    server.close();
  }
});

/** The M45 wait server: handshake, then subscribe acks, describe-all
 * replies from a script (in order), one optional pushed event, and the
 * request log the subscription-ownership assertions read. The bulk-read
 * server's shape, widened for the wait's conversation. */
function startWaitServer(
  bodies: Uint8Array[],
  pushAfterFirstDescribe: Uint8Array | null,
): Promise<{
  port: number;
  requests: string[];
  close: () => void;
}> {
  const requests: string[] = [];
  let describes = 0;
  let pushed = false;
  return new Promise((resolve) => {
    const server = net.createServer((socket) => {
      socket.on("data", (chunk: Buffer) => {
        const variant = chunk[5];
        const reply = (payload: Uint8Array) => {
          const frame = new Uint8Array(4 + payload.length);
          const view = new DataView(frame.buffer);
          view.setUint32(0, payload.length, true);
          frame.set(payload, 4);
          socket.write(frame);
        };
        if (variant === 0) {
          requests.push("hello");
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
          reply(enc.finish());
          return;
        }
        if (variant === 15) {
          // Subscribe(On|Off) → the ack echoing the filter (§7, M12).
          const on = chunk[6] === 0;
          requests.push(on ? "subscribe_on" : "subscribe_off");
          const enc = new Encoder();
          enc.u32(0).u32(18);
          if (on) {
            enc.seqLen(2).u32(0).u32(1); // tree_changed + frame_presented
          } else {
            enc.seqLen(0);
          }
          reply(enc.finish());
          return;
        }
        if (variant === 19) {
          // DescribeAll → the next scripted body; optionally push a
          // TreeChanged event after the first reply (the wake leg).
          requests.push(`describe_${describes}`);
          const body = bodies[Math.min(describes, bodies.length - 1)];
          describes += 1;
          reply(body);
          if (pushAfterFirstDescribe !== null && describes === 1 && !pushed) {
            pushed = true;
            socket.write(pushAfterFirstDescribe);
          }
          return;
        }
        if (variant === 9) {
          // Bye → Bye (the clean close, §7's last word).
          requests.push("bye");
          reply(Uint8Array.from([0, 10]));
          socket.end();
          return;
        }
        requests.push(`unexpected_variant_${variant}`);
        // Ping → Pong keeps any post-wait probe honest.
        if (variant === 1) {
          requests.push("ping");
          reply(Uint8Array.from([0, 1]));
          return;
        }
      });
    });
    server.listen(0, "127.0.0.1", () => {
      const port = (server.address() as net.AddressInfo).port;
      resolve({ port, requests, close: () => server.close() });
    });
  });
}

test("waitFor(visible) is satisfied on entry and drops the subscription (M45)", async () => {
  // The blessed cross-language contract again: the M39 vector is what a
  // real Rust server answers DescribeAll with; its Save row carries
  // bounds, so the visible wait is satisfied on the entry read.
  const golden = JSON.parse(
    readFileSync(join(dirname(fileURLToPath(import.meta.url)), "golden.json"), "utf8"),
  );
  const described = golden.responses.find(
    (v: { name: string }) => v.name === "described_all_buttons",
  );
  const frame = hexToBytes(described.frame_hex);
  const server = await startWaitServer([frame.subarray(4)], null);
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    await client.handshake("client.test/0.1");

    const row = await client.waitFor(
      { kind: "role", role: "Button", name: { text: "Save", exact: true, ignore_case: false } },
      "visible",
    );
    assert.equal(row?.name, "Save");
    assert.deepEqual(row?.bounds, [16, 16, 136, 48]);

    // The subscription window dropped on return (§7/M12 ownership).
    assert.ok(server.requests.includes("subscribe_off"), "the wait unsubscribed on return");
    await client.close();
  } finally {
    server.close();
  }
});

test("waitFor wakes on the event channel and re-reads (M45)", async () => {
  // The composition the m42 record named: empty set → a pushed
  // TreeChanged wake → the re-read answers. No polling anywhere — the
  // wake arrives because the wait subscribed.
  const golden = JSON.parse(
    readFileSync(join(dirname(fileURLToPath(import.meta.url)), "golden.json"), "utf8"),
  );
  const described = golden.responses.find(
    (v: { name: string }) => v.name === "described_all_buttons",
  );
  const frame = hexToBytes(described.frame_hex);
  // Empty DescribedAll (count 0), then the blessed rows on the wake.
  const empty = new Encoder();
  empty.u32(0).u32(22).seqLen(0);
  const wake = new Encoder();
  wake.u32(0).u32(17).u32(0).u64(7); // Event(TreeChanged{seq:7})
  const wakeFrame = new Uint8Array(4 + wake.finish().length);
  new DataView(wakeFrame.buffer).setUint32(0, wake.finish().length, true);
  wakeFrame.set(wake.finish(), 4);

  const server = await startWaitServer([empty.finish(), frame.subarray(4)], wakeFrame);
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    await client.handshake("client.test/0.1");

    const row = await client.waitFor({ kind: "role", role: "Button", name: null }, "visible", 2000);
    assert.equal(row?.name, "Save", "the re-read after the tree wake satisfied the wait");

    await client.close();
  } finally {
    server.close();
  }
});

test("waitFor expiry is the typed timeout and the door stays healthy (M45)", async () => {
  // Nothing ever answers the selector: the spent budget rejects with
  // GwpWaitTimeout naming the state, the locator and the last
  // evaluation — and the SAME connection keeps answering afterwards
  // (the m44 read-half contract on the TS side: FrameReader always
  // buffered; the subscription drops best-effort on expiry).
  const empty = new Encoder();
  empty.u32(0).u32(22).seqLen(0);
  const server = await startWaitServer([empty.finish()], null);
  try {
    const client = await GwpClient.connectTcp(`127.0.0.1:${server.port}`);
    await client.handshake("client.test/0.1");

    await assert.rejects(
      () =>
        client.waitFor(
          {
            kind: "role",
            role: "Button",
            name: { text: "Ghost", exact: true, ignore_case: false },
          },
          "visible",
          120,
        ),
      (err) => {
        assert.ok(err instanceof GwpWaitTimeout, "the typed wait timeout");
        assert.equal(err.state, "visible");
        assert.equal(err.timeoutMs, 120);
        assert.match(err.selector, /getByRole\("Button", "Ghost"\)/);
        assert.match(err.message, /timed out after 120 ms/);
        assert.match(err.lastEvaluation, /no match/);
        return true;
      },
    );

    // The expired wait left the subscription dropped and the door alive.
    assert.ok(server.requests.includes("subscribe_off"), "the expiry unsubscribed");
    await client.ping();
    await client.close();
  } finally {
    server.close();
  }
});
