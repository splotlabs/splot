## Why

The local ac0ej3 probe now reaches AV2 §5.20.7.27 coefficient syntax at byte
offset 110 and stops on a broad transform-tool residual guard. Some nonzero
residual blocks still have a `DCT_DCT` transform path that either reads no
`transform_type()` syntax or reads active luma transform-type syntax that maps
back to `DCT_DCT`; admitting only that path can move the stream forward without
claiming unsupported non-DCT, CCTX, IST, or FSC transform tools.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-DCTONLY-RESIDUAL-FRONTIER`.
- Teach the ac0ej3 Wiener NS LR selectable transform-record path to distinguish
  nonzero residuals whose actual transform branch is AV2 §5.20.8.3
  `TX_SET_DCTONLY` / `DCT_DCT`, or whose supported active luma transform-type
  branch resolves to `DCT_DCT`, from residuals that still require unsupported
  non-DCT transform-type, CCTX, IST, or FSC syntax.
- Allow DCT-only nonzero residuals to use the existing coefficient loop and
  continue deriving `LrTxSkip` records.
- Keep active transform-tool cases fail-closed with structured
  `decode/unsupported-feature` diagnostics before skipped syntax can desync the
  stream.

## Capabilities

### New Capabilities

- `ac0ej3-dctonly-residual-frontier`: DCT-only residual admission for the local
  ac0ej3 Wiener NS LR transform-record frontier.

### Modified Capabilities

- `decoder-support`: add a decoder support row for the DCT-only residual
  frontier.

## Impact

- Affects `crates/splot-decode` coefficient/residual handoff and Wiener NS LR
  transform-record diagnostics.
- Adds luma transform-type CDF coverage for `intra_tx_type_set1`,
  `intra_tx_type_set2`, `is_long_side_dct`, and `intra_tx_type_long`.
- Updates implementation and decoder support matrices plus generated status
  documents.
- Adds focused tests and refreshes the local ac0ej3 probe expectation.
- No new dependencies, public APIs, encoder behavior, validator behavior, or
  external decoder invocation in repo code/CI.
