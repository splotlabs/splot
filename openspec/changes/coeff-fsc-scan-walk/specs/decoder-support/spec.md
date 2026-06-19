## ADDED Requirements

### Requirement: Coefficient FSC scan walk support status

The decoder support model SHALL track `DECODE-COEFF-FSC-SCAN-WALK` as a distinct
crate-private row named `coeff-fsc-scan-walk`. The row SHALL mark only checked
FSC/IDTX scan-window derivation as partial support, and SHALL keep runtime
`useFsc` symbol sequencing, nonzero coefficient state writes, dequantization,
reconstruction, output, reference refresh, public APIs, and AVM/dav2d evidence
unsupported until separately implemented.

#### Scenario: Matrix records the FSC scan boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-fsc-scan-walk` appears with Feature ID
  `DECODE-COEFF-FSC-SCAN-WALK`
- **AND** it cites AV2 §5.20.7.27 for the `bob = segEob - eob` scan window
- **AND** it names focused FSC scan-walk tests
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
