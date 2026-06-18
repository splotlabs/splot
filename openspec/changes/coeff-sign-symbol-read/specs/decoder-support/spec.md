## ADDED Requirements

### Requirement: Coefficient sign symbol-read support
The decoder support model SHALL track `DECODE-COEFF-SIGN-SYMBOL-READ` as a
distinct crate-private row named `coeff-sign-symbol-read`. The row SHALL mark
only ordinary non-FSC coefficient sign CDF/literal sequencing over caller-owned
source facts as implemented, and SHALL keep `QuantSign[]`, `Quant[]`,
`read_quant`, reconstruction, and broad `decode_block()` support partial or
unsupported until separately implemented.

#### Scenario: Matrix records narrow sign-read support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-sign-symbol-read` appears with Feature ID
  `DECODE-COEFF-SIGN-SYMBOL-READ`
- **AND** it cites AV2 §5.20.7.27 and §8.3.2 as the sign read-order and
  `dc_sign` CDF-selection boundary
- **AND** it names focused tests for mixed CDF/literal/skip reads, invalid
  selector no-consumption behavior, input-count mismatch, missing required signs,
  and scan-entry mismatch
- **AND** it does not claim `QuantSign[]`, nonzero `Quant[]`, `read_quant`,
  reconstruction, external decoder invocation, public API, or broad runtime
  `decode_tile()` support
