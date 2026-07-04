## Why

The local decoder mission stream now reaches a luma-only `BLOCK_8X32` selectable
transform leaf whose max-rectangle `TX_8X32` partition path can collapse into a
zero-width `VERT5` subrecord. That is a representation dead end for the LR
tx-skip handoff even though the supported luma-only narrow leaf already has an
unambiguous actual 8x32 extent. Once that leaf is retained, the stream also
reaches a luma-only chroma-offset `BLOCK_4X32` leaf that can be handled with the
same actual-extent luma record path without deriving chroma residual
coordinates.

## What Changes

- Track `DECODE-SELECTABLE-TRANSFORM-RECORDS` as the Feature ID for this
  bounded selectable-record handoff.
- Consume transform-partition syntax for supported luma-only narrow selectable
  leaves, then retain their actual leaf extents when applying the consumed
  partition would create impossible max-rectangle subrecords.
- Admit luma-only chroma-offset narrow leaves while continuing to reject
  chroma-bearing offset leaves before chroma residual coordinate handoff.
- Preserve skipped luma residual records as `skip_flag = true, eob = 0` so live
  `LrTxSkip` population reflects AV2 §5.20.7.24 / §5.20.7.27 rather than just
  nonzero residuals.
- Update the local decoder mission probe expectation to advance past
  `unsupported_wienerns_lr_selectable_transform_records_empty_transform` and
  stop at the next structured active-MRL frontier.
- Keep active MRL prediction, decoded samples, `FilterClass`, `SubclassLookup`,
  loop-restoration filtering/output, reference refresh, AVM/dav2d byte equality,
  and successful local decoder mission decode out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `selectable-transform-records`: selectable record derivation consumes
  partition syntax and admits the observed luma-only narrow extent without
  fabricating broader transform cells.
- `selectable-narrow-luma-records`: the narrow luma leaf handoff records
  actual 4x4-grid extents and preserves fail-closed behavior for chroma-bearing
  narrow and chroma-offset leaves.
- `decoder-support`: support tracking records the new local decoder mission frontier and
  the proof commands.

## Impact

- `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs`
- `crates/splot-cli/tests/decode_cli.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml` and generated decoder support status
- OpenSpec delta specs for the modified capabilities
