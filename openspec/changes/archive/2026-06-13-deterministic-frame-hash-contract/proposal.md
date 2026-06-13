## Why

The decoder roadmap requires deterministic frame hashes before Y4M output or
portable fixture expectations can be treated as supported, but the exact
repository-owned hash contract is still pending. Defining it now unblocks later
decoded-frame types, fixtures, and encoder roundtrip evidence without changing
the dependency graph.

## What Changes

- Define `DOC-DETERMINISTIC-FRAME-HASH-CONTRACT` as a docs-only contract for
  future decoded-frame hash output.
- Update the decoder roadmap to specify the hash algorithm, sample traversal,
  byte representation, plane order, crop/stride policy, film-grain policy, and
  metadata inclusion policy.
- Update the decoder support matrix so `deterministic-frame-hash` is `partial`
  with self-contained proof and remains explicitly not emitted by source.
- Sync generated decoder/feature/spec status docs and OpenSpec decoder-support
  requirements.
- Record required subagent planning/review evidence in this change's
  `agent-log.md`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: adds the deterministic decoded-frame hash contract needed
  before future decode output, fixtures, Y4M support, or encoder roundtrip
  evidence can be marked supported.

## Impact

- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder/feature/spec status
  docs, and `docs/IMPLEMENTATION-MATRIX.toml`.
- Affected OpenSpec capability: `openspec/specs/decoder-support/spec.md`.
- Affected runtime code: none.
- Affected dependencies/workspace graph: none.
- AVM/dav2d impact: none; this change records no new executable local
  reference workflow and does not add reference-tool runners, wrappers, tests,
  scripts, or CI jobs.
