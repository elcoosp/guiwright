# M90 — The scroll-into-view vocabulary: `scroll_into_view_if_needed` lands end to end

**Status:** implemented, green end to end. The m19 record's §16
follow-on list — *"`select_option`, `scroll_into_view_if_needed`,
`drag_to`, `wait_for(state)` and the content getters … stay §16
follow-ons — several need vocabulary the wire does not carry yet
(option lists, scrollable ancestors)"* — names this item second; m89
paid the first. This record pays the "scrollable ancestors" vocabulary:
the §16 actions row's scroll-into-view, the Playwright
`locator.scrollIntoViewIfNeeded()` shape, riding the appended
`SemActionV1::ScrollIntoView` variant.

---

## 1. Why now

Playwright's scroll-into-view is the standard "make it visible before
you measure it" step, and the a11y substrate already carries the
honest form: AccessKit's `Action::ScrollIntoView` is a first-class
semantic (freya's accessibility tree handles it natively — §4.3
"Focus/Blur/ScrollIntoView handled natively"; masonry's dialect map
already normalizes the action, `normalize.rs`; egui forwards the
request into the framework's own scroll semantics). What never existed
was the vocabulary above the adapters: no `SemAction` variant, no kit
method, no REPL row, no TS table row. The m89 record's append-only
reasoning applies unchanged — a new variant at the end of the table is
free, the published schema moves with it, and the per-adapter
translation follows the §8.4 precision rule the other actions carry.

## 2. Design: the framework scrolls, we ask

Two decisions carry the milestone:

* **The request is the native action, not a synthesized wheel.**
  `SemAction::ScrollIntoView` carries no coordinates and no delta —
  scroll-into-view is a *semantic* (the framework walks its scrollable
  ancestors), where `SemAction::Scroll { dx, dy }` is the *input* half
  (a wheel event at coordinates, §9). The L1 stacks translate 1:1:
  egui and masonry forward the accesskit request; freya routes it
  through the seam's `handle_action` (the same native path Focus/Blur
  ride). The engine adapters and glues whose dispatch surfaces carry no
  scroll-into-view request answer the typed `Unsupported` with the
  documented reason — the honest per-adapter cap, never a silent
  no-op.
* **The gate rides unchanged.** The action goes through the session's
  `perform_action` path like every locator action: the §8.3
  actionability checks run first, the response carries the frame it
  applied at, the codegen recorder emits the Playwright-shaped line
  (`.scroll_into_view_if_needed();`), and the wire honors
  `force`/`timeout_ms` (M17's act-time gate).

## 3. Surface

* `crates/guiwright-driver/src/adapter.rs` — `SemAction::ScrollIntoView`
  + the `describe` arm (`scroll_into_view_if_needed`).
* `crates/guiwright-proto/src/schema.rs` — the appended
  `SemActionV1::ScrollIntoView` variant (discriminant 11).
* `crates/guiwright-driver/src/{server,session}.rs` — the wire
  translation + the codegen verb.
* `crates/guiwright-kit/src/app.rs` —
  `Locator::scroll_into_view_if_needed()`.
* `crates/guiwright-cli/src/repl.rs` — the `scrollintoview` row.
* Adapters: egui/masonry (native accesskit request), freya (the seam's
  native route), iced engine + floem/gpui/slint glues (typed
  `Unsupported` with the documented dispatch gap).
* `clients/typescript/src/{schema,codec}.ts` — the appended
  discriminant + the `scroll_into_view_if_needed` kind, encode-side.
* No fixture change: the `buttons()` base is the scroll target.

## 4. The evidence

* **Kit (`guiwright-kit/tests/locator_actions.rs`, +1):** the red step —
  `locator.scroll_into_view_if_needed()` records
  `SemAction::ScrollIntoView` at the adapter with the readable detail —
  green after the vocabulary landed.
* **Wire (goldens):** `action_scroll_into_view` joins the golden
  vectors; `GUIWRIGHT_BLESS=1` re-pinned
  `clients/typescript/test/golden.json`, the TS codec test asserts
  byte-identity (the m90 pin joining the m89/M30/M64… pins).
* **REPL (`guiwright-cli/tests/repl_e2e.rs`, +1):** a real UDS session —
  the `scrollintoview` row renders the `ActionDone` detail verbatim and
  the takes-no-arguments refusal is one error line.
* **Example (`guiwright-kit/examples/scroll_into_view.rs`):** the run
  prints the adapter detail, the recorded codegen line and the trace
  path — `cargo run -p guiwright-kit --example scroll_into_view`.
* No budget row: the action rides the existing semantic-action budget
  rows (the m54-family rule — vocabulary, not a new hot path).

## 5. What this does not claim

* No scrollable-ancestor *facts* read on the wire: whether a node is
  clipped or which ancestor scrolls rides the bounds/visibility facts
  the tree reads already carry — the vocabulary answers "make it
  visible", not "why is it not".
* The engine/glue rows stay the documented `Unsupported` caps: the
  iced engine carries no scroll op in its `WidgetOp` table, and the
  floem/gpui/slint dispatch surfaces name the gap honestly (the same
  shape the m89 select rows took).
* The TS client gains no dedicated `scrollIntoViewIfNeeded()` sugar:
  the typed `action(...)` row (M47) speaks the appended variant
  directly — the per-surface sugar is its own milestone when a remote
  TS suite asks (the m64/m65 pattern).
