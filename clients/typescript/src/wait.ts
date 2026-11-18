/**
 * The client-side wait evaluator (M45 — the m42 seam's TypeScript row,
 * the m42 record's named follow-up: "the TS `waitFor()` rides the same
 * composition and lands on its own milestone"). The vocabulary and the
 * row-set evaluation live here — a pure, transport-blind core — while
 * the wire composition (subscribe → describe-all re-reads → wake on the
 * §7 event channel) rides `GwpClient.waitFor()`.
 *
 * Two boundaries keep this honest (the Rust seam's rules, mirrored):
 *
 * - **No wire change.** The states below are a client-side vocabulary
 *   (never serialized): the wait composes the existing `subscribe`,
 *   `describeAll()` and event domains byte-for-byte as M12/M39 blessed
 *   them. A driver that predates M39 rejects `describeAll()` with the
 *   typed `Unsupported` and the wait surfaces that rejection rather
 *   than degrading (the m41 fail-fast rule).
 * - **The rows are the truth.** Evaluation sees exactly what the M39
 *   `describeAll()` walk answers: a *visible* row is one whose bounds
 *   are present with area > 0 — the wire shape of the §8.3 visible
 *   check. `stable` mirrors the core crate's `StabilityTracker`
 *   exactly: frame-fed only, bounds identical across two consecutive
 *   presented frames, reset on identity change (§8.2) and on a changed
 *   rect. (The evaluator mirrors `crates/guiwright-proto/src/wait.rs`
 *   row-for-row; its state machine is pinned in `test/wait.test.ts`.)
 */

import type { DescribedNode, NodeKey, Selector, TextMatchV1 } from "./schema.ts";

/** Playwright's `waitFor(state)` vocabulary (§16 auto-waiting row), the
 * client-side mirror of the kit's `WaitState` and the Rust client's
 * `wait::WaitState`. Same names, same semantics. */
export type WaitState = "attached" | "visible" | "hidden" | "stable";

/** The four states, in the vocabulary's order (validation at the method). */
export const WAIT_STATES: readonly WaitState[] = ["attached", "visible", "hidden", "stable"];

/** One evaluation step's answer: either the wait is done — carrying the
 * row that satisfied it (a directly usable `action` node, the dump-row
 * rule; `null` for the absence-shaped `hidden`) — or the reason it is
 * still pending, which becomes the timeout error's last-evaluation
 * line. */
export type Eval =
  | { done: true; row: DescribedNode | null }
  | { done: false; reason: string };

/** The §8.3 visible check's wire shape: bounds present *and* of
 * non-empty area. Degenerate rects (zero width or height) are not laid
 * out, whatever the row says. */
function visibleBounds(row: DescribedNode): [number, number, number, number] | null {
  const b = row.bounds;
  if (b === null) return null;
  const [x0, y0, x1, y1] = b;
  return x1 > x0 && y1 > y0 ? b : null;
}

/** Identity of a row's node handle (the §8.2 re-pin rule compares the
 * pinned row across wakes). */
function sameNode(a: NodeKey, b: NodeKey): boolean {
  if (a.kind !== b.kind) return false;
  return a.kind === "accesskit"
    ? a.tree === (b as typeof a).tree && a.node === (b as typeof a).node
    : a.adapter === (b as typeof a).adapter && a.id === (b as typeof a).id;
}

/** The row-set evaluator behind `GwpClient.waitFor()`. Feed it fresh
 * describe-all rows after every wake and it answers the state machine;
 * the wake kind decides which entry point runs (a tree wake re-decides,
 * a presented frame may also confirm `stable`). */
export class WaitTracker {
  private readonly state: WaitState;
  /** The row the `stable` tracker is watching (the set's first visible
   * row, re-pinned whenever the set's head changes identity, §8.2). */
  private pin: NodeKey | null = null;
  /** The last confirmed bounds of the pinned row (the stability
   * baseline — identical bounds across two consecutive presented
   * frames confirm). */
  private held: [number, number, number, number] | null = null;

  /** A tracker for one wait, in the state it is waiting for. */
  constructor(state: WaitState) {
    this.state = state;
  }

  /** Tree-time evaluation: the entry read and every `tree_changed`
   * wake. For `stable` this re-pins (identity change resets the
   * tracker) but never confirms — only presented frames do. */
  evaluate(rows: DescribedNode[]): Eval {
    return this.step(rows, false);
  }

  /** Frame-time evaluation: a `frame_presented` wake. For the positive
   * states this decides exactly like a tree wake; for `stable` it is
   * the only call that can confirm. */
  observeFrame(rows: DescribedNode[]): Eval {
    return this.step(rows, true);
  }

  private step(rows: DescribedNode[], frameWake: boolean): Eval {
    switch (this.state) {
      case "attached":
        return this.attachedStep(rows);
      case "visible":
        return this.visibleStep(rows);
      case "hidden":
        return this.hiddenStep(rows);
      case "stable":
        return this.stableStep(rows, frameWake);
    }
  }

  private attachedStep(rows: DescribedNode[]): Eval {
    const row = rows[0];
    if (row) return { done: true, row };
    return { done: false, reason: "no match yet (the set is empty)" };
  }

  private visibleStep(rows: DescribedNode[]): Eval {
    const seen = rows.find((row) => visibleBounds(row) !== null);
    if (seen) return { done: true, row: seen };
    if (rows.length === 0) {
      return { done: false, reason: "no match yet (the set is empty)" };
    }
    return { done: false, reason: "the set's rows carry no bounds yet" };
  }

  private hiddenStep(rows: DescribedNode[]): Eval {
    const visible = rows.filter((row) => visibleBounds(row) !== null).length;
    if (visible === 0) return { done: true, row: null };
    return { done: false, reason: `${visible} visible row(s) remain` };
  }

  private stableStep(rows: DescribedNode[], frameWake: boolean): Eval {
    let head: { row: DescribedNode; bounds: [number, number, number, number] } | null = null;
    for (const row of rows) {
      const bounds = visibleBounds(row);
      if (bounds !== null) {
        head = { row, bounds };
        break;
      }
    }
    if (head === null) {
      // No laid-out row to watch: the tracker resets with the set.
      this.pin = null;
      this.held = null;
      return {
        done: false,
        reason: "no visible row — the stable check composes the visible precondition (§8.3)",
      };
    }
    const node = head.row.node;
    // Identity change (§8.2): a waiter resets on the node it tracked —
    // the pin moves to the set's current head, frames start over.
    if (this.pin !== null && !sameNode(this.pin, node)) {
      const prev = this.pin;
      this.pin = node;
      this.held = null;
      return {
        done: false,
        reason: `re-pinned — ${nodeKeyDebug(prev)} left the set; the frame tracker resets`,
      };
    }
    this.pin = node;
    if (!frameWake) {
      // A tree wake re-reads the set; the tracker is frame-fed only
      // (the core rule: "evaluated only on frame events").
      return { done: false, reason: "pinned — waiting for presented frames to confirm the rect" };
    }
    // Same pin, presented frame: first identical observation sets the
    // baseline, the next confirms — bounds identical across two
    // consecutive presented frames.
    if (
      this.held !== null &&
      this.held[0] === head.bounds[0] &&
      this.held[1] === head.bounds[1] &&
      this.held[2] === head.bounds[2] &&
      this.held[3] === head.bounds[3]
    ) {
      return { done: true, row: head.row };
    }
    this.held = head.bounds;
    return { done: false, reason: "waiting for the rect to hold on the next frame" };
  }
}

/** The kit's selector render (§8.1 vocabulary): `getByRole(Button, "Save")`
 * — the timeout error's locator line, mirroring the core crate's
 * `Selector::render` (fuzzy matches say so). */
export function renderSelector(selector: Selector): string {
  switch (selector.kind) {
    case "role":
      return selector.name === null
        ? `getByRole(${JSON.stringify(selector.role)})`
        : `getByRole(${JSON.stringify(selector.role)}, ${renderMatch(selector.name)})`;
    case "label":
      return `getByLabel(${renderMatch(selector.match)})`;
    case "text":
      return `getByText(${renderMatch(selector.match)})`;
    case "test_id":
      return `getByTestId(${JSON.stringify(selector.id)})`;
    case "value":
      return `getByValue(${renderMatch(selector.match)})`;
    case "all":
      return selector.selectors.map(renderSelector).join(" >> ");
  }
}

function renderMatch(m: TextMatchV1): string {
  const quoted = JSON.stringify(m.text);
  return m.exact && !m.ignore_case ? quoted : `${quoted} (fuzzy)`;
}

function nodeKeyDebug(node: NodeKey): string {
  return node.kind === "accesskit"
    ? `AccessKit { tree: ${node.tree.slice(0, 8)}…, node: ${node.node} }`
    : `Logical { adapter: ${JSON.stringify(node.adapter)}, id: ${node.id} }`;
}
