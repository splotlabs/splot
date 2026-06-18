## ADDED Requirements

### Requirement: Coefficient quantized-state support
The decoder support model SHALL track `DECODE-COEFF-QUANT-STATE-WRITE` as a
distinct crate-private row named `coeff-quant-state-write`. The row SHALL mark
only ordinary non-FSC quantized-coefficient state writes from caller-provided
`read_quant` outputs as implemented, and SHALL keep §5.20.7.28 `read_quant`
syntax parsing, `QuantSign[]` writes, runtime `coeffs()`, dequantization,
reconstruction, and broad `decode_block()` support partial or unsupported until
separately implemented.

#### Scenario: Matrix records narrow quant-state support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-quant-state-write` appears with Feature ID
  `DECODE-COEFF-QUANT-STATE-WRITE`
- **AND** it cites AV2 §5.20.7.27 for `Quant[]`, `culLevel`, `dcCategory`,
  hidden-parity, and TCQ state effects
- **AND** it cites AV2 §5.20.7.28 only as the still-deferred source of
  caller-provided quant values
- **AND** it names focused tests for positive writes, hidden-parity and TCQ
  adjustment, zero-level sign behavior, and mismatch rejection before mutation
- **AND** it does not claim runtime `read_quant`, dequantization,
  reconstruction, `QuantSign[]` writes, external decoder invocation, public API,
  or broad runtime `decode_tile()` support
