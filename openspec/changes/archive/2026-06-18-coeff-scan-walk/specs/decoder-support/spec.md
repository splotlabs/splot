## ADDED Requirements

### Requirement: Coefficient scan walk support row
The decoder support model SHALL track `DECODE-COEFF-SCAN-WALK` as a distinct
crate-private row named `coeff-scan-walk`. The row SHALL mark only the
decode-side, caller-supplied ordinary non-FSC § 5.20.7.27 coefficient scan walk
boundary as supported, and SHALL keep scan-table derivation, transform-type
computation, coefficient base/BR/sign reads, `read_quant`, dequantization,
inverse transform, residual add, and runtime nonzero coefficient blocks partial
or unsupported until separately implemented.

#### Scenario: Matrix records narrow scan-walk support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-scan-walk` appears with Feature ID `DECODE-COEFF-SCAN-WALK`
- **AND** it cites AV2 § 5.20.7.27 as the scan-walk syntax boundary
- **AND** it names focused tests for reverse order, EOB length rejection, and
  out-of-range scan-position rejection
- **AND** it does not claim runtime nonzero coefficient decode or output support
