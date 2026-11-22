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
import { renderSelector, WaitTracker, type WaitState } from "./wait.ts";
import {
  ExpectTracker,
  expectState,
  StateTracker,
  stateWord,
  type ExpectKind,
  type StateKind,
} from "./expect.ts";
import { CountTracker, HAVE_COUNT_STATE, type CountCompare } from "./count.ts";
import {
  type CheckedState,
  type ClockCmd,
  type DescribedIdentityNode,
  type DescribedIdentityStateNode,
  type DescribedNode,
  type EventKind,
  type EventV1,
  type GwpRequest,
  type GwpResponse,
  type GestureCmd,
  type InputEvent,
  type MockCmd,
  type ResolvedMatch,
  type Selector,
  type SemAction,
  type NodeKey,
  type Shot,
  type TextMatchV1,
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

/** The buffered pushed-event queue's bound (§15.6 backpressure) — the
 * Rust side's `EVENT_QUEUE_CAPACITY`: a slow consumer drops the oldest,
 * seq numbers expose the gap. Named here so the client and the driver
 * agree on one number (M45 cleanup; the literal used to be inline). */
export const EVENT_QUEUE_CAPACITY = 64;

/** The client-side wait's typed expiry (M45 — the m42 error shape over
 * TypeScript): the wait is client-side composition, so this is not a
 * wire answer — it names the state, the locator (the kit's render
 * vocabulary) and the last evaluation, the failures-name-names
 * discipline. */
export class GwpWaitTimeout extends Error {
  readonly state: string;
  readonly selector: string;
  readonly timeoutMs: number;
  readonly lastEvaluation: string;

  constructor(state: string, selector: string, timeoutMs: number, lastEvaluation: string) {
    super(
      `wait ${state} timed out after ${timeoutMs} ms — ${selector} (${lastEvaluation})`,
    );
    this.name = "GwpWaitTimeout";
    this.state = state;
    this.selector = selector;
    this.timeoutMs = timeoutMs;
    this.lastEvaluation = lastEvaluation;
  }
}

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
          if (this.eventQueue.length > EVENT_QUEUE_CAPACITY) this.eventQueue.shift();
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

  /** Resolve-all over the wire (§8.2 set read, M30): every current match
   * of the selector, document order — the kit's `count()`/`all()` walk.
   * Non-strict and never waiting: an empty set answers with an empty
   * list. Rows are directly usable as `action` nodes. */
  async resolveAll(selector: Selector): Promise<ResolvedMatch[]> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "resolve_all", selector });
    if (resp.kind !== "resolved_all") throw new Error(`expected ResolvedAll, got ${resp.kind}`);
    return resp.matches;
  }

  /** Hit-test over the wire (§8.3 mechanism as a public read, M32): which
   * node occupies the point — physical px in the tree's bounds space (the
   * same space `resolve` rects and dump rows report). The connection's
   * window scope narrows the answer; empty space answers `null` — the
   * read is total, never an error. The hit is directly usable as an
   * `action` node. */
  async hitTest(x: number, y: number): Promise<ResolvedMatch | null> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "hit_test", x, y });
    if (resp.kind !== "hit_test") throw new Error(`expected HitTest, got ${resp.kind}`);
    return resp.hit;
  }

  /** Node-describe over the wire (§8.1 identity facts, M36 — the m32
   * record's standing gap): one node's role, accessible name, value and
   * bounds for a handle the client already holds (a `resolveAll` row, a
   * hit row, a dump row). Handle-addressed, so no window scope applies;
   * a stale or unknown handle rejects with the typed `error` code the
   * `action` on the same handle would meet. */
  async describe(node: NodeKey, generation: number): Promise<DescribedNode> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "describe", node, generation });
    if (resp.kind !== "described") throw new Error(`expected Described, got ${resp.kind}`);
    return resp;
  }

  /** Describe-all over the wire (§8.2 set read carrying facts, M39 — the
   * m37 record's standing gap): every current match of `selector` with
   * its identity facts — the `resolveAll` walk's rows plus the `describe`
   * facts, one round trip instead of N. Non-strict and never waiting; an
   * empty set answers with an empty list. The connection's window scope
   * narrows the walk exactly like `resolveAll`. */
  async describeAll(selector: Selector): Promise<DescribedNode[]> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "describe_all", selector });
    if (resp.kind !== "described_all") throw new Error(`expected DescribedAll, got ${resp.kind}`);
    return resp.rows;
  }

  /** The describe-all identity read over the wire (m64/m65 — the m62/m63
   * records' reserved wire change, the Rust `ClientConn::
   * describe_all_identity` mirrored): the same walk as `describeAll()`,
   * the rows carrying the accessible description beside the identity
   * four — the fact the M38 identity family's third assertion reads.
   * Appended, not reshaped: the M39 encodings stay byte-pinned, the
   * widening rides its own `describe_all_identity` domain reusing the
   * `describe_all` payload. Snapshot semantics unchanged: no strict
   * mode, never waiting, an empty set answers an empty list. */
  async describeAllIdentity(selector: Selector): Promise<DescribedIdentityNode[]> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "describe_all_identity", selector });
    if (resp.kind !== "described_all_identity")
      throw new Error(`expected DescribedAllIdentity, got ${resp.kind}`);
    return resp.rows;
  }

  /** The single-node identity describe read over the wire (m66/m67 —
   * the m64 record's named standing gap, the Rust `ClientConn::
   * describe_identity` mirrored): one node's facts — the identity four
   * plus the accessible description — for a handle the client already
   * holds (a `resolveAll()` row, a hit row, a dump row: the dump-row
   * rule), one round trip instead of the whole-set walk
   * `describeAllIdentity()` runs. The M36 `describe()` handle pair
   * rides in and the M64 identity row rides out (the m64 payload-reuse
   * rule at handle width); handle-addressed, so no window scope
   * applies; a stale or unknown handle rejects with the typed `error`
   * code the action on the same handle would meet. A driver older than
   * m66 cannot decode the appended request at all (the append-only
   * cost every appended domain has carried since M3), so there is no
   * fallback derivation (the m41 fail-fast rule). */
  async describeIdentity(node: NodeKey, generation: number): Promise<DescribedIdentityNode> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "describe_identity", node, generation });
    if (resp.kind !== "described_identity")
      throw new Error(`expected DescribedIdentity, got ${resp.kind}`);
    return resp;
  }

  /** The single-node identity-state describe read over the wire (m68/m69
   * — the m67 record's named standing gap, the Rust `ClientConn::
   * describe_identity_state` mirrored): one node's facts — the identity
   * four, the accessible description plus the state three (focused /
   * enabled / checked) — for a handle the client already holds (a
   * `resolveAll()` row, a hit row, a dump row: the dump-row rule), one
   * round trip instead of one question per assertion. The M36
   * `describe()` handle pair rides in and the M68 identity-state row
   * rides out (the m64 row-widening rule's third link);
   * handle-addressed, so no window scope applies; a stale or unknown
   * handle rejects with the typed `error` code the action on the same
   * handle would meet. A driver older than m68 cannot decode the
   * appended request at all (the append-only cost every appended domain
   * has carried since M3), so there is no fallback derivation (the m41
   * fail-fast rule). */
  async describeIdentityState(
    node: NodeKey,
    generation: number,
  ): Promise<DescribedIdentityStateNode> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "describe_identity_state", node, generation });
    if (resp.kind !== "described_identity_state")
      throw new Error(`expected DescribedIdentityState, got ${resp.kind}`);
    return resp;
  }

  /** The set-width identity-state describe read over the wire (m70/m71
   * — the m68/m69 records' named standing gap, the Rust `ClientConn::
   * describe_all_identity_state` mirrored): every current match of
   * `selector` with its facts — the identity four, the accessible
   * description plus the state three (focused / enabled / checked) —
   * one round trip instead of one `describeIdentityState()` per row.
   * The M64 `describeAll()` walk payload rides in and the M68
   * identity-state row rides out per match (the m64 row-widening rule's
   * fourth link). Snapshot semantics unchanged: no strict mode, never
   * waiting, an empty set answers an empty list; the connection's
   * window scope narrows the walk exactly like `describeAll()`. A
   * driver older than m70 cannot decode the appended request at all
   * (the append-only cost every appended domain has carried since M3),
   * so there is no fallback derivation (the m41 fail-fast rule). */
  async describeAllIdentityState(selector: Selector): Promise<DescribedIdentityStateNode[]> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "describe_all_identity_state", selector });
    if (resp.kind !== "described_all_identity_state")
      throw new Error(`expected DescribedAllIdentityState, got ${resp.kind}`);
    return resp.rows;
  }

  /** The m29 bulk text read at set width (M41 — the m29 record's remote
   * note: "a remote bulk read would extend the `Resolve` reply exactly
   * as the m26/m22 records name … and stays open until a remote suite
   * asks"; the M39 domain retired the schema question): every match's
   * text content in document order — the accessible name first, the
   * value as the fallback (the source order the text locators match).
   * The kit's `all_text_contents()` read over any door. Rides the M39
   * `describeAll()` round trip — no protocol change, no new vectors; a
   * node with neither fact reads as `""`, keeping positions 1:1 with
   * the walk (the m31 rule). Snapshot semantics: non-strict, never
   * waiting, an empty set answers with an empty list. */
  async allTextContents(selector: Selector): Promise<string[]> {
    const rows = await this.describeAll(selector);
    return rows.map((row) => row.name ?? row.value ?? "");
  }

  /** The m31 value-first bulk read (M41): the `allTextContents` walk
   * with the source order flipped for form semantics — what the form
   * *carries* first, the accessible name as the fallback (the
   * `getByValue` source order). The kit's `all_values()` read over any
   * door, one `describeAll()` round trip, the same snapshot rules. */
  async allValues(selector: Selector): Promise<string[]> {
    const rows = await this.describeAll(selector);
    return rows.map((row) => row.value ?? row.name ?? "");
  }

  /** The client-side wait (M45 — the m42 seam's TypeScript row, the
   * m42/m43 records' named follow-up): Playwright's
   * `locator.waitFor(state)` for remote TS suites. Composes the §7
   * event channel with the M39 describe-all re-reads — subscribe to
   * `tree_changed`/`frame_presented`, evaluate the state against the
   * rows (the `WaitTracker` rules in `wait.ts`), and re-read on every
   * wake until the state holds or `timeoutMs` expires. No wire change:
   * the wait rides existing domains byte-for-byte, the same shape the
   * Rust `ClientConn::wait_for` rides.
   *
   * **Answers.** The positive states resolve with the row that
   * satisfied them — directly usable as an `action()` node (the
   * dump-row rule). `hidden` resolves `null`: absence is the satisfying
   * fact. A spent budget rejects with the typed `GwpWaitTimeout`,
   * naming the state, the locator and the last evaluation.
   *
   * **Subscription ownership.** The wait installs the
   * `tree_changed` + `frame_presented` filter for its duration and
   * drops it on return — on resolution it propagates the unsubscribe
   * result; on expiry it drops the filter best-effort (the Rust
   * `expire()` shape: the timeout outcome is the answer, a failed
   * cleanup surfaces at the next conversation). A connection running
   * its own long-lived subscription (the viewer's live feed) should
   * not hand the same client to a wait: the one-event-consumer rule
   * the §7 channel already states. */
  async waitFor(
    selector: Selector,
    state: WaitState,
    timeoutMs = 5000,
  ): Promise<DescribedNode | null> {
    this.assertHandshaked();
    await this.subscribe(["tree_changed", "frame_presented"]);

    const rendered = renderSelector(selector);
    const deadline = Date.now() + timeoutMs;
    const tracker = new WaitTracker(state);
    // The entry read is a tree-time evaluation; frame wakes re-pin and
    // — only for `stable` — confirm.
    let wake: "tree" | "frame" = "tree";
    try {
      for (;;) {
        const rows = await this.describeAll(selector);
        const answer = wake === "tree" ? tracker.evaluate(rows) : tracker.observeFrame(rows);
        if (answer.done) {
          await this.unsubscribe();
          return answer.row;
        }
        // The event wait: a tree wake re-decides the state, a frame
        // wake may confirm `stable`, `log` records are never wakes, and
        // a spent budget ends the wait — the expiry path below. The
        // deadline races the event read the way the Rust wait's
        // `tokio::select!` does; the m44 read-half contract makes the
        // cancelled read harmless on the wire (the Rust side's buffered
        // `ClientCore`; the TS `FrameReader` always buffered).
        let next: "tree" | "frame" | null = null;
        while (next === null) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) break;
          const event = await this.nextEventWithin(remaining);
          if (event === null) break;
          if (event.kind === "tree_changed") next = "tree";
          if (event.kind === "frame_presented") next = "frame";
          // `log` records: not a wake — keep waiting.
        }
        if (next === null) {
          // Best-effort cleanup (the Rust `expire()` shape), then the
          // typed timeout naming state/locator/last-evaluation.
          await this.unsubscribe().catch(() => undefined);
          throw new GwpWaitTimeout(state, rendered, timeoutMs, answer.reason);
        }
        wake = next;
      }
    } catch (err) {
      // A protocol failure propagates as-is (the m41 fail-fast rule);
      // the wait's subscription window drops on the way out.
      await this.unsubscribe().catch(() => undefined);
      throw err;
    }
  }

  /** The next pushed event or `null` when `ms` elapses first (the m45
   * wait's deadline race): the event waiter is disarmed on expiry so
   * the consumer can keep driving the connection — a later event lands
   * in the buffer instead of resolving an abandoned promise. */
  private nextEventWithin(ms: number): Promise<EventV1 | null> {
    return new Promise<EventV1 | null>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | null = setTimeout(() => {
        timer = null;
        this.eventWaiter = null; // disarm: a spent deadline drops the wait
        resolve(null);
      }, ms);
      this.nextEvent().then(
        (event) => {
          if (timer !== null) {
            clearTimeout(timer);
            timer = null;
            resolve(event);
          }
        },
        (err) => {
          if (timer !== null) {
            clearTimeout(timer);
            timer = null;
            reject(err);
          }
        },
      );
    });
  }

  /** The content assertion, text order (m54 — the kit's
   * `expect(locator).toHaveText(...)` over any door, the Rust
   * `ClientConn::expect_text` mirrored): the m29 source order (the
   * accessible name first, the value as the fallback — the M41
   * derivation reused verbatim) as a waitable assertion. Composes the
   * §7 event channel with the M39 describe-all re-reads exactly like
   * `waitFor()`: subscribe, evaluate the set's head (the
   * `ExpectTracker` rules), and re-read on every wake until the content
   * matches or `timeoutMs` expires. **No wire change**: the assertion
   * rides existing domains byte-for-byte.
   *
   * **Answers.** The satisfied assertion resolves with the row itself —
   * directly usable as an `action()` node (the dump-row rule). A spent
   * budget rejects with the typed `GwpWaitTimeout`, naming the assertion
   * (`have text`), the locator and the last evaluation — expected and
   * found. The subscription window drops on every exit (the `waitFor()`
   * ownership rule): a connection running its own long-lived
   * subscription (the viewer's live feed) should not hand the same
   * client to an assertion. */
  async expectText(
    selector: Selector,
    expected: TextMatchV1,
    timeoutMs = 5000,
  ): Promise<DescribedNode> {
    return this.expectContent(selector, "text", expected, timeoutMs);
  }

  /** The content assertion, value order (m54 — the kit's
   * `expect(locator).toHaveValue(...)` over any door, the Rust
   * `ClientConn::expect_value` mirrored): the m31 source order (the
   * value first, the accessible name as the fallback — the form-dump
   * flip, so the two reads can never silently agree on the wrong datum),
   * the same composition as `expectText()`. */
  async expectValue(
    selector: Selector,
    expected: TextMatchV1,
    timeoutMs = 5000,
  ): Promise<DescribedNode> {
    return this.expectContent(selector, "value", expected, timeoutMs);
  }

  /** The identity assertion, role form (m62/m63 — the kit's
   * `expect(locator).toHaveRole(...)` over any door, the Rust
   * `ClientConn::expect_role` mirrored, M38): the head row's role
   * answers the exact string — the AccessKit debug name the describe
   * rows print (`Button`, `TextInput`), the vocabulary the role
   * locators speak. A structural fact, so the comparison is exact (no
   * matcher, no fuzz). The same composition as `expectText()`: the M45
   * wake loop over the M39 describe-all rows, the set's head pinned,
   * the subscription window owned and dropped. **No wire change**: the
   * assertion rides existing domains byte-for-byte.
   *
   * A spent budget rejects with the typed `GwpWaitTimeout`, naming the
   * assertion (`have role`), the locator and the last evaluation — the
   * role the row carried beside the wanted one. */
  async expectRole(selector: Selector, role: string, timeoutMs = 5000): Promise<DescribedNode> {
    return this.expectContent(selector, "role", role, timeoutMs);
  }

  /** The identity assertion, name form (m62/m63 — the kit's
   * `expect(locator).toHaveAccessibleName(...)` over any door, the Rust
   * `ClientConn::expect_name` mirrored, M38): the label-only accessible
   * name — narrower than `expectText()`'s m29 order (the value never
   * answers), the fact the text row cannot pin. A node with no
   * accessible name stays pending (waitable — the m54 empty-content
   * rule's shape), the timeout naming the missing fact. The same
   * composition; **no wire change**. */
  async expectName(selector: Selector, expected: TextMatchV1, timeoutMs = 5000): Promise<DescribedNode> {
    return this.expectContent(selector, "name", expected, timeoutMs);
  }

  /** The identity assertion, description form (m64/m65 — the kit's
   * `expect(locator).toHaveAccessibleDescription(...)` over any door,
   * the Rust `ClientConn::expect_description` mirrored, M38): the third
   * identity fact, the one the M39 rows left out — the description-only
   * read (the name and the value never answer), a node with no
   * description staying pending (waitable — descriptions arrive with
   * the node's restatement), the timeout naming the missing fact. The
   * same composition over the appended `describeAllIdentity()` read:
   * the wake loop, the set's head pinned, the subscription window owned
   * and dropped. The satisfied answer hands back the full identity row. */
  async expectDescription(
    selector: Selector,
    expected: TextMatchV1,
    timeoutMs = 5000,
  ): Promise<DescribedIdentityNode> {
    return this.expectIdentityContent(selector, "description", expected, timeoutMs);
  }

  /** The identity-row assertion composition (m65): the `expectContent()`
   * loop shape over the appended `describeAllIdentity()` re-reads — one
   * entry read, event-driven re-reads, the subscription window owned
   * and dropped, the typed timeout naming locator/expected/found. */
  private async expectIdentityContent(
    selector: Selector,
    kind: ExpectKind,
    expected: TextMatchV1 | string,
    timeoutMs: number,
  ): Promise<DescribedIdentityNode> {
    this.assertHandshaked();
    await this.subscribe(["tree_changed", "frame_presented"]);

    const rendered = renderSelector(selector);
    const deadline = Date.now() + timeoutMs;
    const tracker = new ExpectTracker(kind, expected);
    try {
      for (;;) {
        const rows = await this.describeAllIdentity(selector);
        const answer = tracker.evaluate(rows);
        if (answer.done) {
          await this.unsubscribe();
          return answer.row as DescribedIdentityNode;
        }
        let woke = false;
        while (!woke) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) break;
          const event = await this.nextEventWithin(remaining);
          if (event === null) break;
          if (event.kind === "tree_changed" || event.kind === "frame_presented") {
            woke = true;
          }
        }
        if (!woke) {
          await this.unsubscribe().catch(() => undefined);
          throw new GwpWaitTimeout(expectState(kind), rendered, timeoutMs, answer.reason);
        }
      }
    } catch (err) {
      await this.unsubscribe().catch(() => undefined);
      throw err;
    }
  }

  /** The shared content- and identity-assertion composition (m54; the
   * m62 identity kinds ride the same loop): the m45 wait's loop shape
   * with the `ExpectTracker` evaluation — one entry read, event-driven
   * re-reads, the subscription window owned and dropped, the typed
   * timeout naming locator/expected/found. Tree and frame wakes both
   * re-decide (content and identity have no `stable`-like frame
   * discipline); `log` records are never wakes. */
  private async expectContent(
    selector: Selector,
    kind: ExpectKind,
    expected: TextMatchV1 | string,
    timeoutMs: number,
  ): Promise<DescribedNode> {
    this.assertHandshaked();
    await this.subscribe(["tree_changed", "frame_presented"]);

    const rendered = renderSelector(selector);
    const deadline = Date.now() + timeoutMs;
    const tracker = new ExpectTracker(kind, expected);
    try {
      for (;;) {
        const rows = await this.describeAll(selector);
        const answer = tracker.evaluate(rows);
        if (answer.done) {
          await this.unsubscribe();
          return answer.row;
        }
        // The event race, the m45 shape: a tree or frame wake re-decides
        // the whole read, `log` records are never wakes, and a spent
        // budget ends the assertion — the expiry path below. The m44
        // read-half contract makes the cancelled read harmless on the
        // wire.
        let woke = false;
        while (!woke) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) break;
          const event = await this.nextEventWithin(remaining);
          if (event === null) break;
          if (event.kind === "tree_changed" || event.kind === "frame_presented") {
            woke = true;
          }
          // `log` records: not a wake — keep waiting.
        }
        if (!woke) {
          // Best-effort cleanup (the Rust `expire()` shape), then the
          // typed timeout naming the assertion, the locator and the
          // last evaluation.
          await this.unsubscribe().catch(() => undefined);
          throw new GwpWaitTimeout(expectState(kind), rendered, timeoutMs, answer.reason);
        }
      }
    } catch (err) {
      // A protocol failure propagates as-is (the m41 fail-fast rule);
      // the assertion's subscription window drops on the way out.
      await this.unsubscribe().catch(() => undefined);
      throw err;
    }
  }

  /** The state assertion, checked form (m75 — the kit's
   * `expect(locator).toBeChecked()` over any door, the Rust
   * `ClientConn::expect_checked` mirrored, M20/M73): the m68 four-way
   * word demanded exactly — the wire vocabulary, never a boolean (a
   * mixed checkbox is not an unchecked one, a not-checkable role
   * staying pending as the total fact). The same composition as
   * `expectText()`: the m54 wake loop over the m70 state-walk re-reads,
   * the set's head pinned, the subscription window owned and dropped.
   * **No wire change**: the assertion rides existing domains
   * byte-for-byte.
   *
   * A spent budget rejects with the typed `GwpWaitTimeout`, naming the
   * assertion (`be checked`), the locator and the last evaluation — the
   * word the row carried beside the wanted one. */
  async expectChecked(
    selector: Selector,
    expected: CheckedState,
    timeoutMs = 5000,
  ): Promise<DescribedIdentityStateNode> {
    return this.expectState(selector, "checked", expected, timeoutMs);
  }

  /** The state assertion, enabled form (m75 — the kit's
   * `expect(locator).toBeEnabled()` / `toBeDisabled()` pair over any
   * door, the Rust `ClientConn::expect_enabled` mirrored): the boolean
   * the row carries (false = the AccessKit `Disabled` flag set),
   * demanded as the one word the pair speaks. The same composition;
   * **no wire change**.
   *
   * A spent budget rejects with the typed `GwpWaitTimeout`, naming the
   * assertion (`be enabled`) and the fact the last evaluation found. */
  async expectEnabled(
    selector: Selector,
    expected: boolean,
    timeoutMs = 5000,
  ): Promise<DescribedIdentityStateNode> {
    return this.expectState(selector, "enabled", expected, timeoutMs);
  }

  /** The state assertion, focused form (m75 — the kit's
   * `expect(locator).toBeFocused()` over any door, the Rust
   * `ClientConn::expect_focused` mirrored): the node IS the tree's
   * focus right now, the focus arriving with the tree update that
   * carries it. No expectation word — the fact itself is the demand,
   * the typed timeout the honest negative (the m73 no-negation rule).
   * The same composition; **no wire change**.
   *
   * A spent budget rejects with the typed `GwpWaitTimeout`, naming the
   * assertion (`be focused`) and the fact the last evaluation found. */
  async expectFocused(
    selector: Selector,
    timeoutMs = 5000,
  ): Promise<DescribedIdentityStateNode> {
    return this.expectState(selector, "focused", undefined, timeoutMs);
  }

  /** The shared state-assertion composition (m75): the
   * `expectIdentityContent()` loop shape over the m70
   * `describeAllIdentityState()` re-reads — one entry read,
   * event-driven re-reads, the subscription window owned and dropped,
   * the typed timeout built from the m75 state word. Tree and frame
   * wakes both re-decide (the state facts have no `stable`-like frame
   * discipline); `log` records are never wakes. An empty set stays
   * pending (waitable — nodes stream in, the m54 empty-content rule's
   * shape). */
  private async expectState(
    selector: Selector,
    kind: StateKind,
    expected: CheckedState | boolean | undefined,
    timeoutMs: number,
  ): Promise<DescribedIdentityStateNode> {
    this.assertHandshaked();
    await this.subscribe(["tree_changed", "frame_presented"]);

    const rendered = renderSelector(selector);
    const deadline = Date.now() + timeoutMs;
    const tracker =
      kind === "checked"
        ? new StateTracker("checked", expected as CheckedState)
        : kind === "enabled"
          ? new StateTracker("enabled", expected as boolean)
          : new StateTracker("focused");
    try {
      for (;;) {
        const rows = await this.describeAllIdentityState(selector);
        const answer = tracker.evaluate(rows);
        if (answer.done) {
          await this.unsubscribe();
          return answer.row;
        }
        // The event race, the m54 shape: a tree or frame wake re-decides
        // the whole read, `log` records are never wakes, and a spent
        // budget ends the assertion — the expiry path below. The m44
        // read-half contract makes the cancelled read harmless on the
        // wire.
        let woke = false;
        while (!woke) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) break;
          const event = await this.nextEventWithin(remaining);
          if (event === null) break;
          if (event.kind === "tree_changed" || event.kind === "frame_presented") {
            woke = true;
          }
          // `log` records: not a wake — keep waiting.
        }
        if (!woke) {
          // Best-effort cleanup (the Rust `expire_state()` shape), then
          // the typed timeout naming the assertion, the locator and the
          // last evaluation.
          await this.unsubscribe().catch(() => undefined);
          throw new GwpWaitTimeout(stateWord(kind), rendered, timeoutMs, answer.reason);
        }
      }
    } catch (err) {
      // A protocol failure propagates as-is (the m41 fail-fast rule);
      // the assertion's subscription window drops on the way out.
      await this.unsubscribe().catch(() => undefined);
      throw err;
    }
  }

  /** The count assertion (m59/m60 — the kit's
   * `expect(locator).toHaveCount(n)` over any door, the Rust
   * `ClientConn::expect_count` mirrored, M22): the whole match set's
   * size as a waitable assertion — the M30 resolve-all round trip's set
   * read (strict mode never consulted), woken by the §7 event channel
   * exactly like `waitFor()` and the m54 content pair. **No wire
   * change**: the assertion rides existing domains byte-for-byte.
   *
   * **Answers.** The satisfied assertion resolves with the found count —
   * the fact the assertion speaks (the Rust seam's m57 summary shape);
   * the set's rows themselves ride `resolveAll()` (M30).
   *
   * **Absence is an answer.** An empty set is found 0 — satisfying
   * `count 0` and every less-than row (Playwright's `toHaveCount(0)`
   * parity), unlike the content pair's empty-set pending.
   *
   * A spent budget rejects with the typed `GwpWaitTimeout`, naming the
   * assertion (`have count`), the locator and the last evaluation — the
   * demanded comparison and the count the last evaluation found. The
   * subscription window drops on every exit (the `waitFor()` ownership
   * rule). */
  async expectCount(selector: Selector, expected: number, timeoutMs = 5000): Promise<number> {
    return this.expectCountWith(selector, { kind: "exact", bound: expected }, timeoutMs);
  }

  /** The one-sided count comparator (m59/m60 — the kit's
   * `expect(locator).toHaveCountGreaterThan(n)` over any door, the Rust
   * `ClientConn::expect_count_greater_than` mirrored, M25): satisfied
   * while *more* than `min` nodes match, the same composition as
   * `expectCount()` with the comparison as the only difference. */
  async expectCountGreaterThan(selector: Selector, min: number, timeoutMs = 5000): Promise<number> {
    return this.expectCountWith(selector, { kind: "greater_than", bound: min }, timeoutMs);
  }

  /** The one-sided count comparator (m59/m60 — the kit's
   * `expect(locator).toHaveCountLessThan(n)` over any door, the Rust
   * `ClientConn::expect_count_less_than` mirrored, M25): satisfied
   * while *fewer* than `max` nodes match, the mirror of
   * `expectCountGreaterThan()`. */
  async expectCountLessThan(selector: Selector, max: number, timeoutMs = 5000): Promise<number> {
    return this.expectCountWith(selector, { kind: "less_than", bound: max }, timeoutMs);
  }

  /** The shared count-assertion composition (m59/m60): the m54 loop
   * shape over the M30 resolve-all round trip — one entry read,
   * event-driven re-reads, the subscription window owned and dropped,
   * the typed timeout naming locator/comparison/found. Tree and frame
   * wakes both re-decide (a count has no `stable`-like frame
   * discipline); `log` records are never wakes. */
  private async expectCountWith(
    selector: Selector,
    comparison: CountCompare,
    timeoutMs: number,
  ): Promise<number> {
    this.assertHandshaked();
    await this.subscribe(["tree_changed", "frame_presented"]);
    const rendered = renderSelector(selector);
    const deadline = Date.now() + timeoutMs;
    const tracker = new CountTracker(comparison);
    try {
      for (;;) {
        const matches = await this.resolveAll(selector);
        const answer = tracker.evaluate(matches);
        if (answer.done) {
          await this.unsubscribe();
          return answer.found;
        }
        // The event race, the m45 shape: a tree or frame wake re-decides
        // the whole read, `log` records are never wakes, and a spent
        // budget ends the assertion — the expiry path below. The m44
        // read-half contract makes the cancelled read harmless on the
        // wire.
        let woke = false;
        while (!woke) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) break;
          const event = await this.nextEventWithin(remaining);
          if (event === null) break;
          if (event.kind === "tree_changed" || event.kind === "frame_presented") {
            woke = true;
          }
          // `log` records: not a wake — keep waiting.
        }
        if (!woke) {
          // Best-effort cleanup (the Rust `expire()` shape), then the
          // typed timeout naming the assertion, the locator and the
          // last evaluation.
          await this.unsubscribe().catch(() => undefined);
          throw new GwpWaitTimeout(HAVE_COUNT_STATE, rendered, timeoutMs, answer.reason);
        }
      }
    } catch (err) {
      // A protocol failure propagates as-is (the m41 fail-fast rule);
      // the assertion's subscription window drops on the way out.
      await this.unsubscribe().catch(() => undefined);
      throw err;
    }
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

  /**
   * `inspect.start_recording` — start the remote interaction → kit-code
   * recorder (§13, M4): every applied Action on the session is recorded
   * against its minimal stable selector until `inspectStopRecording()`.
   * Refuses while a recording is already active (clean `GwpRejected`).
   */
  async inspectStartRecording(): Promise<void> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "inspect", cmd: { kind: "start_recording" } });
    if (resp.kind !== "pong") throw new Error(`expected Pong, got ${resp.kind}`);
  }

  /**
   * `inspect.stop_recording` — stop the recorder and fetch the generated
   * kit code (§13): the §13 cost model's lines for every recorded action,
   * plus the strict-mode warnings collected while recording (ambiguous
   * selectors become warnings, not broken tests). Refuses when nothing
   * is being recorded (clean `GwpRejected`).
   */
  async inspectStopRecording(): Promise<{ code: string; warnings: string[] }> {
    this.assertHandshaked();
    const resp = await this.request({ kind: "inspect", cmd: { kind: "stop_recording" } });
    if (resp.kind !== "codegen") throw new Error(`expected Codegen, got ${resp.kind}`);
    return { code: resp.code, warnings: resp.warnings };
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
