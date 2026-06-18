## ADDED Requirements

### Requirement: Decoder support matrix tracks coefficient EOB value state

The decoder support matrix SHALL include a partial row named
`coeff-eob-value-state`, tracked by Feature ID
`DECODE-COEFF-EOB-VALUE-STATE`, for the crate-private AV2 § 5.20.7.27 helper
that derives a nonzero `eob` value from caller-decoded `eobPt`, `eob_extra`, and
packed `eob_extra_bit` refinements. The row SHALL keep broad coefficient decode
and decoded-output support partial until later changes read the `eob_pt_*` CDF
rows, walk the coefficient scan, fill nonzero coefficient state, and run
reconstruction.

#### Scenario: EOB value helper is scoped and test-backed

- **WHEN** `cargo xtask check-decoder-support` renders decoder support status
- **THEN** `coeff-eob-value-state` appears with Feature ID
  `DECODE-COEFF-EOB-VALUE-STATE`
- **AND** it records focused tests for small `eobPt`, refined `eob_extra`, max
  AV2 EOB, and invalid caller-provided EOB parts
- **AND** it cites AV2 § 5.20.7.27 through
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`

#### Scenario: Broad coefficient decode remains partial

- **WHEN** decoder support and conformance coverage status documents are
  regenerated after this change
- **THEN** broad tile payload, symbol/CDF, and coefficient-loop rows remain
  partial for actual `eob_pt_*` symbol reads, `eob_extra` symbol reads,
  `eob_extra_bit` literal reads, scan-order traversal, coefficient base/br/sign
  symbol reads, nonzero `Quant[]` writes, `read_quant`, dequantization, inverse
  transform, residual addition, and decoded output changes
