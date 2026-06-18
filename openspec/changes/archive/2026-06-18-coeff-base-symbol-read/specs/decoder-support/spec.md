## ADDED Requirements

### Requirement: Coefficient base symbol-read support
The decoder support model SHALL track `DECODE-COEFF-BASE-SYMBOL-READ` as a
distinct crate-private row named `coeff-base-symbol-read`. The row SHALL mark
only ordinary non-FSC coefficient base/base-EOB/base-range symbol-read sequencing
over caller-resolved scan and selector facts as implemented, and SHALL keep
runtime coefficient-state writes, `read_quant`, reconstruction, and broad
`decode_block()` support partial or unsupported until separately implemented.

#### Scenario: Matrix records narrow symbol-read support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-base-symbol-read` appears with Feature ID
  `DECODE-COEFF-BASE-SYMBOL-READ`
- **AND** it cites AV2 §5.20.7.27 and §8.3.2 as the coefficient-loop read-order
  and CDF-selection boundary
- **AND** it names focused tests for direct-read equivalence, scan-entry
  matching, base-range conditional reads, disabled CDF updates, and
  invalid-selector no-consumption behavior
- **AND** it does not claim nonzero `Quant[]` output, `read_quant`,
  reconstruction, external decoder invocation, public API, or broad runtime
  `decode_tile()` support
