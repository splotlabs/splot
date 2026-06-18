## Why

The §8.3.2 coefficient-symbol contexts are now derived, but they still read
caller-provided slices. The next decoder brick needs real `splot-decode` tile
state for the per-transform-block `Level[]` / `QuantSign[]` buffers and the
above/left DC-context buffers before the §5.20.7.27 `coeffs()` loop can consume
those contexts.

## What Changes

- Add Feature ID `DECODE-TILE-COEFF-STATE-BUFFERS` for crate-private coefficient
  state buffers used by future transform-block coefficient decode.
- Add a new `tile-coeff-state-buffers` decoder-support row.
- Add a crate-private `splot-decode` module that:
  - allocates bounded per-transform-block `Level[]` and `QuantSign[]` arrays for
    adjusted transform sizes up to 32x32;
  - exposes checked read/write helpers and row-major views for existing
    coefficient-context functions;
  - owns above/left level and DC-context tile lines for the three AV2 planes;
  - applies the §5.20.7.27 end-of-`coeffs()` context update for
    `AboveLevelContext`, `LeftLevelContext`, `AboveDcContext`, and
    `LeftDcContext`;
  - applies the §5.20 block-context reset path for the same level/DC lines.
- Keep the new state loaded-but-unwired: no coefficient symbols are read and no
  decode output changes.

Non-goals:

- No §5.20.7.27 `coeffs()` symbol loop.
- No `read_quant`, dequant, inverse-transform, residual add, reconstruction,
  hashes, raw/Y4M output expansion, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.
- No broad block/partition syntax, inter prediction, loop filters, CDEF,
  loop-restoration, or film grain.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the new tile coefficient state buffer boundary and
  keeps tile payload, CDF selection, reconstruction, and full decoder support
  partial.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_state.rs`
- `crates/splot-decode/src/tile_payload.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
