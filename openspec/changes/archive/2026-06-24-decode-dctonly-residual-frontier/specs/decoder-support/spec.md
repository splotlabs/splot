## ADDED Requirements

### Requirement: local decoder mission DCT-only residual support row

The decoder support model SHALL track
`DECODE-DCTONLY-RESIDUAL-FRONTIER` as a distinct partial local decoder mission row named
`dctonly-residual-frontier`. The row SHALL describe that selectable
Wiener NS LR transform-record derivation can admit nonzero residuals only when
the actual per-plane transform path resolves to DCT_DCT, either by reading no
active transform-type syntax or by reading supported active luma transform-type
syntax that maps back to DCT_DCT. The row SHALL remain fail-closed for non-DCT
transform types, CCTX, IST, decoded frame samples, loop-restoration
filtering/output, reference refresh, AVM/dav2d byte equality, or successful
local decoder mission decode.

#### Scenario: Matrix evidence records DCT-only residual boundary

- **WHEN** decoder support status is validated
- **THEN** `dctonly-residual-frontier` appears with Feature ID
  `DECODE-DCTONLY-RESIDUAL-FRONTIER`
- **AND** the row cites AV2 §5.20.7.27, §5.20.8.2, and §5.20.8.3
- **AND** it lists focused DCT-only admission tests, active luma transform-type
  mapping/CDF tests, and the local decoder mission runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful local decoder mission
  decode
