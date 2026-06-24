## ADDED Requirements

### Requirement: ac0ej3 DCT-only residual support row

The decoder support model SHALL track
`DECODE-AC0EJ3-DCTONLY-RESIDUAL-FRONTIER` as a distinct partial ac0ej3 row named
`ac0ej3-dctonly-residual-frontier`. The row SHALL describe that selectable
Wiener NS LR transform-record derivation can admit nonzero residuals only when
the actual per-plane transform path resolves to DCT_DCT, either by reading no
active transform-type syntax or by reading supported active luma transform-type
syntax that maps back to DCT_DCT. The row SHALL remain fail-closed for non-DCT
transform types, CCTX, IST, decoded frame samples, loop-restoration
filtering/output, reference refresh, AVM/dav2d byte equality, or successful
ac0ej3 decode.

#### Scenario: Matrix evidence records DCT-only residual boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-dctonly-residual-frontier` appears with Feature ID
  `DECODE-AC0EJ3-DCTONLY-RESIDUAL-FRONTIER`
- **AND** the row cites AV2 §5.20.7.27, §5.20.8.2, and §5.20.8.3
- **AND** it lists focused DCT-only admission tests, active luma transform-type
  mapping/CDF tests, and the local ac0ej3 runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3
  decode
