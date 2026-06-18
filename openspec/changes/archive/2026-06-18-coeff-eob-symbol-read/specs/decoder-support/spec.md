## ADDED Requirements

### Requirement: Decoder support matrix tracks coefficient EOB symbol reads

The decoder support matrix SHALL include a partial row named
`coeff-eob-symbol-read`, tracked by Feature ID
`DECODE-COEFF-EOB-SYMBOL-READ`, for the crate-private AV2 § 5.20.7.27 helper
that reads the caller-selected `eob_pt_*` symbol, any size-specific
`eob_pt_*_extra` literal bits, `eob_extra`, and any `eob_extra_bit` refinement
literals before producing the checked nonzero EOB value. The row SHALL keep
broad coefficient decode and decoded-output support partial until later changes
wire the helper into the coefficient scan and coefficient state writes.

#### Scenario: EOB symbol helper is scoped and test-backed

- **WHEN** `cargo xtask check-decoder-support` renders decoder support status
- **THEN** `coeff-eob-symbol-read` appears with Feature ID
  `DECODE-COEFF-EOB-SYMBOL-READ`
- **AND** it records focused tests for EOB point CDF consumption, EOB extra CDF
  consumption, size-class extra literal handling, invalid selector rollback
  before reads, and disabled CDF update behavior
- **AND** it cites AV2 § 5.20.7.27 and § 8.3.2 through the committed spec mirror

#### Scenario: Broad coefficient decode remains partial

- **WHEN** decoder support and conformance coverage status documents are
  regenerated after this change
- **THEN** broad tile payload, symbol/CDF, and coefficient-loop rows remain
  partial for transform-size derivation, scan-order traversal, coefficient
  base/br/sign symbol reads, nonzero `Level[]` and `Quant[]` writes,
  `read_quant`, dequantization, inverse transform, residual addition, and
  decoded output changes
