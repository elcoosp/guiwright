# M67 — The TypeScript client's single-node identity describe (`describeIdentity()`)

**Status:** implemented. The m66 Rust rows mirrored on the other-language
client (the m45/m60/m63/m65 one-milestone-per-surface pattern): the
`describeIdentity(node, generation)` row over the appended
`describe_identity` domain — the M36 handle pair riding in, the M64
identity row riding out — the cross-language e2e leg against the real
Rust server, and the consumer demo (`npm run example:describe-identity`).
No wire change beyond the m66 domain the codec and variant tables
already carry (the m66 record moved them so this milestone's red step
was the client method, not the codec).

## 1. Surface
- `clients/typescript/src/client.ts` — the `describeIdentity(node,
  generation)` row (the m66 `ClientConn::describe_identity` mirrored;
  the typed rejection rides `GwpRejected` / `codeName`, the Action
  boundary's honesty).
- `clients/typescript/test/client.test.ts` — the handle-width read
  against the golden-frame mock server (the m41 harness shape): the
  blessed `described_identity_install_button` frame replayed
  byte-for-byte, the identity four asserted beside the fact.
- `clients/typescript/scripts/e2e.ts` — the handle-width leg (the
  satisfied row, the absent fact riding null, the ghost handle's typed
  `UnknownNode`) against the demo app, which serves the described
  fixture.
- `clients/typescript/examples/describe_identity.ts` — the runnable
  demo (`npm run example:describe-identity`).

## 2. Tests
- `npm test` — 52 passing (the handle-width pin added).
- `npm run e2e` — the full conversation green including the m66/m67
  leg over TCP (and the M15 WS leg byte-identical).
- `npm run example:describe-identity` — the demo conversation prints
  the satisfied row (the fact named beside the identity four), the
  undescribed node's `null`, and the ghost handle's typed rejection.

## 3. Remaining gaps (honest list)
- No negation on the client: `not()` is the kit decorator's composition
  (M24); the client's honest negative is the typed timeout (the
  m54/m59/m63/m65 rule, carried unchanged) — and for a total read, the
  typed rejection.
- No state flags on the identity row: the checked/focused/enabled
  facts ride the dump rows and the kit assertion vocabulary (the m36
  record's split, unchanged); a state-fact widening lands when a
  consumer asks — the same append-only move, its own domain.
