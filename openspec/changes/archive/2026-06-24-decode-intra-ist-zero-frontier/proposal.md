## Why

The local decoder mission stream now reaches AV2 transform-tool residual syntax
and stops at the intra IST branch before `sec_tx_type` is consumed. This change
lets the decoder parse the covered intra IST zero-secondary-transform subset so
the stream can advance to the next honest runtime frontier without pretending to
support active secondary transforms.

Feature ID: `DECODE-INTRA-IST-ZERO-FRONTIER`.

## What Changes

- Add tile CDF selection, default-copy, update, and lifecycle coverage for the
  AV2 §8.3.2 `TileSecTxTypeCdf` row family used by §5.20.7.29.
- Add the adjacent `TileMostProbableStxSetCdf` and
  `TileMostProbableStxSetAdstCdf` rows so non-zero intra IST syntax can be
  consumed in spec order before the decoder fails closed.
- Change the DCT-only transform-tool residual boundary from rejecting every
  reachable intra IST branch to admitting only `sec_tx_type == 0`.
- Preserve precise unsupported diagnostics for active secondary transforms,
  inter IST, CCTX, non-DCT transform types, and non-DCT transform sets.
- Update implementation and decoder-support tracking plus the local decoder mission
  runtime gate test.

## Capabilities

### New Capabilities

- `intra-ist-zero-frontier`: Tracks the specific intra IST
  zero-secondary-transform syntax frontier and its fail-closed active-secondary
  transform boundary.

### Modified Capabilities

- `decoder-support`: Records the new partial local decoder mission support row and the
  associated local probe evidence without claiming successful decode output.

## Impact

- Affects `crates/splot-decode` tile CDF rows, block-symbol reads, and
  transform-tool residual syntax admission.
- Affects `crates/splot-cli/tests/decode_cli.rs` only if the local decoder mission probe
  advances to a new unsupported runtime gate.
- Affects `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs, and OpenSpec
  decoder-support specs.
- No new dependencies, dependency-graph changes, encoder work, successful
  local decoder mission output claim, AVM/dav2d equivalence claim, reconstruction expansion,
  or broad AV2 secondary-transform implementation.
