# Change: decoder-runtime-structure

## Feature IDs

- `DECODE-RUNTIME-STRUCTURE`

## Why

The active decoder path is still organized under `runtime_minimal` and
`runtime_minimal_recon`, even though that code now owns stream orchestration,
tile/block traversal, prediction, residual reconstruction, reference state,
filters/restoration, and hash/raw/Y4M output. The names imply a temporary small
runtime and hide the real decoder domains.

## Scope

- Spec sections: no new AV2 syntax or semantics; this is a structural
  behavior-preserving refactor of existing decoder implementation.
- Crates/modules: `crates/splot-decode`, decoder-facing docs, implementation
  matrix/status metadata, and OpenSpec records.
- CLI/docs/tests: preserve existing hash/raw/Y4M decode behavior and focused
  CLI tests while updating docs to describe the new decoder module tree.

## Non-goals

- No new AV2 decode support.
- No public API expansion.
- No dependency graph change.
- No AVM/dav2d oracle claim unless a local comparison is explicitly run.
- No scheduler/runtime state moves into `splot-recon`.

## Acceptance criteria

- [ ] Production decode code is no longer organized under `runtime_minimal` or
  `runtime_minimal_recon`.
- [ ] `splot-decode` has domain modules for pipeline, bitstream, prediction,
  residual, reference, filters, output, support, and tile-facing state.
- [ ] Existing hash/raw/Y4M outputs remain byte-identical for committed
  supported fixtures.
- [ ] Documentation explains the decoder module map once and records migration
  history in a decision record.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes.
