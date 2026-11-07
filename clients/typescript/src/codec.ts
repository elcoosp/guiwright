/**
 * GWP codec: `GwpRequest`/`GwpResponse` ⇄ postcard bytes, exactly the
 * envelope the Rust `guiwright-proto` produces (golden-pinned by
 * `test/codec.test.ts` against vectors captured from Rust).
 */

import {
  type CapsV1,
  ClockV1,
  type ClockCmd,
  ErrorCode,
  GwpRequest,
  GwpResponse,
  type GwpResponse as GwpResponseT,
  type InputEvent,
  InputEventV1,
  InspectV1,
  MockV1,
  type MockCmd,
  type NodeKey,
  NodeKeyV1,
  RequestV1,
  type Selector,
  SelectorV1,
  SemActionV1,
  type SemAction,
  type TextMatchV1,
  type VideoCmd,
  VideoV1,
  type WindowCmd,
  type WindowInfoV1,
  WindowV1,
  type GestureCmd,
  GestureV1,
} from "./schema.ts";
import { Decoder, Encoder } from "./postcard.ts";

// --- Encoding ----------------------------------------------------------------

/** Bytes ⇄ hex (the JSON form of the `[u8; 16]` AccessKit TreeId). */
export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.length % 2 === 0 ? hex : `0${hex}`;
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export function encodeRequest(req: GwpRequest): Uint8Array {
  const enc = new Encoder();
  enc.u32(GwpRequest.V1); // envelope
  switch (req.kind) {
    case "hello":
      enc.u32(RequestV1.Hello).u32(req.proto_version).str(req.agent);
      break;
    case "ping":
      enc.u32(RequestV1.Ping);
      break;
    case "tree_snapshot":
      enc.u32(RequestV1.TreeSnapshot);
      break;
    case "resolve":
      enc.u32(RequestV1.Resolve);
      encodeSelector(enc, req.selector);
      enc.u64(req.timeout_ms);
      break;
    case "action":
      enc.u32(RequestV1.Action);
      encodeNodeKey(enc, req.node);
      enc.u32(req.generation);
      encodeSemAction(enc, req.action);
      enc.bool(req.force);
      enc.u64(req.timeout_ms);
      break;
    case "input":
      enc.u32(RequestV1.Input).seqLen(req.events.length);
      for (const ev of req.events) encodeInputEvent(enc, ev);
      break;
    case "clock":
      enc.u32(RequestV1.Clock);
      encodeClockCmd(enc, req.cmd);
      break;
    case "mock":
      enc.u32(RequestV1.Mock);
      encodeMockCmd(enc, req.cmd);
      break;
    case "inspect":
      enc.u32(RequestV1.Inspect);
      switch (req.cmd.kind) {
        case "start_recording":
          enc.u32(InspectV1.StartRecording);
          break;
        case "stop_recording":
          enc.u32(InspectV1.StopRecording);
          break;
      }
      break;
    case "video":
      enc.u32(RequestV1.Video);
      encodeVideoCmd(enc, req.cmd);
      break;
    case "window":
      enc.u32(RequestV1.Window);
      encodeWindowCmd(enc, req.cmd);
      break;
    case "gesture":
      enc.u32(RequestV1.Gesture);
      encodeGestureCmd(enc, req.cmd);
      break;
    case "bye":
      enc.u32(RequestV1.Bye);
      break;
  }
  return enc.finish();
}

function encodeSelector(enc: Encoder, sel: Selector): void {
  switch (sel.kind) {
    case "role":
      enc.u32(SelectorV1.Role).str(sel.role);
      encodeTextMatchOption(enc, sel.name);
      break;
    case "label":
      enc.u32(SelectorV1.Label);
      encodeTextMatch(enc, sel.match);
      break;
    case "text":
      enc.u32(SelectorV1.Text);
      encodeTextMatch(enc, sel.match);
      break;
    case "test_id":
      enc.u32(SelectorV1.TestId).str(sel.id);
      break;
    case "value":
      enc.u32(SelectorV1.Value);
      encodeTextMatch(enc, sel.match);
      break;
    case "all":
      enc.u32(SelectorV1.All).seqLen(sel.selectors.length);
      for (const s of sel.selectors) encodeSelector(enc, s);
      break;
  }
}

function encodeTextMatch(enc: Encoder, m: TextMatchV1): void {
  enc.str(m.text).bool(m.exact).bool(m.ignore_case);
}

function encodeTextMatchOption(enc: Encoder, m: TextMatchV1 | null): void {
  enc.optionTag(m !== null);
  if (m !== null) encodeTextMatch(enc, m);
}

function encodeNodeKey(enc: Encoder, key: NodeKey): void {
  switch (key.kind) {
    case "accesskit":
      enc
        .u32(NodeKeyV1.AccessKit)
        .raw(hexToBytes(key.tree))
        .u64(key.node);
      break;
    case "logical":
      enc.u32(NodeKeyV1.Logical).str(key.adapter).u64(key.id);
      break;
  }
}

function encodeSemAction(enc: Encoder, action: SemAction): void {
  switch (action.kind) {
    case "click":
      enc.u32(SemActionV1.Click);
      break;
    case "double_click":
      enc.u32(SemActionV1.DoubleClick);
      break;
    case "set_value":
      enc.u32(SemActionV1.SetValue).str(action.text);
      break;
    case "focus":
      enc.u32(SemActionV1.Focus);
      break;
    case "blur":
      enc.u32(SemActionV1.Blur);
      break;
    case "hover":
      enc.u32(SemActionV1.Hover);
      break;
    case "press":
      enc.u32(SemActionV1.Press).str(action.key).seqLen(action.modifiers.length);
      for (const m of action.modifiers) enc.str(m);
      break;
    case "scroll":
      enc.u32(SemActionV1.Scroll).f64(action.dx).f64(action.dy);
      break;
    case "check":
      enc.u32(SemActionV1.Check);
      break;
    case "uncheck":
      enc.u32(SemActionV1.Uncheck);
      break;
  }
}

function encodeInputEvent(enc: Encoder, ev: InputEvent): void {
  switch (ev.kind) {
    case "pointer_down":
      enc.u32(InputEventV1.PointerDown).f64(ev.x).f64(ev.y).u32(ev.button);
      break;
    case "pointer_up":
      enc.u32(InputEventV1.PointerUp).f64(ev.x).f64(ev.y).u32(ev.button);
      break;
    case "pointer_move":
      enc.u32(InputEventV1.PointerMove).f64(ev.x).f64(ev.y);
      break;
    case "wheel":
      enc.u32(InputEventV1.Wheel).f64(ev.dx).f64(ev.dy);
      break;
    case "key_down":
      enc.u32(InputEventV1.KeyDown).str(ev.key).seqLen(ev.modifiers.length);
      for (const m of ev.modifiers) enc.str(m);
      break;
    case "key_up":
      enc.u32(InputEventV1.KeyUp).str(ev.key).seqLen(ev.modifiers.length);
      for (const m of ev.modifiers) enc.str(m);
      break;
    case "text_input":
      enc.u32(InputEventV1.TextInput).str(ev.text);
      break;
  }
}

function encodeClockCmd(enc: Encoder, cmd: ClockCmd): void {
  switch (cmd.kind) {
    case "install_virtual":
      enc.u32(ClockV1.InstallVirtual);
      break;
    case "fast_forward":
      enc.u32(ClockV1.FastForward).u64(cmd.ms);
      break;
    case "run_for":
      enc.u32(ClockV1.RunFor).u64(cmd.ms);
      break;
    case "pause":
      enc.u32(ClockV1.Pause);
      break;
    case "resume":
      enc.u32(ClockV1.Resume);
      break;
    case "set_time":
      enc.u32(ClockV1.SetTime).u64(cmd.ms);
      break;
    case "state":
      enc.u32(ClockV1.State);
      break;
  }
}

function encodeMockCmd(enc: Encoder, cmd: MockCmd): void {
  switch (cmd.kind) {
    case "install_seam":
      enc.u32(MockV1.InstallSeam).str(cmd.id);
      enc.optionTag(cmd.response !== null);
      if (cmd.response !== null) enc.str(cmd.response);
      break;
    case "clear":
      enc.u32(MockV1.Clear).str(cmd.id);
      break;
    case "reset_state":
      enc.u32(MockV1.ResetState).str(cmd.scope);
      break;
    case "call":
      enc.u32(MockV1.Call).str(cmd.id).str(cmd.scope);
      break;
  }
}

function encodeVideoCmd(enc: Encoder, cmd: VideoCmd): void {
  switch (cmd.kind) {
    case "start":
      enc.u32(VideoV1.Start).u32(cmd.opts.fps);
      enc.optionTag(cmd.opts.dir !== null);
      if (cmd.opts.dir !== null) enc.str(cmd.opts.dir);
      break;
    case "stop":
      enc.u32(VideoV1.Stop);
      break;
  }
}

function encodeWindowCmd(enc: Encoder, cmd: WindowCmd): void {
  switch (cmd.kind) {
    case "list":
      enc.u32(WindowV1.List);
      break;
    case "use":
      enc.u32(WindowV1.Use).str(cmd.handle);
      break;
  }
}

function encodeGestureCmd(enc: Encoder, cmd: GestureCmd): void {
  switch (cmd.kind) {
    case "click":
      enc.u32(GestureV1.Click).f64(cmd.x).f64(cmd.y).u32(cmd.button).u32(cmd.count);
      break;
    case "drag":
      enc
        .u32(GestureV1.Drag)
        .f64(cmd.from_x)
        .f64(cmd.from_y)
        .f64(cmd.to_x)
        .f64(cmd.to_y)
        .u32(cmd.button);
      break;
    case "wheel":
      enc.u32(GestureV1.Wheel).f64(cmd.dx).f64(cmd.dy);
      break;
    case "key":
      enc.u32(GestureV1.Key).str(cmd.key).seqLen(cmd.modifiers.length);
      for (const m of cmd.modifiers) enc.str(m);
      break;
    case "type":
      enc.u32(GestureV1.Type).str(cmd.text);
      break;
  }
}

// --- Decoding ----------------------------------------------------------------

export function decodeResponse(bytes: Uint8Array): GwpResponseT {
  const dec = new Decoder(bytes);
  const envelope = dec.u32();
  if (envelope !== GwpResponse.V1) {
    throw new Error(`unknown response envelope ${envelope}`);
  }
  const variant = dec.u32();
  return decodeResponseInner(variant, dec);
}

function decodeResponseInner(variant: number, dec: Decoder): GwpResponseT {
  switch (variant) {
    case 0: {
      const proto_version = dec.u32();
      const session_id = dec.str();
      const tree_seq = dec.u64n();
      const frame_seq = dec.u64n();
      const adapter = decodeAdapterInfo(dec);
      return { kind: "welcome", proto_version, session_id, tree_seq, frame_seq, adapter };
    }
    case 1:
      return { kind: "pong" };
    case 2: {
      const tree_seq = dec.u64n();
      const node_count = dec.u64n();
      return { kind: "tree_snapshot", tree_seq, node_count };
    }
    case 3: {
      const node = decodeNodeKey(dec);
      const generation = dec.u32();
      const rect = dec.optionTag()
        ? ([dec.f64(), dec.f64(), dec.f64(), dec.f64()] as [number, number, number, number])
        : null;
      return { kind: "resolved", node, generation, rect };
    }
    case 4: {
      const frame_seq = dec.u64n();
      const detail = dec.str();
      return { kind: "action_done", frame_seq, detail };
    }
    case 5: {
      const frame_seq = dec.u64n();
      return { kind: "input_queued", frame_seq };
    }
    case 6: {
      const virtual_time_ms = dec.optionTag() ? dec.u64n() : null;
      const paused = dec.bool();
      const steps = dec.u64n();
      return { kind: "clock_state", virtual_time_ms, paused, steps };
    }
    case 7: {
      const id = dec.str();
      const detail = dec.str();
      return { kind: "mock_done", id, detail };
    }
    case 8: {
      const code = dec.str();
      const warnings: string[] = [];
      const n = dec.varint();
      for (let i = 0; i < n; i += 1) warnings.push(dec.str());
      return { kind: "codegen", code, warnings };
    }
    case 9: {
      const code = dec.u32();
      const message = dec.str();
      return { kind: "error", code, message };
    }
    case 10:
      return { kind: "bye" };
    case 11: {
      const path = dec.str();
      const frames = dec.u64n();
      const dropped = dec.u64n();
      const duration_ms = dec.u64n();
      return { kind: "video_file", path, frames, dropped, duration_ms };
    }
    case 12: {
      const n = dec.varint();
      const windows: WindowInfoV1[] = [];
      for (let i = 0; i < n; i += 1) {
        windows.push({ handle: dec.str(), title: dec.str(), focused: dec.bool() });
      }
      return { kind: "windows", windows };
    }
    case 13: {
      const handle = dec.str();
      return { kind: "window_used", handle };
    }
    case 14: {
      const frame_seq = dec.u64n();
      const detail = dec.str();
      return { kind: "gesture_done", frame_seq, detail };
    }
    default:
      throw new Error(`unknown response variant ${variant}`);
  }
}

function decodeAdapterInfo(dec: Decoder): {
  name: string;
  framework: string;
  framework_version: string;
  caps: CapsV1;
} {
  const name = dec.str();
  const framework = dec.str();
  const framework_version = dec.str();
  const caps = {
    tree_update: dec.bool(),
    logical_snapshot: dec.bool(),
    input: dec.bool(),
    last_frame: dec.bool(),
    render_frame: dec.bool(),
    clock: dec.bool(),
  };
  return { name, framework, framework_version, caps };
}

function decodeNodeKey(dec: Decoder): NodeKey {
  const variant = dec.u32();
  switch (variant) {
    case NodeKeyV1.AccessKit: {
      // The 16-byte TreeId renders as a hex string in JSON (bytes have no
      // JSON form); encodeNodeKey accepts both and re-hexes strings.
      const tree = toHex(dec.raw(16));
      const node = dec.u64n();
      return { kind: "accesskit", tree, node };
    }
    case NodeKeyV1.Logical: {
      const adapter = dec.str();
      const id = dec.u64n();
      return { kind: "logical", adapter, id };
    }
    default:
      throw new Error(`unknown NodeKey variant ${variant}`);
  }
}

/** Map a decoded error response onto a typed rejection (§7). */
export function isRejected(resp: GwpResponseT, code?: keyof typeof ErrorCode): boolean {
  return resp.kind === "error" && (code === undefined || resp.code === ErrorCode[code]);
}

export { ErrorCode };
