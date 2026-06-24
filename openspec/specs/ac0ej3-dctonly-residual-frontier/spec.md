# ac0ej3-dctonly-residual-frontier Specification

## Purpose

Track the partial ac0ej3 Wiener NS LR residual frontier where selectable
transform-record derivation may admit only residuals whose transform path
resolves to `DCT_DCT`.

## Requirements

### Requirement: ac0ej3 DCT-only residual frontier

The decoder SHALL track `DECODE-AC0EJ3-DCTONLY-RESIDUAL-FRONTIER` as a partial
runtime prerequisite for the ac0ej3 Wiener NS LR path. When selectable
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
  output, reference state, AVM/dav2d byte equality, or successful ac0ej3 decode

#### Scenario: ac0ej3 probe records the follow-on frontier

- **WHEN** `splot decode --output-format hash --json` runs on the local
  `ac0ej3.ivf` mission stream
- **THEN** follow-on rows may advance the live diagnostic beyond the DCT-only
  residual row after consuming additional syntax
- **AND** the command does not emit decoded output
