## Why

The ordinary non-FSC nonzero coefficient pass can now read state-backed DC
context lines and commit final context state, but callers still have to stitch
that path together after the `all_zero`/nonzero EOB branch by hand. A narrow
branch-level composer is the next integration seam before runtime `coeffs()`
can call the ordinary nonzero path from block syntax.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF`.
- Add a crate-private ordinary coefficient branch handoff that preserves the
  existing all-zero branch and routes nonzero branches through the state-backed
  ordinary non-FSC pass.
- Keep scan, transform-class, plane, geometry, TCQ, lossless, and broader block
  syntax facts caller-resolved; do not widen runtime reconstruction or output.
- Add focused tests for all-zero preservation, nonzero ordinary handoff success,
  and failure ordering/state preservation.
- Update decoder tracking, roadmap, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `coeff-ordinary-branch-handoff`: Crate-private coefficient branch handoff that
  composes all-zero and state-backed ordinary nonzero coefficient paths after
  caller-decoded `all_zero`.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the ordinary
  branch handoff.

## Impact

- Affects `crates/splot-decode/src/tile_payload/coeff_loop/` and the minimal
  block-symbol trace only where it can reuse the all-zero arm without changing
  output.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, `docs/DECODER-ROADMAP.md`, and
  generated status/coverage docs.
- No public API, dependency graph, encoder, validator, AVM/dav2d integration,
  dequantization, inverse transform, residual add, reconstruction, output, or
  reference-refresh changes are in scope.
