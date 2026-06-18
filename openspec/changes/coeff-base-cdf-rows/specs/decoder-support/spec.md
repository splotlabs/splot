## ADDED Requirements

### Requirement: Coefficient base CDF row support
The decoder support model SHALL track `DECODE-COEFF-BASE-CDF-ROWS` as a
distinct crate-private row named `coeff-base-cdf-rows`. The row SHALL mark only
loaded-but-unread tile CDF row storage, selection, and lifecycle coverage for
ordinary non-IDTX coefficient base, base-EOB, and base-range symbol families as
supported, and SHALL keep coefficient symbol reads, nonzero coefficient writes,
`read_quant`, dequantization, inverse transform, residual add, and runtime
nonzero coefficient blocks partial or unsupported until separately implemented.

#### Scenario: Matrix records narrow CDF-row support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-base-cdf-rows` appears with Feature ID
  `DECODE-COEFF-BASE-CDF-ROWS`
- **AND** it cites AV2 § 8.3.2 and § 9.3 as the CDF-selection/default-table
  boundary
- **AND** it names focused tests for generated-default loading, selector bounds
  errors, tile-copy non-aliasing, and mutable row handoff
- **AND** it does not claim runtime coefficient symbol reads or output support
