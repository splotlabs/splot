## Why

`splot decode` is still a stub, but the decoder mission needs a bounded,
encoder-useful plan before any crate split, pixel reconstruction, or local
reference evidence can land. The repository already has a canonical
implementation matrix for parser/validator work; decoder and reconstruction
scope needs the same status honesty and CI drift protection.

## What Changes

- Add a decoder/reconstruction roadmap that defines the supported tier, the
  AVM/dav2d local-only boundary, and the staged path from structured
  unsupported diagnostics to frame hashes and Y4M output.
- Add a canonical decoder support matrix with status rows for decode driver,
  reconstruction, output, references, limits, fuzzing, and local reference
  evidence.
- Generate a checked decoder support status document from the matrix, and make
  `cargo xtask ci` fail when generated decoder docs drift.
- Update repository docs to point at the decoder roadmap and support matrix
  without claiming pixel reconstruction exists.
- Record Phase 1 subagent audit evidence in `agent-log.md`.

This change does not implement a decoder, add new crates, change the dependency
graph, or run AVM/dav2d from repo code or CI.

## Capabilities

### New Capabilities

- `decoder-support`: Documents and gates the staged decoder/reconstruction
  support model, including supported tiers, unsupported-feature reporting,
  deterministic hash policy, self-contained tests, and local-only reference
  evidence.

### Modified Capabilities

- `process`: `cargo xtask ci` also checks generated decoder support status for
  drift, without invoking AVM/dav2d or any external decoder.

## Impact

- Docs: `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, and a
  generated decoder status document.
- Automation: `xtask` gains decoder-support status rendering and a drift check
  that runs inside `cargo xtask ci`.
- Feature tracking: new matrix rows for decoder docs/status automation and the
  future `splot decode` surface remain honest until implementation follows.
- OpenSpec: introduces `decoder-support` requirements and updates `process`
  requirements for the new drift gate.
- No new dependencies, no new workspace members, no AVM/dav2d integration, and
  no behavior change to current parser/validator output.
