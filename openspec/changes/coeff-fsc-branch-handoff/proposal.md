## Why

The FSC/IDTX coefficient path now has loaded level, sign, quant, and context
commit stages, but callers still have to assemble the nonzero EOB start and
checked FSC scan walk by hand. A branch-level handoff is the next small step
toward the real AV2 § 5.20.7.27 `coeffs()` routing without widening runtime
decode output yet.

Feature ID: `DECODE-COEFF-FSC-BRANCH-HANDOFF`.

## What Changes

- Add a crate-private loaded-but-unwired FSC branch handoff that starts from the
  caller-decoded nonzero EOB branch, derives the checked `bob..segEob` FSC scan
  window, runs the FSC level pass, then delegates to the existing FSC
  sign/quant/context-commit wrapper.
- Reject all-zero routing and non-luma FSC routing before unintended mutation.
- Add focused tests for equivalence with the explicit FSC pipeline plus
  fail-atomic all-zero, scan, and plane rejection cases.
- Update the implementation matrix, decoder support matrix, decoder conformance
  coverage, roadmap, and generated status documents.

## Capabilities

### New Capabilities

- `coeff-fsc-branch-handoff`: covers the loaded-but-unwired FSC/IDTX nonzero
  coefficient branch handoff for AV2 § 5.20.7.27.

### Modified Capabilities

- `decoder-support`: adds the decoder support row for
  `DECODE-COEFF-FSC-BRANCH-HANDOFF`.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, decoder conformance coverage, roadmap,
  generated status docs, and this OpenSpec change.
- No public API, CLI, dependency graph, licensing, fixture, or runtime decode
  output change is intended.
- Non-goals: deriving runtime `useFsc`, deriving runtime `segEob` or scan from
  full frame state, dequantization, inverse transform, residual add,
  reconstruction/output, reference refresh, inter prediction, filters, or
  broad `decode_tile()`/`decode_block()` support.
