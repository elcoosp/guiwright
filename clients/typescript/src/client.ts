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
  type EventKind,
  type EventV1,
  type GwpRequest,
  type GwpResponse,
  type GestureCmd,
  type InputEvent,
  type MockCmd,
  type Selector,
  type SemAction,
  type NodeKey,
  type Shot,
  type TreeDump,
  type VideoCmd,
  type Welcome,
  type WindowInfoV1,
  PointerButtonV1,
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
  8: "UnknownWindow",
};

export class GwpClient {
  private pending: ((resp: GwpResponse) => void) | null = null;
  private reader = new FrameReader();
  private handshakeDone = false;
  /** Buffered pushed events (§7, M12): arrived between responses. */
  private eventQueue: EventV1[] = [];
  /** The event waiter, armed by `nextEvent()` while nothing is queued. */
  private eventWaiter: ((event: EventV1) => void) | null = null;
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
      // Pushed events (§7, M12) route to the event queue — never to the
      // pending-response slot, which belongs to the request the client
      // actually sent. The queue is bounded at the driver's §15.6
      // capacity; a slow consumer drops the oldest (seq numbers expose
      // the gap, same as the Rust client's buffer).
      if (resp.kind === "event") {
        if (this.eventWaiter) {
          const waiter = this.eventWaiter;
          this.eventWaiter = null;
          waiter(resp.event);
        } else {
          this.eventQueue.push(resp.event);
          if (this.eventQueue.length > 64) this.eventQueue.shift();
        }
        continue;
      }
      const resolve = this.pending;
      this.pending = null;
      if (resolve) resolve(resp);
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

  /** `window.list` — the session's window targets (§16 v0.4, M6). */
  async windowList(): Promise<WindowInfoV1[]> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "window", cmd: { kind: "list" } });
    if (resp.kind !== "windows") throw new Error(`expected Windows, got ${resp.kind}`);
    return resp.windows;
  }

  /**
   * `window.use(handle)` — aim THIS connection at a window target (§16 v0.4):
   * subsequent resolve calls only see that window's subtree. Other
   * connections keep their own target (per-connection state).
   */
  async windowUse(handle: string): Promise<void> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "window", cmd: { kind: "use", handle } });
    if (resp.kind !== "window_used") throw new Error(`expected WindowUsed, got ${resp.kind}`);
  }

  // --- inspector overlay (§13, M8) --------------------------------------------

  /**
   * `inspect.overlay(enable)` — start/stop the live overlay session (§13):
   * the adapter composites hover outlines + interactive-role badges from
   * the tree indexes at present time, refreshed every frame while it is
   * on. Adapters without an overlay surface reject with `Unsupported`.
   */
  async inspectOverlay(enable: boolean): Promise<void> {
    this.assertHandshaked();
    const cmd = enable ? { kind: "enable_overlay" as const } : { kind: "disable_overlay" as const };
    const resp = await this.request({ kind: "inspect", cmd });
    if (resp.kind !== "pong") throw new Error(`expected Pong, got ${resp.kind}`);
  }

  /**
   * `inspect.highlight(node)` — aim the hover outline at a resolved node
   * handle (§13); `null` clears the outline and keeps the overlay
   * session. Resolve first (`resolve()`), then highlight the returned
   * handle — the same conversation the REPL's `inspect <get-args…>` runs.
   */
  async inspectHighlight(node: NodeKey | null): Promise<void> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "inspect", cmd: { kind: "highlight", node } });
    if (resp.kind !== "pong") throw new Error(`expected Pong, got ${resp.kind}`);
  }

  // --- raw input & gestures (§9, M7) -----------------------------------------

  /**
   * Inject raw primitive events yourself (§9 over the wire). Coordinates
   * are logical px relative to this connection's active window target;
   * the batch applies inside the next frame.
   */
  async input(events: InputEvent[]): Promise<{ frame_seq: number }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "input", events });
    if (resp.kind !== "input_queued") throw new Error(`expected InputQueued, got ${resp.kind}`);
    return { frame_seq: resp.frame_seq };
  }

  /**
   * Send a gesture from the §9 vocabulary — the driver synthesizes the
   * raw batch from its adapter's pacing table and applies it in one
   * frame. Prefer the [`mouse`]/[`keyboard`] helpers.
   */
  async gesture(cmd: GestureCmd): Promise<{ frame_seq: number; detail: string }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "gesture", cmd });
    if (resp.kind !== "gesture_done") throw new Error(`expected GestureDone, got ${resp.kind}`);
    return { frame_seq: resp.frame_seq, detail: resp.detail };
  }

  // --- screenshots (§7 render domain, M10) ------------------------------------

  /**
   * `screenshot window` — pull the full window frame as PNG bytes (§10
   * render-on-demand path). The reply carries the session frame the
   * pixels came from and their dimensions; adapters without a pixel path
   * reject with `Unsupported`.
   */
  async screenshotWindow(): Promise<Shot> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "screenshot", cmd: { kind: "window" } });
    if (resp.kind !== "shot") throw new Error(`expected Shot, got ${resp.kind}`);
    return { frame_seq: resp.frame_seq, width: resp.width, height: resp.height, png: resp.png };
  }

  /**
   * `screenshot element` — resolve `selector` (auto-waiting `Attached`
   * within `timeoutMs`, §8.2 over the wire) and crop the window frame by
   * the resolved node's physical bounds — never a re-render (§10 element
   * rule). Nodes without bounds reject with an honest error.
   */
  async screenshotElement(selector: Selector, timeoutMs = 5000): Promise<Shot> {
    this.assertHandshaked();
    const resp = await this.request({
      kind: "screenshot",
      cmd: { kind: "element", selector, timeout_ms: timeoutMs },
    });
    if (resp.kind !== "shot") throw new Error(`expected Shot, got ${resp.kind}`);
    return { frame_seq: resp.frame_seq, width: resp.width, height: resp.height, png: resp.png };
  }

  /**
   * `snapshot full` — read the session's whole logical tree as data (§7
   * tree-dump domain, M11): depth-first per window root, `children`
   * counts rebuilding the hierarchy, the focused node included. Session-
   * wide by design — the connection's window target narrows locators,
   * not whole-tree reads.
   */
  async treeDump(): Promise<TreeDump> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "snapshot", cmd: { kind: "full" } });
    if (resp.kind !== "tree_dump") throw new Error(`expected TreeDump, got ${resp.kind}`);
    return { tree_seq: resp.tree_seq, node_count: resp.node_count, focus: resp.focus, nodes: resp.nodes };
  }

  /**
   * `snapshot under <selector>` — read one subtree as data (§7 tree-dump
   * domain, M13): resolve the selector (auto-wait `Attached` within
   * `timeout_ms`, honoring the connection's window target) and dump only
   * the depth-first walk rooted at the match.
   */
  async treeDumpUnder(selector: Selector, timeoutMs = 5000): Promise<TreeDump> {
    this.assertHandshaked();
    const resp = await this.request({
      kind: "snapshot",
      cmd: { kind: "filtered", selector, timeout_ms: timeoutMs },
    });
    if (resp.kind !== "tree_dump") throw new Error(`expected TreeDump, got ${resp.kind}`);
    return { tree_seq: resp.tree_seq, node_count: resp.node_count, focus: resp.focus, nodes: resp.nodes };
  }

  /**
   * Subscribe this connection to the named event kinds (§7 event
   * channel, M12 + the log-tail milestone): `Event` frames flow between
   * responses while the app advances; read them with `nextEvent()`.
   * Re-subscribing replaces the filter; the ack echoes the kinds the
   * driver honored (all-or-nothing: naming `log` against a session
   * spawned without its log tail rejects the whole request with
   * `Unsupported`).
   */
  async subscribe(kinds: EventKind[]): Promise<EventKind[]> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "subscribe", cmd: { kind: "on", kinds } });
    if (resp.kind !== "subscribed") throw new Error(`expected Subscribed, got ${resp.kind}`);
    return resp.kinds;
  }

  /** Drop this connection's event subscription (§7, M12). */
  async unsubscribe(): Promise<void> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "subscribe", cmd: { kind: "off" } });
    if (resp.kind !== "subscribed" || resp.kinds.length !== 0) {
      throw new Error(`expected an empty Subscribed ack, got ${resp.kind}`);
    }
  }

  /**
   * The next pushed event (§7, M12): from the buffer of events that
   * interleaved earlier responses, or off the wire. Blocks until one
   * arrives — pair it with the subscriber's own pacing, never a hot path.
   */
  async nextEvent(): Promise<EventV1> {
    const buffered = this.eventQueue.shift();
    if (buffered) return buffered;
    if (this.eventWaiter) {
      throw new Error("nextEvent() is already waiting — one event consumer at a time");
    }
    return new Promise<EventV1>((resolve) => {
      this.eventWaiter = resolve;
    });
  }

  /** The optional button name on the mouse helpers (default primary). */
  static buttonIndex(name?: "left" | "right" | "middle"): number {
    switch (name) {
      case "right":
        return PointerButtonV1.Secondary;
      case "middle":
        return PointerButtonV1.Middle;
      default:
        return PointerButtonV1.Primary;
    }
  }

  /**
   * Playwright-shaped pointer surface (§9, M7): window-relative logical
   * px, synthesized driver-side (click = move→down→up ×clickCount; drag
   * = ratchet-interpolated moves; wheel = one event).
   */
  readonly mouse = {
    click: (
      x: number,
      y: number,
      opts?: { button?: "left" | "right" | "middle"; clickCount?: number },
    ): Promise<{ frame_seq: number; detail: string }> =>
      this.gesture({
        kind: "click",
        x,
        y,
        button: GwpClient.buttonIndex(opts?.button),
        count: opts?.clickCount ?? 1,
      }),
    dblclick: (
      x: number,
      y: number,
      opts?: { button?: "left" | "right" | "middle" },
    ): Promise<{ frame_seq: number; detail: string }> =>
      this.gesture({
        kind: "click",
        x,
        y,
        button: GwpClient.buttonIndex(opts?.button),
        count: 2,
      }),
    drag: (
      fromX: number,
      fromY: number,
      toX: number,
      toY: number,
      opts?: { button?: "left" | "right" | "middle" },
    ): Promise<{ frame_seq: number; detail: string }> =>
      this.gesture({
        kind: "drag",
        from_x: fromX,
        from_y: fromY,
        to_x: toX,
        to_y: toY,
        button: GwpClient.buttonIndex(opts?.button),
      }),
    wheel: (dx: number, dy: number): Promise<{ frame_seq: number; detail: string }> =>
      this.gesture({ kind: "wheel", dx, dy }),
  };

  /** Playwright-shaped keyboard surface (§9, M7). */
  readonly keyboard = {
    press: (
      key: string,
      modifiers: string[] = [],
    ): Promise<{ frame_seq: number; detail: string }> =>
      this.gesture({ kind: "key", key, modifiers }),
    type: (text: string): Promise<{ frame_seq: number; detail: string }> =>
      this.gesture({ kind: "type", text }),
  };

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
  // The TCP socket is a node EventEmitter (`once`); the WHATWG WebSocket
  // is an EventTarget (`addEventListener`) — since M15 `connectWs` rides
  // this helper too, so it speaks both shapes.
  const e = emitter as {
    once?: (ev: string, cb: (err?: Error) => void) => void;
    addEventListener?: (
      ev: string,
      cb: (detail?: unknown) => void,
      opts?: { once?: boolean },
    ) => void;
  };
  return new Promise((resolve, reject) => {
    if (typeof e.once === "function") {
      e.once(event, () => resolve());
      e.once("error", (err?: Error) => reject(err));
      return;
    }
    if (typeof e.addEventListener === "function") {
      e.addEventListener(event, () => resolve(), { once: true });
      e.addEventListener(
        "error",
        (detail?: unknown) => reject(detail as Error),
        { once: true },
      );
      return;
    }
    reject(new Error(`not an event emitter: ${event} never fires`));
  });
}

async function parseTcp(addr: string): Promise<{ host: string; port: number }> {
  const idx = addr.lastIndexOf(":");
  if (idx < 0) throw new Error(`tcp address needs host:port, got ${addr}`);
  return { host: addr.slice(0, idx), port: Number(addr.slice(idx + 1)) };
}
