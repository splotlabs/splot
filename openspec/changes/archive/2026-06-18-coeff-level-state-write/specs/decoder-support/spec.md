## ADDED Requirements

### Requirement: Coefficient level state-write support
The decoder support model SHALL track `DECODE-COEFF-LEVEL-STATE-WRITE` as a
distinct crate-private row named `coeff-level-state-write`. The row SHALL mark
only ordinary non-FSC decoded level application into local `Level[]` state as
implemented, and SHALL keep sign reads, `QuantSign[]`, `Quant[]`, `read_quant`,
reconstruction, and broad `decode_block()` support partial or unsupported until
separately implemented.

#### Scenario: Matrix records narrow level-write support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-level-state-write` appears with Feature ID
  `DECODE-COEFF-LEVEL-STATE-WRITE`
- **AND** it cites AV2 §5.20.7.27 as the `Level[row][col] = level`
  state-application boundary
- **AND** it names focused tests for row-major placement, untouched quantization
  state, scan-entry mismatch rejection, and mismatched geometry rejection
- **AND** it does not claim sign reads, nonzero `Quant[]` output, `read_quant`,
  reconstruction, external decoder invocation, public API, or broad runtime
  `decode_tile()` support
