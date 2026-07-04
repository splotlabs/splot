## ADDED Requirements

### Requirement: local decoder mission DCT-only residual frontier

The decoder SHALL track `DECODE-DCTONLY-RESIDUAL-FRONTIER` as a partial
runtime prerequisite for the local decoder mission Wiener NS LR path. When selectable
transform-record derivation reaches a nonzero residual with broad sequence
transform tools enabled, the runtime SHALL admit the residual only if the
per-plane transform path resolves to `DCT_DCT` either without active
`transform_type()` syntax or by reading supported active luma transform-type
syntax under AV2 §5.20.8.2 and §5.20.8.3. Residuals that require unsupported
non-DCT transform-type, CCTX, IST, or FSC syntax branches SHALL remain
fail-closed before coefficient symbols that would skip the missing syntax are
read.

#### Scenario: DCT-only nonzero residual advances

- **WHEN** selectable Wiener NS LR transform-record derivation reaches a nonzero
  residual block
- **AND** AV2 §5.20.8.3 `get_tx_set(txSz, plane)` forces `TX_SET_DCTONLY`
- **THEN** the runtime uses the existing ordinary `DCT_DCT` coefficient loop
  for that residual
- **AND** it derives the corresponding `LrTxSkip` record without emitting the
  broad transform-tool residual diagnostic

#### Scenario: Supported active luma transform type can still admit DCT_DCT

- **WHEN** selectable Wiener NS LR transform-record derivation reaches a nonzero
  luma residual block
- **AND** AV2 §5.20.8.2 reads a supported active luma transform-type symbol from
  `intra_tx_type_set1`, `intra_tx_type_set2`, or the wide/high long-transform
  rows
- **AND** the generated AV2 mapping resolves that symbol to `DCT_DCT`
- **THEN** the runtime admits the residual to the existing ordinary `DCT_DCT`
  coefficient loop
- **AND** it keeps non-DCT mapped transform types fail-closed

#### Scenario: Active transform syntax remains fail-closed

- **WHEN** selectable Wiener NS LR transform-record derivation reaches a nonzero
  residual block whose transform path does not resolve to supported DCT_DCT
- **THEN** the runtime returns a structured `decode/unsupported-feature`
  diagnostic before reading coefficient syntax that would skip active
  `transform_type()`, CCTX, IST, or FSC syntax
- **AND** it does not fabricate `LrTxSkip`, decoded samples, loop-restoration
  output, reference state, AVM/dav2d byte equality, or successful local decoder mission decode

#### Scenario: local decoder mission probe reports the new frontier

- **WHEN** `splot decode --output-format hash --json` runs on the local
  local decoder mission stream
- **THEN** the diagnostic frontier reports
  `unsupported_dctonly_residual_intra_ist` at byte offset 110 under
  `DECODE-DCTONLY-RESIDUAL-FRONTIER`
- **AND** the command does not emit decoded output
