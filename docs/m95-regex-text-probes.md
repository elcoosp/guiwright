# M95 — The regex text probes: the `TextPattern` enum opens

**Status:** implemented, green end to end. The m34 record's second
standing note — *"the enum stays open until the codegen milestone
asks"* — carried through four honest lists (m91, m92, m93, m94), each
re-affirming *"this record does not cash it"* while naming the same
prompt-side gap: *"the REPL's `has_text` word speaks the fuzzy
`TextMatch` default only"* (m93), *"the chain segments' text probes
speak the fuzzy `TextMatch` default only"* (m94). The codegen
milestones shipped without asking; the prompt-side debugging session
the m93/m94 records described is the consumer that arrived, and this
record pays the note: the `TextPattern` enum opens, and a regex probe
resolves through the same pipeline every string probe rides.

---

## 1. Why now

The §8.1 selector model has matched text by Playwright's string rules
since M0 — substring case-insensitive by default, `exact` as the
pin — and every milestone since kept the enum `Str`-shaped on the
recorded rationale: a wire change (the appender's own discipline) needs
a consumer, and none had arrived. The m93/m94 records narrowed the gap
to a concrete shape: the *prompt* — where a debugging session aims
"the Save button" — cannot say `/save|cancel/i`, the `has_text` word
cannot narrow by pattern, and the m94 chain segments inherit the same
silence. Every other surface the plan traces composes text probes; the
enum was the last closed one.

## 2. Design: the enum opens, the wire widens at variant width, the prompt speaks the literal

Three decisions carry the milestone:

* **The core arm owns the semantics.** `TextPattern::Regex(String)`
  carries the pattern **source** only — compiled per match by the
  `TextMatch::matches` arm, never a cache in the enum (the derive's
  `PartialEq`/`Eq` stay on the source string). The JS convention the
  regex family pins: partial by default (a regex `test`, Playwright's
  `getByText(/re/)` shape), case-sensitive until the pattern's inline
  flags (`(?i)save`) — the `ignore_case` field never applies to a
  regex; `exact` anchors (`^(?:…)$`, the full-match rule composed from
  a partial source). The candidate is whitespace-normalized (the
  Playwright text rule), the pattern is not. An uncompilable source
  answers the honest `false` — the boundary callers validate
  ([`TextMatch::try_regex`], the REPL's `/…/` parse), the resolver
  never panics (the fuzz rule).
* **The widening is its own variants — the M64 rule at selector
  width.** Five appended `SelectorV1` rows (`TextRegex` 8, `LabelRegex`
  9, `ValueRegex` 10, `RoleNameRegex` 11, `HasTextRegex` 12), the
  pattern source as the payload. The string arms' encodings are
  byte-pinned unchanged; an old client answers an unknown variant with
  a clean decode error. The regex `has_text` rides `HasTextRegex` as
  its own row — a `nth`/`has_not` beside it composes that row as the
  *source* of a string-`Filter` (the fold's own composition, the
  payload never reshaped; the decode collapses the identity wrapper so
  the core shape the kit built comes back). The wire rows carry the
  pattern only — the case rule rides the pattern's inline flags, one
  wire shape for every language's flag syntax.
* **One seam at the prompt — the shared get-args parse, the m93/m94
  one-seam rule.** A text argument that is **one token** shaped
  `/pattern/flags` speaks the regex probe through the base words
  (`role` name, `text`, `label`, `value`), the `has_text` word, the
  `>>` chain segments and the `assert` family in one pass. The flag
  letters (`i` `m` `s` `x` `U`) fold into the pattern's inline form;
  the literal must be the whole argument — a multi-token probe stays
  the fuzzy string the grammar always spoke, and a text argument that
  genuinely begins with `/` needs a different phrasing (the same
  reserved-token trade-off family the m93/m94 grammars documented). An
  unclosed literal, an unknown flag and an uncompilable pattern are the
  typed usage errors at parse time (the regex crate's own message
  beside the usage) — never a silent downgrade to the fuzzy default
  (the resolver's total-read rule would answer an invalid pattern with
  the empty set, so the prompt validates before it sends).

## 3. Surface

- `crates/guiwright-core/src/selector.rs` — the `TextPattern::Regex`
  arm + the `TextMatch::regex` / `regex_exact` / `try_regex`
  constructors, the `matches` arm's semantics (normalize, inline
  flags, anchoring, the honest `false`), the `render_match` literal
  shape (`getByText(/re/)`), the `TextPattern::validate` typed check.
- `crates/guiwright-proto/src/schema.rs` — the five appended
  `SelectorV1` rows (append-only, §7; the string arms byte-pinned).
- `crates/guiwright-proto/src/convert.rs` — the probe classification
  (`Str` → the pinned `TextMatchV1` arms, `Regex` → the appended rows)
  both directions; the decode's identity-wrapper collapse.
- `crates/guiwright-proto/src/lib.rs` — the `TextPattern` re-export
  the prompt's parse validates through.
- `crates/guiwright-cli/src/repl.rs` — `regex_probe` / `text_probe`
  (the `/…/` literal: shape check, flag fold, typed validation), the
  base arms + the `has_text` word speaking the rows, the render arms
  (`text /(?i)save/` — the composed pattern's own words), the help row.
- `crates/guiwright-kit/tests/regex_text.rs` +
  `crates/guiwright-kit/examples/regex_text.rs` — the consumer demo:
  the partial/case/anchored semantics, the role-name and value probes,
  the `has_text` narrowing, the waitable pair, and the strict-gated
  click — `cargo run -p guiwright-kit --example regex_text`.

## 4. Tests

- `crates/guiwright-core/src/selector.rs` (inline) — the semantics
  pins: partial by default, the inline flag, the anchored form, the
  normalized candidate, the uncompilable pattern's honest `false` and
  typed `try_regex` error, the literal render. `crates/guiwright-core/tests/selectors.rs`
  — six resolution pins over the fixture: the regex set, the case
  rule, the role-name probe (strict refusal + honest empty), the
  `has_text` filter, the invalid pattern's empty set, the chain
  segment. **Red→green recorded**: the first test fails to compile
  against the closed enum (`TextMatch::regex` — 8 E0599 errors), the
  arm opens them green.
- `crates/guiwright-proto/src/convert.rs` — the five regex shapes
  roundtrip byte-identically (encode → decode → core), the string
  shapes beside them unchanged. The golden vectors pin the five rows'
  bytes (`resolve_*_regex`, blessed into `clients/typescript/test/golden.json`);
  the TS codec test builds the requests and asserts byte-identity (the
  m91-style pin joining the M30/M89/M90/M91 pins).
- `crates/guiwright-cli/tests/repl_e2e.rs` —
  `repl_regex_text_probes` over a real UDS socket: the probe words on
  every get-args command, the chain segments, the `assert count` row,
  the strict `get` + click loop (the refusal naming the core's
  `getByText(/…/)` render), the error honesty (unclosed, unknown
  flags, uncompilable — the session survives), the help row.
  **Red→green recorded**: the first run answers
  `0 matches (label "/Welcome, Ada/")` — the multi-token probe stays
  fuzzy (the documented trade-off); the single-token literal passes
  after the grammar lands.
- `crates/guiwright-kit/tests/regex_text.rs` — the kit surface end to
  end: embedded resolution, the strict refusal, the role-name + value
  probes, the `has_text` narrowing, the waitable pair with the negation
  composition, and the door half (the same probes over the
  remote-to-self wire path, byte-identical with every door).
- No budget row (the m54-family rule): the probes ride the existing
  resolve / resolve-all / describe-all conversations — a second row
  would measure the same conversation twice. The §15.1 string-probe
  rows keep their numbers; a regex probe compiles its pattern per
  match (documented trade-off — the resolver's hot path is unchanged,
  the compile rides the probe's own arm).

## 5. Remaining gaps (honest list)

- The regex literal at the prompt is **one token** — a pattern with
  spaces needs the `\s` class or a different phrasing (the grammar's
  reserved-token trade-off, documented beside the m93/m94 notes; the
  kit DSL and the TS rows have no such collision).
- The flags fold into the pattern at the prompt (`/save/i` rides as
  `(?i)save`), so the render prints the composed pattern's own words —
  the wire rows carry the pattern only, one shape per language.
- No TS *typed* regex rows yet (the codec/tables/byte pins moved with
  this schema, the wire's discipline): the TS surface's `RegExp`
  acceptance — `filter({ hasText: /re/ })`, the `getBy*` factories —
  is its own pass (the m45/m60/m63 one-milestone-per-surface pattern).
- The codegen cost model still emits exact/fuzzy strings only (the
  m34 note's own terms — a recorded regex line renders via the
  selector's render arm, no new tier).
