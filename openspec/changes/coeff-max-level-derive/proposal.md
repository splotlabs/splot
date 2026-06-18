## Why

The ordinary non-FSC quant pass can now parse `read_quant` syntax and write
signed `Quant[]`, but it still takes `maxLevel` as a caller-provided fact. AV2
§ 5.20.7.27 derives `maxLevel` from the coefficient position, transform class,
plane, and hidden-parity state immediately before calling `read_quant`. A narrow
decode-local derivation helper removes that caller fact without wiring full
runtime `coeffs()` yet.

## What Changes

- Add Feature ID `DECODE-COEFF-MAX-LEVEL-DERIVE`.
- Add a crate-private helper that implements `get_lf_limits(row, col, txClass,
  plane)` and the ordinary non-FSC `maxLevel` selection from AV2 § 5.20.7.27.
- Return records that can be converted directly into the existing quant-pass
  inputs.
- Keep transform-class derivation, hidden-parity derivation, scan-table
  derivation, runtime `coeffs()` integration, dequantization, reconstruction,
  and output bytes unchanged.
- Repair the adjacent quant-pass proof lists to name the current post-review
  tests.

## Capabilities

### New Capabilities

- `coeff-max-level-derive`: crate-private ordinary non-FSC `maxLevel` derivation
  for checked scan entries.

### Modified Capabilities

- `decoder-support`: records the new decoder-support row and keeps broad
  coefficient-loop runtime support partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated feature/support/spec coverage
  docs, and `docs/DECODER-ROADMAP.md`.
- Public APIs and crate dependencies are unchanged.
- The minimal runtime decode path and committed fixture output are unchanged.
