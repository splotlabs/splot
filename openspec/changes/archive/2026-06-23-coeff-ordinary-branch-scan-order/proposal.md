## Why

The ordinary coefficient branch `txSz` wrapper still accepts a caller-provided
`scan` slice even though AV2 section 5.20.7.27 derives it directly after
`txClass` as `scan = get_scan(txSz, txClass)`. The next safe step is to derive
that scan order inside the staged wrapper while keeping runtime `coeffs()` output
unchanged.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER`.
- Extend the crate-private ordinary branch `txSz` wrapper to derive the AV2
  section 5.20.7.30 scan order from `txSz` and the already caller-resolved
  `PlaneTxType` / `txClass`.
- Remove caller-supplied `scan` from the `txSz`-dimensions wrapper input while
  preserving lower explicit handoff APIs for staged tests.
- Add focused tests proving 2D, horizontal, and vertical scan derivation,
  all-zero preservation, invalid scan-shape fail-atomicity, and no dependency on
  `splot-recon` for entropy-path scan derivation.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-scan-order`: The loaded-but-unwired ordinary
  coefficient branch derives `get_scan(txSz, txClass)` before invoking the
  existing ordinary pass.

### Modified Capabilities

- `decoder-support`: Record the scan-order ordinary branch row and proof while
  keeping runtime coefficient-loop integration partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`
  and focused ordinary branch tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, broad `decode_block()` /
  `decode_tile()` behavior, runtime `coeffs()` call site, `compute_tx_type`,
  dequantization, reconstruction, output, or reference refresh support is added.
