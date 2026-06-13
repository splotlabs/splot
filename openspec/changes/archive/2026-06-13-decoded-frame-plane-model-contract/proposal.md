## Why

Future `splot` decoder, reconstruction, hash, Y4M, reference-frame, and encoder
roundtrip work all need the same understanding of what a decoded frame and plane
mean. Defining that contract now prevents later crate scaffolding from baking in
storage-layout assumptions that conflict with AV2 output semantics.

## What Changes

- Define `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT` as a docs-only contract for
  future decoded-frame and plane data structures.
- Document future `DecodedFrame`, `Plane<T>`, `PixelFormat`, and `BitDepth`
  invariants: ownership, visible dimensions, plane dimensions, stride policy,
  chroma subsampling, monochrome handling, crop/output metadata, output order,
  and reference-store facts.
- Update `decoded-frame-plane-model` in the decoder support matrix from `todo`
  to contract-only `partial`.
- Sync generated decoder support, feature status, spec coverage, and OpenSpec
  decoder-support requirements.
- Record required subagent planning/review evidence in `agent-log.md`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: adds the decoded frame and plane model contract needed
  before future reconstruction crates, frame hashes, Y4M output, reference-frame
  storage, or encoder closed-loop APIs can be marked supported.

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
