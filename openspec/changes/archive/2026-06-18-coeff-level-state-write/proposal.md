## Why

The ordinary non-FSC coefficient path can now read base/base-EOB/base-range
symbols and return decoded levels, but those levels are still summaries only.
AV2 §5.20.7.27 immediately writes each decoded level into `Level[row][col]`
before the later sign and `read_quant` passes can inspect the block state.
Adding that narrow state-application boundary is the next step toward real
nonzero coefficient decoding without jumping to quantization or reconstruction.

## What Changes

- Add Feature ID `DECODE-COEFF-LEVEL-STATE-WRITE` for applying decoded ordinary
  non-FSC base/base-range levels to the local transform-block `Level[]` state.
- Add a crate-private helper that accepts a nonzero block start, checked scan
  walk, and decoded base-symbol summaries, validates their scan-entry pairing,
  and writes each decoded level through the existing checked
  `TransformCoeffBlockState` accessors.
- Preserve transactional validation: input count, scan-entry mismatch, and block
  coordinate errors fail before any level write is performed.
- Add focused tests for correct row-major `Level[]` placement, unchanged
  `QuantSign[]`/`Quant[]` arrays, count/entry mismatch rejection, and
  mismatched block/walk geometry rejection.
- Update the implementation matrix, decoder support matrix, decoder conformance
  coverage metadata, roadmap, and generated status/coverage docs.

Non-goals:

- No runtime `coeffs()` integration and no decode-output change.
- No derivation of real scan tables, transform type, low-frequency status,
  parity hiding, TCQ, or sign contexts from block syntax.
- No sign-symbol reads, no `dc_sign`/`idtx_sign`, no `QuantSign[]` or `Quant[]`
  writes, no `read_quant`, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, public API, AVM/dav2d invocation,
  dependency change, or scheduler change.

## Capabilities

### New Capabilities

- `coeff-level-state-write`: crate-private ordinary non-FSC coefficient
  `Level[]` state-application boundary after base/base-range symbol reads.

### Modified Capabilities

- `decoder-support`: records `DECODE-COEFF-LEVEL-STATE-WRITE` as a partial
  coefficient-decode support row while signs, quantization, reconstruction, and
  runtime integration remain incomplete.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_loop.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/level_state.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/level_state_tests.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/branch.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
