# Change: decoder-full-conformance-contract

## Feature IDs

- `DOC-DECODER-FULL-CONFORMANCE-CONTRACT`
- `XTASK-DECODER-CONFORMANCE-COVERAGE`

## Why

The Step 0 decoder audit shows that `splot decode` is still a plan-only,
intentionally unsupported runtime entry point while the existing decoder support
matrix covers only a small subset of AV2 decode-relevant sections. Before any
runtime decoder feature PR can honestly claim progress toward full AV2 v1.0.0
conformance, the repository needs a generated section-to-owner coverage contract
and a CI gate that prevents silent gaps or false `supported` claims.

## What Changes

- Add `docs/DECODER-FULL-CONFORMANCE.md` to define the public full-decoder
  conformance claim, staged status language, output variants, local-reference
  evidence boundary, and final completion criteria.
- Expand `docs/DECODER-SUPPORT-MATRIX.toml` with contract/tooling rows for the
  full conformance document and decoder conformance coverage gate.
- Add a generated `docs/DECODER-SPEC-COVERAGE.md` document mapping every
  decode-relevant AV2 v1.0.0 section family to an implementation owner, status,
  tests, fuzz target, diagnostics, and local-reference evidence.
- Add `cargo xtask check-decoder-conformance-coverage` to regenerate/check the
  decoder spec coverage document and fail on drift or unsupported status values.
- Update generated decoder support status and user-facing decoder docs to keep
  the current plan-only runtime status honest.
- Do not implement tile decoding, reconstruction, hash success output, Y4M
  runtime output, or any codec feature in this change.
- Do not add AVM/dav2d integration, wrappers, setup scripts, `xtask` commands
  that invoke external decoders, CI jobs, or mandatory local tooling.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: add the full decoder conformance contract and generated
  decoder spec coverage requirements, including a self-contained drift gate.

## Impact

- Documentation: `docs/DECODER-FULL-CONFORMANCE.md`,
  `docs/DECODER-SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-STATUS.md`, and any roadmap/status text needed to avoid
  overclaiming.
- Tooling: `xtask` gains a decoder conformance coverage render/check command
  wired into `cargo xtask ci`.
- Tests/checks: xtask unit tests for coverage rendering/checking and existing
  decoder support/status gates.
- Architecture: no crate dependency graph change, no public API change, no
  decoder runtime behavior change, and no external reference decoder execution.
