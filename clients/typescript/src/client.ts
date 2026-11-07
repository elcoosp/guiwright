/**
 * `GwpClient` — the other-language client surface (architecture §16
 * "Other-language clients — GWP over the wire; schema-published", M3).
 *
 * Transports:
 *  - TCP (`node:net`) — the §7 transport for CI containers + this client
 *    (the cross-language e2e drives a real Rust server over it);
 *  - WebSocket (the global `WebSocket` client, Node ≥ 22) — the §7/§12
 *    viewer live-mode door.
 *
 * One request → one response (§7 conversation): connect, `handshake()`
 * first, then drive. `Error` responses reject with `GwpRejected`.
 */

import * as net from "node:net";
import { encodeRequest, decodeResponse } from "./codec.ts";

/** The protocol version this client speaks. */
export { PROTO_VERSION } from "./schema.ts";
import { encodeFrame, FrameReader } from "./framing.ts";
import {
  type ClockCmd,
  type GwpRequest,
  type GwpResponse,
  type MockCmd,
  type Selector,
  type SemAction,
  type NodeKey,
  type VideoCmd,
  type Welcome,
  PROTO_VERSION,
} from "./schema.ts";

/** A typed driver rejection (`ResponseV1::Error`, §7). */
export class GwpRejected extends Error {
  readonly code: number;
  readonly codeName: string;

  constructor(code: number, message: string) {
    super(`driver rejected the request (code ${code}): ${message}`);
    this.name = "GwpRejected";
    this.code = code;
    this.codeName = ERROR_NAMES[code] ?? `Unknown(${code})`;
  }
}

const ERROR_NAMES: Record<number, string> = {
  0: "UnsupportedVersion",
  1: "NotHandshaked",
  2: "UnknownNode",
  3: "StaleGeneration",
  4: "StrictModeViolation",
  5: "Timeout",
  6: "Unsupported",
  7: "Internal",
};

/** The protocol version this client speaks. */

export class GwpClient {
  private pending: ((resp: GwpResponse) => void) | null = null;
  private reader = new FrameReader();
  private handshakeDone = false;
  private readonly transport: {
    write: (bytes: Uint8Array) => void;
    close: () => Promise<void>;
  };
  welcome: Welcome | null = null;

  private constructor(transport: {
    write: (bytes: Uint8Array) => void;
    close: () => Promise<void>;
  }) {
    this.transport = transport;
  }

  /** Wire the client onto a byte stream (already connected). */
  static over(transport: {
    write: (bytes: Uint8Array) => void;
    close: () => Promise<void>;
    onData: (cb: (bytes: Uint8Array) => void) => void;
  }): GwpClient {
    const client = new GwpClient(transport);
    transport.onData((bytes) => client.onBytes(bytes));
    return client;
  }

  /** Connect over TCP (`127.0.0.1:43210`). */
  static async connectTcp(addr: string): Promise<GwpClient> {
    const socket = net.connect(await parseTcp(addr));
    await onceEvent(socket, "connect");
    socket.setNoDelay(true);
    const client = GwpClient.over({
      write: (bytes) => socket.write(bytes),
      close: () =>
        new Promise((resolve) => {
          socket.end(() => resolve());
        }),
      onData: (cb) => {
        socket.on("data", (chunk: Buffer) => cb(new Uint8Array(chunk)));
      },
    });
    return client;
  }

  /** Connect over WebSocket (`ws://…`) — the viewer live-mode door. */
  static async connectWs(url: string): Promise<GwpClient> {
    const ws = new WebSocket(url);
    await onceEvent(ws, "open");
    const client = GwpClient.over({
      write: (bytes) => {
        ws.send(bytes);
      },
      close: () =>
        new Promise((resolve) => {
          ws.addEventListener(
            "close",
            () => resolve(),
            { once: true },
          );
          ws.close();
        }),
      onData: (cb) => {
        ws.addEventListener("message", async (ev: MessageEvent) => {
          const data = ev.data as unknown;
          if (data instanceof ArrayBuffer) {
            cb(new Uint8Array(data));
          } else if (data instanceof Blob) {
            cb(new Uint8Array(await data.arrayBuffer()));
          } else if (ArrayBuffer.isView(data)) {
            cb(new Uint8Array((data as Uint8Array<ArrayBuffer>).buffer));
          }
        });
      },
    });
    return client;
  }

  private onBytes(bytes: Uint8Array): void {
    for (const frame of this.reader.push(bytes)) {
      const resp = decodeResponse(frame);
      const resolve = this.pending;
      this.pending = null;
      if (resolve) resolve(resp);
      // Unsolicited frames (events) are tolerated and dropped until the
      // event-subscription milestone (§7 multiplexing).
    }
  }

  private async request(req: GwpRequest): Promise<GwpResponse> {
    const frame = encodeFrame(encodeRequest(req));
    const response = await new Promise<GwpResponse>((resolve) => {
      this.pending = resolve;
      this.transport.write(frame);
    });
    if (response.kind === "error") {
      throw new GwpRejected(response.code, response.message);
    }
    return response;
  }

  /** §7 handshake — must be the first call on a connection. */
  async handshake(agent = "@guiwright/gwp"): Promise<Welcome> {
    const resp = await this.request({
      kind: "hello",
      proto_version: PROTO_VERSION,
      agent,
    });
    if (resp.kind !== "welcome") {
      throw new Error(`expected Welcome, got ${resp.kind}`);
    }
    if (resp.proto_version !== PROTO_VERSION) {
      throw new Error(
        `driver protocol ${resp.proto_version}, client protocol ${PROTO_VERSION}`,
      );
    }
    this.handshakeDone = true;
    this.welcome = resp;
    return resp;
  }

  private assertHandshaked(): void {
    if (!this.handshakeDone) {
      throw new Error("handshake() must run before any other request (§7)");
    }
  }

  async ping(): Promise<void> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "ping" });
    if (resp.kind !== "pong") throw new Error(`expected Pong, got ${resp.kind}`);
  }

  async treeSnapshot(): Promise<{ tree_seq: number; node_count: number }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "tree_snapshot" });
    if (resp.kind !== "tree_snapshot") throw new Error(`expected TreeSnapshot, got ${resp.kind}`);
    return { tree_seq: resp.tree_seq, node_count: resp.node_count };
  }

  /** Resolve with server-side auto-wait (§8.2 over the wire). */
  async resolve(
    selector: Selector,
    timeoutMs = 5000,
  ): Promise<{ node: NodeKey; generation: number; rect: number[] | null }> {
    this.assertHandshaked();
    const resp = await this.request({
      kind: "resolve",
      selector,
      timeout_ms: timeoutMs,
    });
    if (resp.kind !== "resolved") throw new Error(`expected Resolved, got ${resp.kind}`);
    return { node: resp.node, generation: resp.generation, rect: resp.rect };
  }

  async action(
    node: NodeKey,
    generation: number,
    action: SemAction,
    timeoutMs = 5000,
    force = false,
  ): Promise<{ frame_seq: number; detail: string }> {
    this.assertHandshaked();
    const resp = await this.request({
      kind: "action",
      node,
      generation,
      action,
      force,
      timeout_ms: timeoutMs,
    });
    if (resp.kind !== "action_done") throw new Error(`expected ActionDone, got ${resp.kind}`);
    return { frame_seq: resp.frame_seq, detail: resp.detail };
  }

  /** `clock.fast_forward(ms)` etc. (§14 over the wire). */
  async clock(cmd: ClockCmd): Promise<{
    virtual_time_ms: number | null;
    paused: boolean;
    steps: number;
  }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "clock", cmd });
    if (resp.kind !== "clock_state") throw new Error(`expected ClockState, got ${resp.kind}`);
    return { virtual_time_ms: resp.virtual_time_ms, paused: resp.paused, steps: resp.steps };
  }

  /** `mock.install/call/reset_state` (§14 state seams over the wire). */
  async mock(cmd: MockCmd): Promise<{ id: string; detail: string }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "mock", cmd });
    if (resp.kind !== "mock_done") throw new Error(`expected MockDone, got ${resp.kind}`);
    return { id: resp.id, detail: resp.detail };
  }

  /** `video.start(opts)` (§10 over the wire). */
  async videoStart(
    opts: { fps: number; dir: string | null } = { fps: 25, dir: null },
  ): Promise<void> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "video", cmd: { kind: "start", opts } });
    if (resp.kind !== "pong") throw new Error(`expected Pong, got ${resp.kind}`);
  }

  /** `video.stop()` — the finalized artifact summary (§10). */
  async videoStop(): Promise<{ path: string; frames: number; dropped: number; duration_ms: number }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "video", cmd: { kind: "stop" } });
    if (resp.kind !== "video_file") throw new Error(`expected VideoFile, got ${resp.kind}`);
    return { path: resp.path, frames: resp.frames, dropped: resp.dropped, duration_ms: resp.duration_ms };
  }

  /** Convenience: `clock.fast_forward` (the common test verb). */
  async fastForward(ms: number): Promise<void> {
    await this.clock({ kind: "fast_forward", ms });
  }

  /** Convenience: run a registered seam once (§14). */
  async callSeam(id: string, scope = ""): Promise<string> {
    const done = await this.mock({ kind: "call", id, scope });
    return done.detail;
  }

  /** Convenience: reset app state to a fixture scope (§14). */
  async resetState(scope: string): Promise<string> {
    const done = await this.mock({ kind: "reset_state", scope });
    return done.detail;
  }

  /** Close the session cleanly (§7 Bye). */
  async close(): Promise<void> {
    if (!this.handshakeDone) {
      await this.transport.close();
      return;
    }
    try {
      await this.request({ kind: "bye" });
    } finally {
      await this.transport.close();
    }
  }

}

function onceEvent(emitter: unknown, event: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const e = emitter as {
      once: (ev: string, cb: () => void) => void;
      on: (ev: string, cb: (err?: Error) => void) => void;
    };
    e.once(event, () => resolve());
    e.once("error", (err?: Error) => reject(err));
  });
}

async function parseTcp(addr: string): Promise<{ host: string; port: number }> {
  const idx = addr.lastIndexOf(":");
  if (idx < 0) throw new Error(`tcp address needs host:port, got ${addr}`);
  return { host: addr.slice(0, idx), port: Number(addr.slice(idx + 1)) };
}
